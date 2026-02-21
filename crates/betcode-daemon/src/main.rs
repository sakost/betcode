//! `BetCode` Daemon
//!
//! The daemon manages Claude Code subprocesses and serves the gRPC API
//! to clients (CLI, Flutter app) over local socket or relay tunnel.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing::info;

use betcode_daemon::server::{GrpcServer, ServerConfig};
use betcode_daemon::storage::Database;
use betcode_daemon::subprocess::SubprocessManager;
use betcode_daemon::tunnel::{TunnelClient, TunnelConfig};

#[derive(Parser, Debug)]
#[command(name = "betcode-daemon")]
#[command(version, about = "BetCode daemon - Claude Code multiplexer")]
struct Args {
    /// TCP bind address
    #[arg(long, default_value = "127.0.0.1:50051", env = "BETCODE_ADDR")]
    addr: SocketAddr,

    /// Database file path
    #[arg(long, env = "BETCODE_DB_PATH")]
    db_path: Option<PathBuf>,

    /// Maximum concurrent Claude subprocesses
    #[arg(long, default_value_t = 5, env = "BETCODE_MAX_PROCESSES")]
    max_processes: usize,

    /// Maximum concurrent client sessions (gRPC connections); multiple sessions can share a subprocess
    #[arg(long, default_value_t = 10, env = "BETCODE_MAX_SESSIONS")]
    max_sessions: usize,

    /// Relay server URL (enables tunnel mode, e.g. "<https://relay.betcode.io:443>")
    #[arg(long, env = "BETCODE_RELAY_URL")]
    relay_url: Option<String>,

    /// Machine ID for relay registration
    #[arg(long, env = "BETCODE_MACHINE_ID")]
    machine_id: Option<String>,

    /// Human-readable machine name for relay
    #[arg(long, env = "BETCODE_MACHINE_NAME", default_value = "betcode-daemon")]
    machine_name: String,

    /// Username for relay authentication
    #[arg(long, env = "BETCODE_RELAY_USERNAME")]
    relay_username: Option<String>,

    /// Password for relay authentication
    #[arg(long, env = "BETCODE_RELAY_PASSWORD")]
    relay_password: Option<String>,

    /// Path to custom CA certificate for verifying the relay's TLS certificate (PEM).
    /// Use this for self-signed or development certificates.
    #[arg(long, env = "BETCODE_RELAY_CUSTOM_CA_CERT")]
    relay_custom_ca_cert: Option<PathBuf>,

    /// Path to PEM-encoded client certificate for mTLS with the relay.
    /// If not specified, auto-discovered from `$HOME/.betcode/certs/client.pem`.
    #[arg(long, env = "BETCODE_CLIENT_CERT")]
    client_cert: Option<PathBuf>,

    /// Path to PEM-encoded client private key for mTLS with the relay.
    /// If not specified, auto-discovered from `$HOME/.betcode/certs/client-key.pem`.
    #[arg(long, env = "BETCODE_CLIENT_KEY")]
    client_key: Option<PathBuf>,

    /// Base directory for git worktrees
    #[arg(long, env = "BETCODE_WORKTREE_DIR")]
    worktree_dir: Option<PathBuf>,

    /// Path to the `claude` CLI binary
    #[arg(long, default_value = "claude", env = "BETCODE_CLAUDE_BIN")]
    claude_bin: PathBuf,

    /// Log level filter for the daemon (e.g. "info", "debug", "warn").
    #[arg(long, default_value = "info", env = "BETCODE_LOG_LEVEL")]
    log_level: String,

    /// Default permission strategy for subprocesses.
    #[arg(
        long,
        default_value = "prompt-tool-stdio",
        env = "BETCODE_PERMISSION_STRATEGY",
        value_parser = ["prompt-tool-stdio", "skip-permissions"]
    )]
    permission_strategy: String,

    /// Seconds to wait for graceful subprocess shutdown before SIGKILL.
    #[arg(long, default_value_t = 5, env = "BETCODE_TERMINATE_TIMEOUT")]
    terminate_timeout: u64,

    /// Output logs as JSON (for structured log aggregation).
    #[arg(long, env = "BETCODE_LOG_JSON")]
    log_json: bool,

    /// OpenTelemetry OTLP endpoint for traces and metrics export
    /// (e.g. `http://localhost:4317`). Requires the `metrics` feature.
    #[cfg(feature = "metrics")]
    #[arg(long, env = "BETCODE_METRICS_ENDPOINT")]
    metrics_endpoint: Option<String>,
}

