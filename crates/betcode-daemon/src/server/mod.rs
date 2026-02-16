//! gRPC server for `BetCode` daemon.
//!
//! Provides local socket and TCP server implementations.

mod agent;
pub mod command_svc;
mod config;
mod config_svc;
pub(crate) mod gitlab_convert;
mod gitlab_svc;
mod handler;
mod health;
mod repo_svc;
mod worktree_svc;

#[cfg(test)]
mod gitlab_svc_tests;

pub use agent::AgentServiceImpl;
pub use command_svc::CommandServiceImpl;
pub use config::ServerConfig;
pub use config_svc::ConfigServiceImpl;
pub use gitlab_svc::GitLabServiceImpl;
pub use health::HealthServiceImpl;
pub use repo_svc::GitRepoServiceImpl;
pub use worktree_svc::WorktreeServiceImpl;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tonic::transport::Server;
use tracing::{info, warn};

use tokio::sync::RwLock;

use betcode_proto::v1::agent_service_server::AgentServiceServer;
use betcode_proto::v1::bet_code_health_server::BetCodeHealthServer;
use betcode_proto::v1::command_service_server::CommandServiceServer;
use betcode_proto::v1::config_service_server::ConfigServiceServer;
use betcode_proto::v1::git_repo_service_server::GitRepoServiceServer;
use betcode_proto::v1::health_server::HealthServer;
use betcode_proto::v1::worktree_service_server::WorktreeServiceServer;

use crate::commands::CommandRegistry;
use crate::commands::service_executor::ServiceExecutor;
use crate::completion::agent_lister::AgentLister;
use crate::completion::file_index::FileIndex;
use crate::gitlab::{GitLabClient, GitLabConfig};
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
    command_service: CommandServiceImpl,
    config_service: ConfigServiceImpl,
    repo_service: GitRepoServiceImpl,
    worktree_service: WorktreeServiceImpl,
}

impl GrpcServer {
    /// Create a new gRPC server with all components wired together.
    ///
    /// `shutdown_tx` is used by the `exit-daemon` command to trigger graceful
    /// daemon shutdown.  The caller should subscribe to this channel and stop
    /// the server when `true` is received.
    pub async fn new(
        config: ServerConfig,
        db: Database,
        subprocess_manager: SubprocessManager,
        shutdown_tx: tokio::sync::watch::Sender<bool>,
        worktree_base_dir: std::path::PathBuf,
    ) -> Self {
        use crate::completion::agent_lister::{AgentInfo, AgentKind, AgentStatus};

        let subprocess_manager = Arc::new(subprocess_manager);
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());

        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let mut registry = CommandRegistry::new();

        // Discover Claude Code commands (hardcoded + user-defined)
        let discovery = crate::commands::cc_discovery::discover_all_cc_commands(&cwd, None);
        for cmd in discovery.commands {
            registry.add(cmd);
        }

        // Discover plugin commands from ~/.claude/
        let claude_dir = dirs::home_dir()
            .map(|h| h.join(".claude"))
            .unwrap_or_default();
        let plugin_entries = betcode_core::commands::discover_plugin_entries(&claude_dir);
        for entry in plugin_entries {
            registry.add(entry);
        }

        let command_registry = Arc::new(RwLock::new(registry));

        let relay = Arc::new(SessionRelay::new(
            Arc::clone(&subprocess_manager),
            Arc::clone(&multiplexer),
            db.clone(),
            Arc::clone(&command_registry),
        ));

        let file_index = Arc::new(RwLock::new(
            FileIndex::build(&cwd, 10_000)
                .await
                .unwrap_or_else(|_| FileIndex::empty()),
        ));
        let mut lister = AgentLister::new();
        for name in betcode_core::commands::discover_agents(&cwd) {
            lister.update(AgentInfo {
                name,
                kind: AgentKind::ClaudeInternal,
                status: AgentStatus::Idle,
                session_id: None,
            });
        }
        let agent_lister = Arc::new(RwLock::new(lister));
        let service_executor = Arc::new(RwLock::new(ServiceExecutor::new(cwd)));

        let command_service = CommandServiceImpl::new(
            command_registry,
            file_index,
            agent_lister,
            service_executor,
            shutdown_tx,
        );

        let config_service = ConfigServiceImpl::new(config.clone());

