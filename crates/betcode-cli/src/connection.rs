//! Daemon connection client.
//!
//! Manages gRPC connection to the betcode-daemon.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};
use tracing::{error, info, warn};

use betcode_proto::v1::{
    agent_service_client::AgentServiceClient, git_lab_service_client::GitLabServiceClient,
    worktree_service_client::WorktreeServiceClient, AgentEvent, AgentRequest, CancelTurnRequest,
    CancelTurnResponse, CreateWorktreeRequest, GetIssueRequest, GetIssueResponse,
    GetMergeRequestRequest, GetMergeRequestResponse, GetPipelineRequest, GetPipelineResponse,
    GetWorktreeRequest, KeyExchangeRequest, ListIssuesRequest, ListIssuesResponse,
    ListMergeRequestsRequest, ListMergeRequestsResponse, ListPipelinesRequest,
    ListPipelinesResponse, ListSessionsRequest, ListSessionsResponse, ListWorktreesRequest,
    ListWorktreesResponse, RemoveWorktreeRequest, RemoveWorktreeResponse, ResumeSessionRequest,
    WorktreeDetail,
};

use betcode_crypto::{
    CryptoSession, FingerprintCheck, FingerprintStore, IdentityKeyPair, KeyExchangeState,
};

/// Connection configuration.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Target address (daemon for local, relay for remote).
    pub addr: String,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Request timeout.
    pub request_timeout: Duration,
    /// JWT token for relay authentication.
    pub auth_token: Option<String>,
    /// Machine ID for relay routing (sent as x-machine-id header).
    pub machine_id: Option<String>,
    /// Path to the client X25519 identity key file for E2E encryption.
    /// Defaults to `~/.betcode/client_identity.key`.
    pub identity_key_path: Option<std::path::PathBuf>,
    /// Path to the known daemons fingerprint store.
    /// Defaults to `~/.betcode/known_daemons.json`.
    pub fingerprint_store_path: Option<std::path::PathBuf>,
}

