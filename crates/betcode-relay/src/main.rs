//! `BetCode` Relay Server
//!
//! gRPC relay that routes requests through tunnels to daemon instances.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tonic::transport::Server;
use tracing::{info, warn};

use betcode_proto::v1::agent_service_server::AgentServiceServer;
use betcode_proto::v1::auth_service_server::AuthServiceServer;
use betcode_proto::v1::command_service_server::CommandServiceServer;
use betcode_proto::v1::config_service_server::ConfigServiceServer;
use betcode_proto::v1::git_lab_service_server::GitLabServiceServer;
use betcode_proto::v1::git_repo_service_server::GitRepoServiceServer;
use betcode_proto::v1::health_server::HealthServer;
use betcode_proto::v1::machine_service_server::MachineServiceServer;
use betcode_proto::v1::subagent_service_server::SubagentServiceServer;
use betcode_proto::v1::tunnel_service_server::TunnelServiceServer;
use betcode_proto::v1::worktree_service_server::WorktreeServiceServer;

use betcode_relay::auth::JwtManager;
use betcode_relay::buffer::BufferManager;
use betcode_relay::registry::ConnectionRegistry;
use betcode_relay::router::RequestRouter;
use betcode_relay::server::{
    AgentProxyService, AuthServiceImpl, CommandProxyService, ConfigProxyService,
    GitLabProxyService, GitRepoProxyService, MachineServiceImpl, RelayHealthService,
    SubagentProxyService, TunnelServiceImpl, WorktreeProxyService,
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

    /// Path to `SQLite` database file.
    #[arg(long, env = "BETCODE_DB_PATH")]
    db_path: Option<PathBuf>,

    /// JWT secret key (required; at least 32 bytes).
    #[arg(long, env = "BETCODE_JWT_SECRET")]
    jwt_secret: String,

    /// Access token TTL in seconds.
    #[arg(long, default_value_t = 3600)]
    access_ttl: i64,

    /// Refresh token TTL in seconds.
    #[arg(long, default_value_t = 604800)]
    refresh_ttl: i64,

    /// Grace period for refresh token rotation retries (seconds).
    #[arg(long, default_value_t = 30)]
    refresh_grace_period: i64,

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

    /// Path to CA certificate (PEM) for validating daemon client certificates
    /// (mutual TLS). When provided, tunnel connections must present a valid
    /// client certificate signed by this CA.
    #[arg(long)]
    mtls_ca_cert: Option<PathBuf>,

    /// TTL for buffered messages in seconds.
    #[arg(long, default_value_t = 86400)]
    buffer_ttl: i64,

    /// Maximum buffered messages per machine.
    #[arg(long, default_value_t = 1000)]
    buffer_cap: usize,

    /// Output logs as JSON (for structured log aggregation).
    #[arg(long)]
    log_json: bool,

    /// OpenTelemetry OTLP endpoint for traces and metrics export
    /// (e.g. `http://localhost:4317`). Requires the `metrics` feature.
    #[cfg(feature = "metrics")]
    #[arg(long, env = "BETCODE_METRICS_ENDPOINT")]
    metrics_endpoint: Option<String>,

    /// Path to FCM service account credentials JSON file.
    /// Required when the `push-notifications` feature is enabled.
    #[cfg(feature = "push-notifications")]
    #[arg(long, env = "BETCODE_FCM_CREDENTIALS_PATH")]
    fcm_credentials_path: PathBuf,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    #[cfg(feature = "metrics")]
    let metrics_endpoint = args.metrics_endpoint.as_deref();
    #[cfg(not(feature = "metrics"))]
    let metrics_endpoint: Option<&str> = None;

    // Hold the guard so the OTel pipeline stays alive for the process lifetime.
    let _metrics_guard = betcode_core::tracing_init::init_tracing_with_metrics(
        "betcode_relay=info",
        args.log_json,
        metrics_endpoint,
    );

    info!(
        version = env!("CARGO_PKG_VERSION"),
        addr = %args.addr,
        "Starting betcode-relay"
    );

    betcode_relay::auth::validate_jwt_secret(&args.jwt_secret)?;

    let db = if let Some(path) = &args.db_path {
        info!(path = %path.display(), "Opening relay database");
        RelayDatabase::open(path).await?
    } else {
        let default_path = default_db_path()?;
        info!(path = %default_path.display(), "Opening relay database (default path)");
        RelayDatabase::open(&default_path).await?
    };

    let jwt = Arc::new(JwtManager::new(
        args.jwt_secret.as_bytes(),
        args.access_ttl,
        args.refresh_ttl,
    ));

    let registry = Arc::new(ConnectionRegistry::new());
    let buffer = Arc::new(BufferManager::new(
        db.clone(),
        Arc::clone(&registry),
        args.buffer_ttl,
        args.buffer_cap,
    ));
    let router = Arc::new(RequestRouter::new(
        Arc::clone(&registry),
        Arc::clone(&buffer),
        Duration::from_secs(args.request_timeout),
    ));

    // mTLS is enabled when a client CA cert path is provided
    let mtls_enabled = args.mtls_ca_cert.is_some();

    // Build services
    let auth = AuthServiceImpl::new(db.clone(), Arc::clone(&jwt), args.refresh_grace_period);
    let tunnel = TunnelServiceImpl::new(
        Arc::clone(&registry),
        db.clone(),
        Arc::clone(&buffer),
        mtls_enabled,
    );
    let machine = MachineServiceImpl::new(db.clone());
    let agent_proxy = AgentProxyService::new(Arc::clone(&router), db.clone());
    let command_proxy = CommandProxyService::new(Arc::clone(&router), db.clone());
    let worktree_proxy = WorktreeProxyService::new(Arc::clone(&router), db.clone());
    let git_repo_proxy = GitRepoProxyService::new(Arc::clone(&router), db.clone());
    let config_proxy = ConfigProxyService::new(Arc::clone(&router), db.clone());
    let gitlab_proxy = GitLabProxyService::new(Arc::clone(&router), db.clone());
    let subagent_proxy = SubagentProxyService::new(Arc::clone(&router), db.clone());

    // Build notification service (only with push-notifications feature)
    #[cfg(feature = "push-notifications")]
    let notification_svc = {
        use betcode_relay::notifications::{FcmClient, NotificationServiceImpl};
        let fcm = FcmClient::from_credentials_file(&args.fcm_credentials_path)?;
        info!(
            project_id = %fcm.project_id(),
            "FCM push notifications enabled"
        );
        NotificationServiceImpl::new(db.clone(), fcm)
    };

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

    let mut tls_config = tls_mode.to_server_tls_config()?;

    // Apply mutual TLS if a client CA cert is provided
    if let Some(ca_path) = &args.mtls_ca_cert {
        if let Some(tls) = tls_config.take() {
            let ca_pem = std::fs::read_to_string(ca_path).map_err(|e| {
                anyhow::anyhow!("Failed to read mTLS CA cert {}: {e}", ca_path.display())
            })?;
            tls_config = Some(betcode_relay::tls::apply_mtls(tls, &ca_pem));
            info!(ca = %ca_path.display(), "Mutual TLS enabled for tunnel connections");
        } else {
            warn!("--mtls-ca-cert specified but TLS is disabled; mTLS will have no effect");
        }
    }

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

    // Standard grpc.health.v1 health service (no auth — used by load balancers/K8s)
    let (grpc_health_reporter, grpc_health_service) = tonic_health::server::health_reporter();
    grpc_health_reporter
        .set_serving::<AuthServiceServer<AuthServiceImpl>>()
        .await;

    let grpc_router = builder
        .add_service(grpc_health_service)
        .add_service(HealthServer::new(RelayHealthService::new()))
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
        .add_service(GitRepoServiceServer::with_interceptor(
            git_repo_proxy,
            jwt_check.clone(),
        ))
        .add_service(ConfigServiceServer::with_interceptor(
            config_proxy,
            jwt_check.clone(),
        ))
        .add_service(GitLabServiceServer::with_interceptor(
            gitlab_proxy,
            jwt_check.clone(),
        ))
        .add_service(SubagentServiceServer::with_interceptor(
            subagent_proxy,
            jwt_check.clone(),
        ));

    // Conditionally add notification service when push-notifications feature is enabled
    #[cfg(feature = "push-notifications")]
    let grpc_router = {
        use betcode_proto::v1::notification_service_server::NotificationServiceServer;
        grpc_router.add_service(NotificationServiceServer::with_interceptor(
            notification_svc,
            jwt_check,
        ))
    };

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
