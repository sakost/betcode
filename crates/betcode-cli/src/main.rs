//! BetCode CLI
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

    // Warn if --relay was given but relay mode prerequisites are missing
    if cli_config.relay_url.is_some() && !cli_config.is_relay_mode() {
        if cli_config.auth.is_none() {
            anyhow::bail!(
                "Relay URL set but not logged in. Run: betcode --relay {} auth login -u <user> -p <pass>",
                cli_config.relay_url.as_ref().unwrap()
            );
        }
        if cli_config.active_machine.is_none() {
            anyhow::bail!(
                "Relay URL set but no active machine. Run: betcode --relay {} machine list",
                cli_config.relay_url.as_ref().unwrap()
            );
        }
    }

    // Refresh token before relay operations
    if cli_config.is_relay_mode() {
        if let Err(e) = auth_cmd::ensure_valid_token(&mut cli_config).await {
            eprintln!("Warning: token refresh failed: {}", e);
        }
    }

    // Build connection config: relay mode if we have auth + machine, else local daemon
    let conn_config = if cli_config.is_relay_mode() {
        ConnectionConfig {
            addr: cli_config.relay_url.clone().unwrap_or_default(),
            auth_token: cli_config.auth.as_ref().map(|a| a.access_token.clone()),
            machine_id: cli_config.active_machine.clone(),
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

    // Dispatch remaining subcommands or chat mode
    if let Some(Commands::Worktree { action }) = cli.command {
        worktree_cmd::run(&mut conn, action).await?;
    } else if let Some(Commands::Gitlab { action }) = cli.command {
        gitlab_cmd::run(&mut conn, action).await?;
    } else if let Some(prompt) = cli.prompt {
        // Headless mode
        let working_dir = cli.working_dir.unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string()
        });

        let config = HeadlessConfig {
            prompt,
            session_id: cli.session,
            working_directory: working_dir,
            model: cli.model,
            auto_accept: cli.yes,
        };

        headless::run(&mut conn, config).await?;
    } else {
        // Interactive TUI mode
        betcode_cli::tui::run(&mut conn, &cli.session, &cli.working_dir, &cli.model).await?;
    }

    Ok(())
}
