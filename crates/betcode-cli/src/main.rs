//! BetCode CLI
//!
//! Terminal interface for interacting with Claude Code through the daemon.
//! Provides both TUI (ratatui) and headless modes.

use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "betcode")]
#[command(version, about = "Claude Code multiplexer CLI", long_about = None)]
struct Cli {
    /// Run in headless mode (no TUI)
    #[arg(long)]
    headless: bool,

    /// Session ID to resume (creates new if not specified)
    #[arg(short, long)]
    session: Option<String>,

    /// Working directory for the session
    #[arg(short = 'd', long)]
    working_dir: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        headless = cli.headless,
        session = ?cli.session,
        working_dir = ?cli.working_dir,
        "Starting betcode CLI"
    );

    // Placeholder - Sprint 1.5 will implement:
    // 1. Connect to local daemon
    // 2. Start TUI or headless mode
    // 3. Handle conversation flow

    info!("CLI scaffold ready. Implementation starts in Sprint 1.5.");

    Ok(())
}