// jscpd:ignore-start -- binary bootstrap is inherently similar across daemons
#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    #[cfg(feature = "metrics")]
    let metrics_endpoint = args.metrics_endpoint.as_deref();
    #[cfg(not(feature = "metrics"))]
    let metrics_endpoint: Option<&str> = None;

    // Hold the guard so the OTel pipeline stays alive for the process lifetime.
    let log_filter = format!("betcode_daemon={}", args.log_level);
    let _metrics_guard = betcode_core::tracing_init::init_tracing_with_metrics(
        &log_filter,
        args.log_json,
        metrics_endpoint,
    );
    // jscpd:ignore-end

    info!(
        version = env!("CARGO_PKG_VERSION"),
        addr = %args.addr,
        max_processes = args.max_processes,
        relay = args.relay_url.is_some(),
        "Starting betcode-daemon"
    );

    // Initialize database
    let db = if let Some(path) = &args.db_path {
        info!(path = %path.display(), "Opening database");
        Database::open(path).await?
    } else {
        let default_path = default_db_path()?;
        info!(path = %default_path.display(), "Opening database (default path)");
        Database::open(&default_path).await?
    };

    // Parse permission strategy
    let default_permission_strategy = match args.permission_strategy.as_str() {
        "skip-permissions" => betcode_daemon::subprocess::PermissionStrategy::SkipPermissions,
        _ => betcode_daemon::subprocess::PermissionStrategy::PromptToolStdio,
    };

    // Create subprocess manager
    let subprocess_manager = SubprocessManager::with_options(
        args.max_processes,
        args.claude_bin.clone(),
        default_permission_strategy,
        args.terminate_timeout,
    );

    // Daemon-level shutdown channel (triggered by exit-daemon command or Ctrl+C)
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Subscribe early, before passing shutdown_tx to any component, to avoid
    // a race where a component could send the signal before we subscribe.
    let mut daemon_shutdown_rx = shutdown_tx.subscribe();

    // Resolve worktree base directory
    let worktree_dir = match args.worktree_dir {
        Some(dir) => dir,
        None => default_worktree_dir()?,
    };

    // Create and start gRPC server
    let config = ServerConfig::tcp(args.addr)
        .with_max_sessions(args.max_sessions)
        .with_max_processes(args.max_processes);
    let server = GrpcServer::new(
        config,
        db,
        subprocess_manager,
        shutdown_tx.clone(),
        worktree_dir,
        args.claude_bin,
    )
    .await;

    // Optionally spawn tunnel client
    let tunnel_handle = if let Some(relay_url) = &args.relay_url {
        let machine_id = args
            .machine_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let username = args.relay_username.clone().unwrap_or_default();
        let password = args.relay_password.clone().unwrap_or_default();

        let mut tunnel_config = TunnelConfig::new(
            relay_url.clone(),
            machine_id.clone(),
            args.machine_name.clone(),
            username,
            password,
        );
        tunnel_config
            .ca_cert_path
            .clone_from(&args.relay_custom_ca_cert);
        tunnel_config.client_cert_path.clone_from(&args.client_cert);
        tunnel_config.client_key_path.clone_from(&args.client_key);

        info!(
            relay_url = %relay_url,
            machine_id = %machine_id,
            "Spawning tunnel client"
        );

        let mut tunnel_client = TunnelClient::new(
            tunnel_config,
            Arc::clone(server.relay()),
            Arc::clone(server.multiplexer()),
            server.db().clone(),
        )?;
        tunnel_client.set_command_service(Arc::new(server.command_service_impl()));
        tunnel_client.set_repo_service(Arc::new(server.repo_service_impl()));
        tunnel_client.set_worktree_service(Arc::new(server.worktree_service_impl()));
        tunnel_client.set_config_service(Arc::new(server.config_service_impl()));
        tunnel_client.set_version_service(Arc::new(server.version_service_impl()));
        if let Some(gitlab_svc) = GrpcServer::gitlab_service_impl_from_env() {
            info!("GitLab service configured for tunnel");
            tunnel_client.set_gitlab_service(Arc::new(gitlab_svc));
        }
        Some(tokio::spawn(async move {
            tunnel_client.run(shutdown_rx).await;
        }))
    } else {
        drop(shutdown_rx);
        None
    };

    // Spawn certificate expiry monitor (checks daily, rotates if < 30 days to expiry)
    let cert_monitor_rx = shutdown_tx.subscribe();
    let cert_monitor_handle = betcode_daemon::tunnel::spawn_cert_monitor(cert_monitor_rx);

    // Serve until shutdown signal
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    // Notify systemd that the daemon is ready to serve (unix only).
    // The `true` parameter unsets $NOTIFY_SOCKET so child processes
    // (Claude Code subprocesses) don't accidentally notify systemd.
    #[cfg(unix)]
    sd_notify::notify(true, &[sd_notify::NotifyState::Ready])?;

    #[cfg(unix)]
    let sigterm_future = sigterm.recv();
    #[cfg(not(unix))]
    let sigterm_future = std::future::pending::<Option<()>>();

    info!(addr = %args.addr, "gRPC server ready");

    tokio::select! {
        result = server.serve_tcp(args.addr) => {
            result?;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C shutdown signal");
        }
        _ = sigterm_future => {
            info!("Received SIGTERM shutdown signal");
        }
        _ = daemon_shutdown_rx.changed() => {
            info!("Daemon shutdown requested via exit-daemon command");
        }
    }

    // Signal tunnel client and cert monitor to shut down
    let _ = shutdown_tx.send(true);
    if let Some(handle) = tunnel_handle {
        let _ = handle.await;
    }
    let _ = cert_monitor_handle.await;

    info!("Daemon stopped");
    Ok(())
}

/// Default database path: ~/.betcode/daemon.db
fn default_db_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".betcode").join("daemon.db"))
}

/// Default worktree base directory: ~/.betcode/worktrees/
fn default_worktree_dir() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".betcode").join("worktrees"))
}