impl ConnectionConfig {
    /// Whether this config targets a relay (has auth + machine_id).
    pub fn is_relay(&self) -> bool {
        self.auth_token.is_some() && self.machine_id.is_some()
    }
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            addr: "http://127.0.0.1:50051".to_string(),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            auth_token: None,
            machine_id: None,
            identity_key_path: None,
            fingerprint_store_path: None,
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

/// Apply relay metadata to a tonic request.
fn apply_relay_meta<T>(
    req: &mut tonic::Request<T>,
    auth_token: &Option<String>,
    machine_id: &Option<String>,
) {
    if let Some(token) = auth_token {
        if let Ok(val) = format!("Bearer {}", token).parse() {
            req.metadata_mut().insert("authorization", val);
        }
    }
    if let Some(mid) = machine_id {
        if let Ok(val) = mid.parse() {
            req.metadata_mut().insert("x-machine-id", val);
        }
    }
}

/// Client connection to the daemon.
pub struct DaemonConnection {
    config: ConnectionConfig,
    client: Option<AgentServiceClient<Channel>>,
    worktree_client: Option<WorktreeServiceClient<Channel>>,
    gitlab_client: Option<GitLabServiceClient<Channel>>,
    state: ConnectionState,
    /// E2E crypto session, established via key exchange for relay connections.
    crypto: Option<std::sync::Arc<CryptoSession>>,
    /// Client identity keypair for E2E encryption key exchange.
    identity: Option<std::sync::Arc<IdentityKeyPair>>,
    /// TOFU fingerprint store for known daemons.
    fingerprint_store: FingerprintStore,
    /// Path where the fingerprint store is persisted.
    fingerprint_store_path: std::path::PathBuf,
}

impl DaemonConnection {
    /// Create a new connection (not yet connected).
    pub fn new(config: ConnectionConfig) -> Self {
        let fp_path = config.fingerprint_store_path.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".betcode")
                .join("known_daemons.json")
        });
        let fp_store = FingerprintStore::load(&fp_path).unwrap_or_default();

        let identity = Self::load_identity(&config);

        Self {
            config,
            client: None,
            worktree_client: None,
            gitlab_client: None,
            state: ConnectionState::Disconnected,
            crypto: None,
            identity,
            fingerprint_store: fp_store,
            fingerprint_store_path: fp_path,
        }
    }

    /// Load or generate the client identity keypair.
    fn load_identity(config: &ConnectionConfig) -> Option<std::sync::Arc<IdentityKeyPair>> {
        if !config.is_relay() {
            return None;
        }
        let path = config.identity_key_path.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".betcode")
                .join("client_identity.key")
        });
        match IdentityKeyPair::load_or_generate(&path) {
            Ok(kp) => {
                info!(fingerprint = %kp.fingerprint(), "Loaded client identity keypair");
                Some(std::sync::Arc::new(kp))
            }
            Err(e) => {
                warn!(?e, "Failed to load client identity key, proceeding without");
                None
            }
        }
    }

    /// Connect to the daemon.
    pub async fn connect(&mut self) -> Result<(), ConnectionError> {
        self.state = ConnectionState::Connecting;

        let endpoint = Endpoint::from_shared(self.config.addr.clone())
            .map_err(|e| ConnectionError::InvalidAddress(e.to_string()))?
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.request_timeout)
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10));

        let channel = endpoint.connect().await.map_err(|e| {
            self.state = ConnectionState::Disconnected;
            ConnectionError::ConnectFailed(e.to_string())
        })?;

        self.client = Some(AgentServiceClient::new(channel.clone()));
        self.worktree_client = Some(WorktreeServiceClient::new(channel.clone()));
        self.gitlab_client = Some(GitLabServiceClient::new(channel));
        self.state = ConnectionState::Connected;

        info!(addr = %self.config.addr, relay = self.config.is_relay(), "Connected");
        Ok(())
    }

    /// Perform E2E key exchange with the daemon via the relay.
    ///
    /// Generates an ephemeral X25519 keypair, sends the public key to the daemon,
    /// receives the daemon's ephemeral key, and derives a shared CryptoSession.
    /// Also performs TOFU fingerprint verification against the known daemons store.
    /// Must be called after `connect()` and before `converse()` for relay connections.
    ///
    /// Returns `(daemon_fingerprint, fingerprint_check)`.
    pub async fn exchange_keys(
        &mut self,
        machine_id: &str,
    ) -> Result<(String, FingerprintCheck), ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id_meta = self.config.machine_id.clone();
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        let state = match &self.identity {
            Some(id) => KeyExchangeState::with_identity(std::sync::Arc::clone(id)),
            None => KeyExchangeState::new(),
        };
        let our_pubkey = state.public_bytes();

        let (identity_pubkey, fingerprint_str) = match &self.identity {
            Some(id) => (id.public_bytes().to_vec(), id.fingerprint()),
            None => (Vec::new(), String::new()),
        };

        let mut request = tonic::Request::new(KeyExchangeRequest {
            machine_id: machine_id.to_string(),
            identity_pubkey,
            fingerprint: fingerprint_str,
            ephemeral_pubkey: our_pubkey.to_vec(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id_meta);

        let response = client
            .exchange_keys(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(format!("Key exchange failed: {}", e)))?;

        let resp = response.into_inner();
        let session = state
            .complete(&resp.daemon_ephemeral_pubkey)
            .map_err(|e| ConnectionError::RpcFailed(format!("Key derivation failed: {}", e)))?;

        let daemon_fingerprint = resp.daemon_fingerprint.clone();

        // Check TOFU fingerprint store
        let fp_check = self
            .fingerprint_store
            .check(machine_id, &daemon_fingerprint);

        match &fp_check {
            FingerprintCheck::TrustOnFirstUse => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                self.fingerprint_store
                    .record(machine_id, &daemon_fingerprint, now);
                if let Err(e) = self.fingerprint_store.save(&self.fingerprint_store_path) {
                    warn!(?e, "Failed to save fingerprint store");
                }
                info!(
                    daemon_fingerprint = %daemon_fingerprint,
                    machine_id = %machine_id,
                    "TOFU: first connection, fingerprint recorded"
                );
            }
            FingerprintCheck::Matched => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                self.fingerprint_store
                    .record(machine_id, &daemon_fingerprint, now);
                let _ = self.fingerprint_store.save(&self.fingerprint_store_path);
                info!(
                    daemon_fingerprint = %daemon_fingerprint,
                    "Daemon fingerprint verified (matches known)"
                );
            }
            FingerprintCheck::Mismatch { expected, actual } => {
                warn!(
                    machine_id = %machine_id,
                    expected = %expected,
                    actual = %actual,
                    "DAEMON FINGERPRINT MISMATCH — possible MITM attack!"
                );
                // Don't install the crypto session — caller decides what to do
                return Ok((daemon_fingerprint, fp_check));
            }
        }

        self.crypto = Some(std::sync::Arc::new(session));

        Ok((daemon_fingerprint, fp_check))
    }

    /// Accept a fingerprint mismatch (e.g., after user confirms the daemon key changed).
    ///
    /// Updates the stored fingerprint and allows future connections.
    pub fn accept_fingerprint_change(&mut self, machine_id: &str, new_fingerprint: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.fingerprint_store
            .update_fingerprint(machine_id, new_fingerprint, now);
        let _ = self.fingerprint_store.save(&self.fingerprint_store_path);
    }

    /// Mark a daemon's fingerprint as explicitly verified.
    pub fn verify_fingerprint(&mut self, machine_id: &str) {
        self.fingerprint_store.mark_verified(machine_id);
        let _ = self.fingerprint_store.save(&self.fingerprint_store_path);
    }

    /// Whether a crypto session has been established.
    pub fn has_crypto(&self) -> bool {
        self.crypto.is_some()
    }

    /// Get the client identity fingerprint, if loaded.
    pub fn client_fingerprint(&self) -> Option<String> {
        self.identity.as_ref().map(|id| id.fingerprint())
    }

    /// Get a reference to the fingerprint store.
    pub fn fingerprint_store(&self) -> &FingerprintStore {
        &self.fingerprint_store
    }

    /// Start a bidirectional conversation stream.
    ///
    /// Returns a sender for requests, a receiver for events, and a handle to
    /// the background stream reader task. Abort the handle on shutdown to avoid
    /// waiting for the server to close its end of the stream.
    pub async fn converse(
        &mut self,
    ) -> Result<
        (
            mpsc::Sender<AgentRequest>,
            mpsc::Receiver<Result<AgentEvent, tonic::Status>>,
            tokio::task::JoinHandle<()>,
        ),
        ConnectionError,
    > {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        // Channel for outgoing requests (client -> daemon)
        let (request_tx, request_rx) = mpsc::channel::<AgentRequest>(32);
        let request_stream = ReceiverStream::new(request_rx);

        // Call the bidirectional streaming RPC
        let mut request = tonic::Request::new(request_stream);
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .converse(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        let mut event_stream = response.into_inner();

        // Channel for incoming events (daemon -> client)
        let (event_tx, event_rx) = mpsc::channel::<Result<AgentEvent, tonic::Status>>(128);

        // Spawn task to forward events from the stream
        let stream_handle = tokio::spawn(async move {
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

        Ok((request_tx, event_rx, stream_handle))
    }

    /// List sessions.
    pub async fn list_sessions(
        &mut self,
        working_directory: Option<&str>,
    ) -> Result<ListSessionsResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ListSessionsRequest {
            working_directory: working_directory.unwrap_or_default().to_string(),
            worktree_id: String::new(),
            limit: 50,
            offset: 0,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_sessions(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Resume a session and replay historical events from a given sequence number.
    ///
    /// Returns all stored events for the session (from `from_sequence` onward).
    /// Used to populate the UI with conversation history before starting a new turn.
    pub async fn resume_session(
        &mut self,
        session_id: &str,
        from_sequence: u64,
    ) -> Result<Vec<AgentEvent>, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ResumeSessionRequest {
            session_id: session_id.to_string(),
            from_sequence,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);

        let response = client
            .resume_session(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        let mut stream = response.into_inner();
        let mut events = Vec::new();
        while let Some(event) = stream
            .message()
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?
        {
            events.push(event);
        }
        Ok(events)
    }

    /// Cancel the current turn in a session.
    pub async fn cancel_turn(
        &mut self,
        session_id: &str,
    ) -> Result<CancelTurnResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(CancelTurnRequest {
            session_id: session_id.to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .cancel_turn(request)
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
    // GitLab operations
    // =========================================================================

    /// List merge requests for a project.
    pub async fn list_merge_requests(
        &mut self,
        project: &str,
        state_filter: i32,
        limit: u32,
    ) -> Result<ListMergeRequestsResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .gitlab_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;
        let mut request = tonic::Request::new(ListMergeRequestsRequest {
            project: project.to_string(),
            state_filter,
            limit,
            offset: 0,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_merge_requests(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;
        Ok(response.into_inner())
    }

    /// Get a single merge request by IID.
    pub async fn get_merge_request(
        &mut self,
        project: &str,
        iid: u64,
    ) -> Result<GetMergeRequestResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .gitlab_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;
        let mut request = tonic::Request::new(GetMergeRequestRequest {
            project: project.to_string(),
            iid,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .get_merge_request(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;
        Ok(response.into_inner())
    }

    /// List pipelines for a project.
    pub async fn list_pipelines(
        &mut self,
        project: &str,
        status_filter: i32,
        limit: u32,
    ) -> Result<ListPipelinesResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .gitlab_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;
        let mut request = tonic::Request::new(ListPipelinesRequest {
            project: project.to_string(),
            status_filter,
            limit,
            offset: 0,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_pipelines(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;
        Ok(response.into_inner())
    }

    /// Get a single pipeline by ID.
    pub async fn get_pipeline(
        &mut self,
        project: &str,
        pipeline_id: u64,
    ) -> Result<GetPipelineResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .gitlab_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;
        let mut request = tonic::Request::new(GetPipelineRequest {
            project: project.to_string(),
            pipeline_id,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .get_pipeline(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;
        Ok(response.into_inner())
    }

    /// List issues for a project.
    pub async fn list_issues(
        &mut self,
        project: &str,
        state_filter: i32,
        limit: u32,
    ) -> Result<ListIssuesResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .gitlab_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;
        let mut request = tonic::Request::new(ListIssuesRequest {
            project: project.to_string(),
            state_filter,
            limit,
            offset: 0,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_issues(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;
        Ok(response.into_inner())
    }

    /// Get a single issue by IID.
    pub async fn get_issue(
        &mut self,
        project: &str,
        iid: u64,
    ) -> Result<GetIssueResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .gitlab_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;
        let mut request = tonic::Request::new(GetIssueRequest {
            project: project.to_string(),
            iid,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .get_issue(request)
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
        assert!(!config.is_relay());
        assert!(config.identity_key_path.is_none());
        assert!(config.fingerprint_store_path.is_none());
    }

    #[test]
    fn relay_config() {
        let config = ConnectionConfig {
            auth_token: Some("tok".into()),
            machine_id: Some("m1".into()),
            ..Default::default()
        };
        assert!(config.is_relay());
    }

    #[test]
    fn new_connection_is_disconnected() {
        let conn = DaemonConnection::new(ConnectionConfig::default());
        assert_eq!(conn.state(), ConnectionState::Disconnected);
        assert!(!conn.is_connected());
    }

    #[test]
    fn apply_relay_meta_adds_headers() {
        let token = Some("my-token".to_string());
        let mid = Some("m1".to_string());
        let mut req = tonic::Request::new(());
        apply_relay_meta(&mut req, &token, &mid);
        let auth = req
            .metadata()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(auth, "Bearer my-token");
        let machine = req
            .metadata()
            .get("x-machine-id")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(machine, "m1");
    }

    #[test]
    fn apply_relay_meta_noop_when_none() {
        let mut req = tonic::Request::new(());
        apply_relay_meta(&mut req, &None, &None);
        assert!(req.metadata().get("authorization").is_none());
        assert!(req.metadata().get("x-machine-id").is_none());
    }

    #[test]
    fn new_connection_has_no_crypto() {
        let conn = DaemonConnection::new(ConnectionConfig::default());
        assert!(!conn.has_crypto());
    }

    #[test]
    fn local_connection_has_no_identity() {
        let conn = DaemonConnection::new(ConnectionConfig::default());
        assert!(conn.client_fingerprint().is_none());
    }

    #[test]
    fn relay_connection_loads_identity() {
        let dir = std::env::temp_dir().join(format!("betcode-cli-id-{}", uuid::Uuid::new_v4()));
        let key_path = dir.join("client_identity.key");
        let fp_path = dir.join("known_daemons.json");

        let config = ConnectionConfig {
            auth_token: Some("tok".into()),
            machine_id: Some("m1".into()),
            identity_key_path: Some(key_path.clone()),
            fingerprint_store_path: Some(fp_path),
            ..Default::default()
        };
        let conn = DaemonConnection::new(config);
        assert!(conn.client_fingerprint().is_some());
        assert!(key_path.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fingerprint_store_starts_empty() {
        let conn = DaemonConnection::new(ConnectionConfig::default());
        assert!(conn.fingerprint_store().daemons.is_empty());
    }

    #[tokio::test]
    async fn exchange_keys_without_connection_returns_error() {
        let mut conn = DaemonConnection::new(ConnectionConfig::default());
        let result = conn.exchange_keys("m1").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ConnectionError::NotConnected => {}
            other => panic!("Expected NotConnected, got {:?}", other),
        }
    }
}
