//! BetCode Relay Server
//!
//! gRPC relay that routes requests through tunnels to daemon instances.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tonic::transport::Server;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use betcode_proto::v1::agent_service_server::AgentServiceServer;
use betcode_proto::v1::auth_service_server::AuthServiceServer;
use betcode_proto::v1::command_service_server::CommandServiceServer;
use betcode_proto::v1::git_lab_service_server::GitLabServiceServer;
use betcode_proto::v1::machine_service_server::MachineServiceServer;
use betcode_proto::v1::tunnel_service_server::TunnelServiceServer;
use betcode_proto::v1::worktree_service_server::WorktreeServiceServer;

use betcode_relay::auth::JwtManager;
use betcode_relay::buffer::BufferManager;
use betcode_relay::registry::ConnectionRegistry;
use betcode_relay::router::RequestRouter;
use betcode_relay::server::{
    AgentProxyService, AuthServiceImpl, CommandProxyService, GitLabProxyService,
    MachineServiceImpl, TunnelServiceImpl, WorktreeProxyService,
};
use betcode_relay::storage::RelayDatabase;
use betcode_relay::tls::TlsMode;

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

    /// Enable dev TLS with auto-generated self-signed certificates.
    #[arg(long)]
    dev_tls: bool,

    /// Path to TLS certificate file (PEM). Mutually exclusive with --dev-tls.
    #[arg(long, requires = "tls_key")]
    tls_cert: Option<PathBuf>,

    /// Path to TLS private key file (PEM). Mutually exclusive with --dev-tls.
    #[arg(long, requires = "tls_cert")]
    tls_key: Option<PathBuf>,

    /// Output logs as JSON (for structured log aggregation).
    #[arg(long)]
    log_json: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let env_filter = tracing_subscriber::EnvFilter::new(
        std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode_relay=info".into()),
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
    let buffer = Arc::new(BufferManager::new(db.clone(), Arc::clone(&registry)));
    let router = Arc::new(RequestRouter::new(
        Arc::clone(&registry),
        Arc::clone(&buffer),
        Duration::from_secs(args.request_timeout),
    ));

    // Build services
    let auth = AuthServiceImpl::new(db.clone(), Arc::clone(&jwt));
    let tunnel = TunnelServiceImpl::new(Arc::clone(&registry), db.clone(), Arc::clone(&buffer));
    let machine = MachineServiceImpl::new(db.clone());
    let agent_proxy = AgentProxyService::new(Arc::clone(&router));
    let command_proxy = CommandProxyService::new(Arc::clone(&router));
    let worktree_proxy = WorktreeProxyService::new(Arc::clone(&router));
    let gitlab_proxy = GitLabProxyService::new(Arc::clone(&router));

    let jwt_check = betcode_relay::server::jwt_interceptor(Arc::clone(&jwt));

    // Determine TLS mode
    let tls_mode = if args.dev_tls {
        let cert_dir = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
            .join(".betcode")
            .join("certs");
        TlsMode::DevSelfSigned { cert_dir }
    } else if let (Some(cert), Some(key)) = (&args.tls_cert, &args.tls_key) {
        TlsMode::Custom {
            cert_path: cert.clone(),
            key_path: key.clone(),
        }
    } else {
        TlsMode::Disabled
    };

    let tls_config = tls_mode.to_server_tls_config()?;

    let mut builder = Server::builder()
        .http2_keepalive_interval(Some(Duration::from_secs(30)))
        .http2_keepalive_timeout(Some(Duration::from_secs(10)));
    if let Some(tls) = tls_config {
        builder = builder.tls_config(tls)?;
        info!(addr = %args.addr, "Relay server starting with TLS");
    } else {
        info!(addr = %args.addr, "Relay server starting (plaintext)");
    }

    // Spawn background task to clean up expired buffered messages (hourly)
    let cleanup_buffer = Arc::clone(&buffer);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.tick().await; // Skip first immediate tick
        loop {
            interval.tick().await;
            match cleanup_buffer.cleanup_expired().await {
                Ok(removed) if removed > 0 => {
                    info!(removed, "Background buffer cleanup completed");
                }
                Err(e) => {
                    warn!(error = %e, "Background buffer cleanup failed");
                }
                _ => {}
            }
        }
    });

    let grpc_router = builder
        .add_service(AuthServiceServer::new(auth))
        .add_service(TunnelServiceServer::with_interceptor(
            tunnel,
            jwt_check.clone(),
        ))
        .add_service(MachineServiceServer::with_interceptor(
            machine,
            jwt_check.clone(),
        ))
        .add_service(AgentServiceServer::with_interceptor(
            agent_proxy,
            jwt_check.clone(),
        ))
        .add_service(CommandServiceServer::with_interceptor(
            command_proxy,
            jwt_check.clone(),
        ))
        .add_service(WorktreeServiceServer::with_interceptor(
            worktree_proxy,
            jwt_check.clone(),
        ))
        .add_service(GitLabServiceServer::with_interceptor(
            gitlab_proxy,
            jwt_check,
        ));

    tokio::select! {
        result = grpc_router.serve(args.addr) => {
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
