//! gRPC server for BetCode daemon.
//!
//! Provides local socket and TCP server implementations.

mod agent;
mod config;
mod handler;

pub use agent::AgentServiceImpl;
pub use config::ServerConfig;

use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tonic::transport::Server;
use tracing::info;

use betcode_proto::v1::agent_service_server::AgentServiceServer;

use crate::relay::SessionRelay;
use crate::session::SessionMultiplexer;
use crate::storage::Database;
use crate::subprocess::SubprocessManager;

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
    multiplexer: Arc<SessionMultiplexer>,
    relay: Arc<SessionRelay>,
}

impl GrpcServer {
    /// Create a new gRPC server with all components wired together.
    pub fn new(config: ServerConfig, db: Database, subprocess_manager: SubprocessManager) -> Self {
        let subprocess_manager = Arc::new(subprocess_manager);
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = Arc::new(SessionRelay::new(
            subprocess_manager,
            Arc::clone(&multiplexer),
            db.clone(),
        ));

        Self {
            config,
            db,
            multiplexer,
            relay,
        }
    }

    /// Start serving on TCP socket.
    pub async fn serve_tcp(self, addr: SocketAddr) -> Result<(), ServerError> {
        let agent_service = AgentServiceImpl::new(
            self.db,
            Arc::clone(&self.relay),
            Arc::clone(&self.multiplexer),
        );

        info!(%addr, "Starting gRPC server on TCP");

        Server::builder()
            .add_service(AgentServiceServer::new(agent_service))
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
            self.db,
            Arc::clone(&self.relay),
            Arc::clone(&self.multiplexer),
        );

        info!(path = %path.display(), "Starting gRPC server on Unix socket");

        Server::builder()
            .add_service(AgentServiceServer::new(agent_service))
            .serve_with_incoming(stream)
            .await?;

        Ok(())
    }

    /// Get the server configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
}
