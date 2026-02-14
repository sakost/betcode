use std::net::SocketAddr;

use clap::Parser;
use tracing::info;

use betcode_releases::routes;

#[derive(Parser)]
struct Args {
    /// Listen address
    #[arg(long, default_value = "0.0.0.0:8080", env = "LISTEN_ADDR")]
    addr: SocketAddr,

    /// GitHub repository (owner/repo)
    #[arg(long, default_value = "sakost/betcode", env = "GITHUB_REPO")]
    repo: String,

    /// Base URL for the download server
    #[arg(long, default_value = "get.betcode.dev", env = "BASE_URL")]
    base_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let state = routes::AppState {
        repo: args.repo,
        base_url: args.base_url,
    };

    let app = routes::build_router(state);

    info!(addr = %args.addr, "starting betcode-releases");
    let listener = tokio::net::TcpListener::bind(args.addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
