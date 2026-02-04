//! BetCode Relay Server
//!
//! gRPC relay that routes requests through tunnels to daemon instances.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use betcode_proto::v1::agent_service_server::AgentServiceServer;
use betcode_proto::v1::auth_service_server::AuthServiceServer;
use betcode_proto::v1::machine_service_server::MachineServiceServer;
use betcode_proto::v1::tunnel_service_server::TunnelServiceServer;

use betcode_relay::auth::JwtManager;
use betcode_relay::registry::ConnectionRegistry;
use betcode_relay::router::RequestRouter;
use betcode_relay::server::{
    AgentProxyService, AuthServiceImpl, MachineServiceImpl, TunnelServiceImpl,
};
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

    /// JWT secret key.
    #[arg(
        long,
        env = "BETCODE_JWT_SECRET",
        default_value = "dev-secret-change-me"
    )]
    jwt_secret: String,

    /// Access token TTL in seconds.
    #[arg(long, default_value_t = 3600)]
    access_ttl: i64,

    /// Refresh token TTL in seconds.
    #[arg(long, default_value_t = 604800)]
    refresh_ttl: i64,

    /// Request forwarding timeout in seconds.
    #[arg(long, default_value_t = 30)]
    request_timeout: u64,
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

    let jwt = Arc::new(JwtManager::new(
        args.jwt_secret.as_bytes(),
        args.access_ttl,
        args.refresh_ttl,
    ));

    let registry = Arc::new(ConnectionRegistry::new());
    let router = Arc::new(RequestRouter::new(
        Arc::clone(&registry),
        Duration::from_secs(args.request_timeout),
    ));

    // Build services
    let auth = AuthServiceImpl::new(db.clone(), Arc::clone(&jwt));
    let tunnel = TunnelServiceImpl::new(Arc::clone(&registry), db.clone());
    let machine = MachineServiceImpl::new(db.clone());
    let agent_proxy = AgentProxyService::new(Arc::clone(&router));

    let jwt_check = betcode_relay::server::jwt_interceptor(Arc::clone(&jwt));

    info!(addr = %args.addr, "Relay server starting with all services");

    tokio::select! {
        result = Server::builder()
            .add_service(AuthServiceServer::new(auth))
            .add_service(TunnelServiceServer::with_interceptor(tunnel, jwt_check.clone()))
            .add_service(MachineServiceServer::with_interceptor(machine, jwt_check.clone()))
            .add_service(AgentServiceServer::with_interceptor(agent_proxy, jwt_check))
            .serve(args.addr) => {
            result?;
        }
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
