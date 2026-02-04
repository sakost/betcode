//! BetCode Daemon
//!
//! The daemon manages Claude Code subprocesses and serves the gRPC API
//! to clients (CLI, Flutter app) over local socket or relay tunnel.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use betcode_daemon::server::{GrpcServer, ServerConfig};
use betcode_daemon::storage::Database;
use betcode_daemon::subprocess::SubprocessManager;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode_daemon=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        addr = %args.addr,
        max_processes = args.max_processes,
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

    info!("Daemon stopped");
    Ok(())
}

/// Default database path: ~/.betcode/daemon.db
fn default_db_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".betcode").join("daemon.db"))
}