        let worktree_manager = WorktreeManager::new(db.clone(), worktree_base_dir);
        let repo_service = GitRepoServiceImpl::new(db.clone(), worktree_manager.clone());
        let worktree_service = WorktreeServiceImpl::new(worktree_manager, db.clone());

        Self {
            config,
            db,
            subprocess_manager,
            multiplexer,
            relay,
            command_service,
            config_service,
            repo_service,
            worktree_service,
        }
    }

    /// Build a `tonic::transport::server::Router` with all gRPC services wired in.
    ///
    /// Shared between `serve_tcp` and `serve_unix` to avoid duplicating the
    /// service-creation and server-builder code.
    async fn build_grpc_router(self) -> tonic::transport::server::Router {
        let agent_service = AgentServiceImpl::new(
            self.db.clone(),
            Arc::clone(&self.relay),
            Arc::clone(&self.multiplexer),
        );
        let health_service =
            HealthServiceImpl::new(self.db.clone(), Arc::clone(&self.subprocess_manager));

        let (grpc_health_reporter, grpc_health_service) = tonic_health::server::health_reporter();
        grpc_health_reporter
            .set_serving::<AgentServiceServer<AgentServiceImpl>>()
            .await;

        Server::builder()
            .http2_keepalive_interval(Some(Duration::from_secs(30)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .add_service(grpc_health_service)
            .add_service(AgentServiceServer::new(agent_service))
            .add_service(CommandServiceServer::new(self.command_service))
            .add_service(ConfigServiceServer::new(self.config_service))
            .add_service(GitRepoServiceServer::new(self.repo_service))
            .add_service(HealthServer::new(health_service.clone()))
            .add_service(BetCodeHealthServer::new(health_service))
            .add_service(WorktreeServiceServer::new(self.worktree_service))
    }

    /// Start serving on TCP socket.
    pub async fn serve_tcp(self, addr: SocketAddr) -> Result<(), ServerError> {
        info!(%addr, "Starting gRPC server on TCP");
        let router = self.build_grpc_router().await;
        router.serve(addr).await?;
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

        info!(path = %path.display(), "Starting gRPC server on Unix socket");
        let router = self.build_grpc_router().await;
        router.serve_with_incoming(stream).await?;
        Ok(())
    }

    /// Get the server configuration.
    pub const fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Get a reference to the session relay (for tunnel handler).
    pub const fn relay(&self) -> &Arc<SessionRelay> {
        &self.relay
    }

    /// Get a reference to the session multiplexer (for tunnel handler).
    pub const fn multiplexer(&self) -> &Arc<SessionMultiplexer> {
        &self.multiplexer
    }

    /// Get a clone of the database (for tunnel handler).
    pub const fn db(&self) -> &Database {
        &self.db
    }

    /// Get a clone of the `CommandServiceImpl` that shares state with the gRPC server.
    pub fn command_service_impl(&self) -> CommandServiceImpl {
        self.command_service.clone()
    }

    /// Get a clone of the `ConfigServiceImpl` that shares state with the gRPC server.
    pub fn config_service_impl(&self) -> ConfigServiceImpl {
        self.config_service.clone()
    }

    /// Get a clone of the `GitRepoServiceImpl` that shares state with the gRPC server.
    pub fn repo_service_impl(&self) -> GitRepoServiceImpl {
        self.repo_service.clone()
    }

    /// Get a clone of the `WorktreeServiceImpl` that shares state with the gRPC server.
    pub fn worktree_service_impl(&self) -> WorktreeServiceImpl {
        self.worktree_service.clone()
    }

    /// Try to build a `GitLabServiceImpl` from environment variables.
    ///
    /// Returns `Some` when both `BETCODE_GITLAB_URL` and `BETCODE_GITLAB_TOKEN`
    /// are set (and valid); returns `None` otherwise, meaning GitLab RPCs through
    /// the tunnel will respond with "not available".
    pub fn gitlab_service_impl_from_env() -> Option<GitLabServiceImpl> {
        let base_url = std::env::var("BETCODE_GITLAB_URL")
            .ok()
            .filter(|s| !s.is_empty())?;
        let token = std::env::var("BETCODE_GITLAB_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())?;
        let config = GitLabConfig { base_url, token };
        let client = match GitLabClient::new(&config) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to create GitLab client: {e}");
                return None;
            }
        };
        Some(GitLabServiceImpl::new(Arc::new(client)))
    }
}
