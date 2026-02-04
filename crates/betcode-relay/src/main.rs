//! BetCode Relay Server
//!
//! gRPC relay that routes requests through tunnels to daemon instances.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use betcode_relay::storage::RelayDatabase;

#[derive(Parser, Debug)]
#[command(name = "betcode-relay")]
#[command(
    version,
    about = "BetCode relay server - gRPC router and tunnel manager"
)]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "0.0.0.0:443")]
    addr: SocketAddr,

    /// Path to SQLite database file.
    #[arg(long)]
    db_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode_relay=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        addr = %args.addr,
        "Starting betcode-relay"
    );

    let db = match &args.db_path {
        Some(path) => {
            info!(path = %path.display(), "Opening relay database");
            RelayDatabase::open(path).await?
        }
        None => {
            let default_path = default_db_path()?;
            info!(path = %default_path.display(), "Opening relay database (default path)");
            RelayDatabase::open(&default_path).await?
        }
    };

    // TODO: Wire gRPC services in Sprint 3.5
    let _db = db;

    info!(addr = %args.addr, "Relay server ready (services pending)");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    info!("Relay stopped");
    Ok(())
}

fn default_db_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".betcode").join("relay.db"))
}
