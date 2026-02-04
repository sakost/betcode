//! Daemon connection client.
//!
//! Manages gRPC connection to the betcode-daemon.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};
use tracing::{error, info, warn};

use betcode_proto::v1::{
    agent_service_client::AgentServiceClient, worktree_service_client::WorktreeServiceClient,
    AgentEvent, AgentRequest, CancelTurnRequest, CancelTurnResponse, CreateWorktreeRequest,
    GetWorktreeRequest, ListSessionsRequest, ListSessionsResponse, ListWorktreesRequest,
    ListWorktreesResponse, RemoveWorktreeRequest, RemoveWorktreeResponse, WorktreeDetail,
};

/// Connection configuration.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Daemon address (e.g., "http://127.0.0.1:50051").
    pub addr: String,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Request timeout.
    pub request_timeout: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            addr: "http://127.0.0.1:50051".to_string(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
        }
    }
}

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

/// Client connection to the daemon.
pub struct DaemonConnection {
    config: ConnectionConfig,
    client: Option<AgentServiceClient<Channel>>,
    worktree_client: Option<WorktreeServiceClient<Channel>>,
    state: ConnectionState,
}

impl DaemonConnection {
    /// Create a new connection (not yet connected).
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            client: None,
            worktree_client: None,
            state: ConnectionState::Disconnected,
        }
    }

    /// Connect to the daemon.
    pub async fn connect(&mut self) -> Result<(), ConnectionError> {
        self.state = ConnectionState::Connecting;

        let endpoint = Endpoint::from_shared(self.config.addr.clone())
            .map_err(|e| ConnectionError::InvalidAddress(e.to_string()))?
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.request_timeout);

        let channel = endpoint.connect().await.map_err(|e| {
            self.state = ConnectionState::Disconnected;
            ConnectionError::ConnectFailed(e.to_string())
        })?;

        self.client = Some(AgentServiceClient::new(channel.clone()));
        self.worktree_client = Some(WorktreeServiceClient::new(channel));
        self.state = ConnectionState::Connected;

        info!(addr = %self.config.addr, "Connected to daemon");
        Ok(())
    }

    /// Start a bidirectional conversation stream.
    pub async fn converse(
        &mut self,
    ) -> Result<
        (
            mpsc::Sender<AgentRequest>,
            mpsc::Receiver<Result<AgentEvent, tonic::Status>>,
        ),
        ConnectionError,
    > {
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        // Channel for outgoing requests (client -> daemon)
        let (request_tx, request_rx) = mpsc::channel::<AgentRequest>(32);
        let request_stream = ReceiverStream::new(request_rx);

        // Call the bidirectional streaming RPC
        let response = client
            .converse(request_stream)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        let mut event_stream = response.into_inner();

        // Channel for incoming events (daemon -> client)
        let (event_tx, event_rx) = mpsc::channel::<Result<AgentEvent, tonic::Status>>(128);

        // Spawn task to forward events from the stream
        tokio::spawn(async move {
            loop {
                match event_stream.message().await {
                    Ok(Some(event)) => {
                        if event_tx.send(Ok(event)).await.is_err() {
                            warn!("Event receiver dropped");
                            break;
                        }
                    }
                    Ok(None) => {
                        info!("Event stream ended");
                        break;
                    }
                    Err(e) => {
                        error!(?e, "Event stream error");
                        let _ = event_tx.send(Err(e)).await;
                        break;
                    }
                }
            }
        });

        Ok((request_tx, event_rx))
    }

    /// List sessions.
    pub async fn list_sessions(
        &mut self,
        working_directory: Option<&str>,
    ) -> Result<ListSessionsResponse, ConnectionError> {
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        let response = client
            .list_sessions(ListSessionsRequest {
                working_directory: working_directory.unwrap_or_default().to_string(),
                worktree_id: String::new(),
                limit: 50,
                offset: 0,
            })
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Cancel the current turn in a session.
    pub async fn cancel_turn(
        &mut self,
        session_id: &str,
    ) -> Result<CancelTurnResponse, ConnectionError> {
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        let response = client
            .cancel_turn(CancelTurnRequest {
                session_id: session_id.to_string(),
            })
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    // =========================================================================
    // Worktree operations
    // =========================================================================

    /// Create a new worktree.
    pub async fn create_worktree(
        &mut self,
        name: &str,
        repo_path: &str,
        branch: &str,
        setup_script: Option<&str>,
    ) -> Result<WorktreeDetail, ConnectionError> {
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let response = client
            .create_worktree(CreateWorktreeRequest {
                name: name.to_string(),
                repo_path: repo_path.to_string(),
                branch: branch.to_string(),
                setup_script: setup_script.unwrap_or_default().to_string(),
            })
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Remove a worktree.
    pub async fn remove_worktree(
        &mut self,
        id: &str,
    ) -> Result<RemoveWorktreeResponse, ConnectionError> {
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let response = client
            .remove_worktree(RemoveWorktreeRequest { id: id.to_string() })
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// List worktrees.
    pub async fn list_worktrees(
        &mut self,
        repo_path: Option<&str>,
    ) -> Result<ListWorktreesResponse, ConnectionError> {
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let response = client
            .list_worktrees(ListWorktreesRequest {
                repo_path: repo_path.unwrap_or_default().to_string(),
            })
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Get a single worktree.
    pub async fn get_worktree(&mut self, id: &str) -> Result<WorktreeDetail, ConnectionError> {
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let response = client
            .get_worktree(GetWorktreeRequest { id: id.to_string() })
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    // =========================================================================
    // Connection state
    // =========================================================================

    /// Get connection state.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected
    }
}

/// Connection errors.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Connection failed: {0}")]
    ConnectFailed(String),

    #[error("Not connected to daemon")]
    NotConnected,

    #[error("RPC call failed: {0}")]
    RpcFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = ConnectionConfig::default();
        assert_eq!(config.addr, "http://127.0.0.1:50051");
        assert_eq!(config.connect_timeout, Duration::from_secs(5));
    }

    #[test]
    fn new_connection_is_disconnected() {
        let conn = DaemonConnection::new(ConnectionConfig::default());
        assert_eq!(conn.state(), ConnectionState::Disconnected);
        assert!(!conn.is_connected());
    }
}
