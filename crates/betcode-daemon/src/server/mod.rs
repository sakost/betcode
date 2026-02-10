//! gRPC server for BetCode daemon.
//!
//! Provides local socket and TCP server implementations.

mod agent;
pub mod command_svc;
mod config;
pub(crate) mod gitlab_convert;
mod gitlab_svc;
mod handler;
mod health;
mod worktree_svc;

#[cfg(test)]
mod gitlab_svc_tests;

pub use agent::AgentServiceImpl;
pub use command_svc::CommandServiceImpl;
pub use config::ServerConfig;
pub use gitlab_svc::GitLabServiceImpl;
pub use health::HealthServiceImpl;
pub use worktree_svc::WorktreeServiceImpl;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tonic::transport::Server;
use tracing::info;

use betcode_proto::v1::agent_service_server::AgentServiceServer;
use betcode_proto::v1::bet_code_health_server::BetCodeHealthServer;
use betcode_proto::v1::health_server::HealthServer;
use betcode_proto::v1::worktree_service_server::WorktreeServiceServer;

use crate::relay::SessionRelay;
use crate::session::SessionMultiplexer;
use crate::storage::Database;
use crate::subprocess::SubprocessManager;
use crate::worktree::WorktreeManager;

/// Server errors.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),
}

/// gRPC server handle.
pub struct GrpcServer {
    config: ServerConfig,
    db: Database,
    subprocess_manager: Arc<SubprocessManager>,
    multiplexer: Arc<SessionMultiplexer>,
    relay: Arc<SessionRelay>,
}

impl GrpcServer {
    /// Create a new gRPC server with all components wired together.
    pub fn new(config: ServerConfig, db: Database, subprocess_manager: SubprocessManager) -> Self {
        let subprocess_manager = Arc::new(subprocess_manager);
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = Arc::new(SessionRelay::new(
            Arc::clone(&subprocess_manager),
            Arc::clone(&multiplexer),
            db.clone(),
        ));

        Self {
            config,
            db,
            subprocess_manager,
            multiplexer,
            relay,
        }
    }

    /// Start serving on TCP socket.
    pub async fn serve_tcp(self, addr: SocketAddr) -> Result<(), ServerError> {
        let agent_service = AgentServiceImpl::new(
            self.db.clone(),
            Arc::clone(&self.relay),
            Arc::clone(&self.multiplexer),
        );
        let health_service =
            HealthServiceImpl::new(self.db.clone(), Arc::clone(&self.subprocess_manager));
        let worktree_service = WorktreeServiceImpl::new(WorktreeManager::new(self.db));

        info!(%addr, "Starting gRPC server on TCP");

        Server::builder()
            .http2_keepalive_interval(Some(Duration::from_secs(30)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .add_service(AgentServiceServer::new(agent_service))
            .add_service(HealthServer::new(health_service.clone()))
            .add_service(BetCodeHealthServer::new(health_service))
            .add_service(WorktreeServiceServer::new(worktree_service))
            .serve(addr)
            .await?;

        Ok(())
    }

    /// Start serving on Unix socket (non-Windows).
    #[cfg(unix)]
    pub async fn serve_unix(self, path: std::path::PathBuf) -> Result<(), ServerError> {
        use tokio::net::UnixListener;
        use tokio_stream::wrappers::UnixListenerStream;

        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&path)?;
        let stream = UnixListenerStream::new(listener);

        let agent_service = AgentServiceImpl::new(
            self.db.clone(),
            Arc::clone(&self.relay),
            Arc::clone(&self.multiplexer),
        );
        let health_service =
            HealthServiceImpl::new(self.db.clone(), Arc::clone(&self.subprocess_manager));
        let worktree_service = WorktreeServiceImpl::new(WorktreeManager::new(self.db));

        info!(path = %path.display(), "Starting gRPC server on Unix socket");

        Server::builder()
            .http2_keepalive_interval(Some(Duration::from_secs(30)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .add_service(AgentServiceServer::new(agent_service))
            .add_service(HealthServer::new(health_service.clone()))
            .add_service(BetCodeHealthServer::new(health_service))
            .add_service(WorktreeServiceServer::new(worktree_service))
            .serve_with_incoming(stream)
            .await?;

        Ok(())
    }

    /// Get the server configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Get a reference to the session relay (for tunnel handler).
    pub fn relay(&self) -> &Arc<SessionRelay> {
        &self.relay
    }

    /// Get a reference to the session multiplexer (for tunnel handler).
    pub fn multiplexer(&self) -> &Arc<SessionMultiplexer> {
        &self.multiplexer
    }

    /// Get a clone of the database (for tunnel handler).
    pub fn db(&self) -> &Database {
        &self.db
    }
}
