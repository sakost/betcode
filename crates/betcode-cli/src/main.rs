//! `BetCode` CLI
//!
//! Terminal interface for interacting with Claude Code through the daemon.
//! Provides both TUI (ratatui) and headless modes.

use std::io;

use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use betcode_cli::auth_cmd::{self, AuthAction};
use betcode_cli::config::CliConfig;
use betcode_cli::connection::{ConnectionConfig, DaemonConnection};
use betcode_cli::gitlab_cmd::{self, GitLabAction};
use betcode_cli::headless::{self, HeadlessConfig};
use betcode_cli::machine_cmd::{self, MachineAction};
use betcode_cli::repo_cmd::{self, RepoAction};
use betcode_cli::worktree_cmd::{self, WorktreeAction};

#[derive(Parser, Debug)]
#[command(name = "betcode")]
#[command(version, about = "Claude Code multiplexer CLI", long_about = None)]
struct Cli {
    /// Prompt to send (enables headless mode if no --interactive)
    #[arg(short, long)]
    prompt: Option<String>,

    /// Session ID to resume (creates new if not specified)
    #[arg(short, long)]
    session: Option<String>,

    /// Working directory for the session
    #[arg(short = 'd', long)]
    working_dir: Option<String>,

    /// Model to use (e.g., "claude-sonnet-4")
    #[arg(short, long)]
    model: Option<String>,

    /// Daemon address
    #[arg(long, default_value = "http://127.0.0.1:50051")]
    daemon_addr: String,

    /// Relay server URL (overrides config file)
    #[arg(long)]
    relay: Option<String>,

    /// Target machine ID for relay routing (overrides config file)
    #[arg(long)]
    machine: Option<String>,

    /// Path to custom CA certificate for verifying relay TLS (for self-signed/dev certs)
    #[arg(long)]
    relay_custom_ca_cert: Option<std::path::PathBuf>,

    /// Continue the most recent session in the current working directory
    #[arg(short = 'c', long = "continue")]
    continue_session: bool,

    /// Auto-accept all permission prompts (headless only)
    #[arg(long)]
    yes: bool,

    /// Subcommand to run (omit for chat mode)
    #[command(subcommand)]
    command: Option<Commands>,
}

/// Top-level subcommands.
#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Manage git worktrees
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Manage registered git repositories
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
    /// Authenticate with a relay server
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Manage remote machines
    Machine {
        #[command(subcommand)]
        action: MachineAction,
    },
    /// GitLab project operations (MRs, pipelines, issues)
    Gitlab {
        #[command(subcommand)]
        action: GitLabAction,
    },
}

#[tokio::main]
#[allow(clippy::too_many_lines, clippy::expect_used, clippy::print_stderr)]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Use file-based tracing for TUI mode to avoid polluting terminal
    let is_headless = cli.prompt.is_some() || cli.command.is_some();
    if is_headless {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode=info".into()),
            ))
            .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode=warn".into()),
            ))
            .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
            .init();
    }

    info!(version = env!("CARGO_PKG_VERSION"), "Starting betcode CLI");

    // Load persistent config and merge CLI flags
    let mut cli_config = CliConfig::load();
    if let Some(ref url) = cli.relay {
        cli_config.relay_url = Some(url.clone());
    }
    if let Some(ref mid) = cli.machine {
        cli_config.active_machine = Some(mid.clone());
    }
    if let Some(ref ca) = cli.relay_custom_ca_cert {
        cli_config.relay_custom_ca_cert = Some(ca.clone());
    }

    // Dispatch auth/machine subcommands (don't need daemon connection)
    match cli.command {
        Some(Commands::Auth { action }) => {
            return auth_cmd::run(action, &mut cli_config).await;
        }
        Some(Commands::Machine { action }) => {
            return machine_cmd::run(action, &mut cli_config).await;
        }
        _ => {}
    }

    // For relay mode: refresh token first, then check prerequisites
    if cli_config.relay_url.is_some() {
        if cli_config.auth.is_some() {
            // Try to refresh the access token silently
            match auth_cmd::ensure_valid_token(&mut cli_config).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Warning: token refresh failed: {e}");
                    // Don't bail — the existing token might still work,
                    // or the relay will reject it with a clear error.
                }
            }
        }

        if cli_config.auth.is_none() {
            anyhow::bail!(
                "Relay URL set but not logged in. Run: betcode --relay {} auth login -u <user> -p <pass>",
                cli_config
                    .relay_url
                    .as_ref()
                    .expect("relay_url checked above")
            );
        }
        if cli_config.active_machine.is_none() {
            anyhow::bail!(
                "Relay URL set but no active machine. Run: betcode --relay {} machine list",
                cli_config
                    .relay_url
                    .as_ref()
                    .expect("relay_url checked above")
            );
        }
    }

    // Build connection config: relay mode if we have auth + machine, else local daemon
    let conn_config = if cli_config.is_relay_mode() {
        ConnectionConfig {
            addr: cli_config.relay_url.clone().unwrap_or_default(),
            auth_token: cli_config.auth.as_ref().map(|a| a.access_token.clone()),
            machine_id: cli_config.active_machine.clone(),
            ca_cert_path: cli_config.relay_custom_ca_cert.clone(),
            ..Default::default()
        }
    } else {
        ConnectionConfig {
            addr: cli.daemon_addr.clone(),
            ..Default::default()
        }
    };

    let mut conn = DaemonConnection::new(conn_config);
    conn.connect().await?;

    // Resolve --continue to a session ID
    let mut session_id = cli.session;
    if cli.continue_session && session_id.is_none() {
        let working_dir = cli.working_dir.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .expect("current directory must be accessible")
                .to_string_lossy()
                .to_string()
        });
        let resp = conn.list_sessions(Some(&working_dir)).await?;
        if let Some(latest) = resp.sessions.first() {
            info!(session_id = %latest.id, "Continuing most recent session");
            session_id = Some(latest.id.clone());
        } else {
            anyhow::bail!("No sessions found in {working_dir}. Start a new session first.");
        }
    }

    // Dispatch remaining subcommands or chat mode
    if let Some(Commands::Worktree { action }) = cli.command {
        worktree_cmd::run(&mut conn, action).await?;
    } else if let Some(Commands::Repo { action }) = cli.command {
        repo_cmd::run(&mut conn, action).await?;
    } else if let Some(Commands::Gitlab { action }) = cli.command {
        gitlab_cmd::run(&mut conn, action).await?;
    } else if let Some(prompt) = cli.prompt {
        // Headless mode
        let working_dir = cli.working_dir.unwrap_or_else(|| {
            std::env::current_dir()
                .expect("current directory must be accessible")
                .to_string_lossy()
                .to_string()
        });

        let config = HeadlessConfig {
            prompt,
            session_id,
            working_directory: working_dir,
            model: cli.model,
            auto_accept: cli.yes,
        };

        headless::run(&mut conn, config).await?;
    } else {
        // Interactive TUI mode
        betcode_cli::tui::run(&mut conn, &session_id, &cli.working_dir, &cli.model).await?;
    }

    Ok(())
}
