//! BetCode Daemon
//!
//! The daemon manages Claude Code subprocesses and serves the gRPC API
//! to clients (CLI, Flutter app) over local socket or relay tunnel.

use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode_daemon=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Starting betcode-daemon"
    );

    // Placeholder - Sprint 1.3 will implement:
    // 1. Load configuration
    // 2. Initialize SQLite database
    // 3. Start subprocess manager
    // 4. Start gRPC server on local socket

    info!("Daemon scaffold ready. Implementation starts in Sprint 1.3.");

    Ok(())
}
