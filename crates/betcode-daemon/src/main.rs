//! BetCode Daemon
//!
//! The daemon manages Claude Code subprocesses and serves the gRPC API
//! to clients (CLI, Flutter app) over local socket or relay tunnel.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use betcode_daemon::server::{GrpcServer, ServerConfig};
use betcode_daemon::storage::Database;
use betcode_daemon::subprocess::SubprocessManager;
use betcode_daemon::tunnel::{TunnelClient, TunnelConfig};

#[derive(Parser, Debug)]
#[command(name = "betcode-daemon")]
#[command(version, about = "BetCode daemon - Claude Code multiplexer")]
struct Args {
    /// TCP bind address
    #[arg(long, default_value = "127.0.0.1:50051")]
    addr: SocketAddr,

    /// Database file path
    #[arg(long)]
    db_path: Option<PathBuf>,

    /// Maximum concurrent Claude subprocesses
    #[arg(long, default_value_t = 5)]
    max_processes: usize,

    /// Maximum concurrent sessions
    #[arg(long, default_value_t = 10)]
    max_sessions: usize,

    /// Relay server URL (enables tunnel mode, e.g. "https://relay.betcode.io:443")
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

    /// Path to CA certificate for verifying the relay's TLS certificate (PEM).
    #[arg(long, env = "BETCODE_RELAY_CA_CERT")]
    relay_ca_cert: Option<PathBuf>,

    /// Output logs as JSON (for structured log aggregation).
    #[arg(long)]
    log_json: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let env_filter = tracing_subscriber::EnvFilter::new(
        std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode_daemon=info".into()),
    );
    if args.log_json {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    info!(
        version = env!("CARGO_PKG_VERSION"),
        addr = %args.addr,
        max_processes = args.max_processes,
        relay = args.relay_url.is_some(),
        "Starting betcode-daemon"
    );

    // Initialize database
    let db = match &args.db_path {
        Some(path) => {
            info!(path = %path.display(), "Opening database");
            Database::open(path).await?
        }
        None => {
            let default_path = default_db_path()?;
            info!(path = %default_path.display(), "Opening database (default path)");
            Database::open(&default_path).await?
        }
    };

    // Create subprocess manager
    let subprocess_manager = SubprocessManager::new(args.max_processes);

    // Create and start gRPC server
    let config = ServerConfig::tcp(args.addr).with_max_sessions(args.max_sessions);
    let server = GrpcServer::new(config, db, subprocess_manager);

    // Shutdown signal for tunnel client
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

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
        tunnel_config.ca_cert_path = args.relay_ca_cert.clone();

        info!(
            relay_url = %relay_url,
            machine_id = %machine_id,
            "Spawning tunnel client"
        );

        let tunnel_client = TunnelClient::new(
            tunnel_config,
            Arc::clone(server.relay()),
            Arc::clone(server.multiplexer()),
            server.db().clone(),
        )?;
        Some(tokio::spawn(async move {
            tunnel_client.run(shutdown_rx).await;
        }))
    } else {
        drop(shutdown_rx);
        None
    };

    info!(addr = %args.addr, "gRPC server listening");

    // Serve until shutdown signal
    tokio::select! {
        result = server.serve_tcp(args.addr) => {
            result?;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    // Signal tunnel client to shut down
    let _ = shutdown_tx.send(true);
    if let Some(handle) = tunnel_handle {
        let _ = handle.await;
    }

    info!("Daemon stopped");
    Ok(())
}

/// Default database path: ~/.betcode/daemon.db
fn default_db_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".betcode").join("daemon.db"))
}
