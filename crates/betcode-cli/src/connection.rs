//! Daemon connection client.
//!
//! Manages gRPC connection to the betcode-daemon.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use tracing::{error, info, warn};

use betcode_proto::v1::{
    agent_service_client::AgentServiceClient, command_service_client::CommandServiceClient,
    git_lab_service_client::GitLabServiceClient,
    git_repo_service_client::GitRepoServiceClient,
    worktree_service_client::WorktreeServiceClient, AddPluginRequest, AddPluginResponse,
    AgentEvent, AgentRequest, CancelTurnRequest, CancelTurnResponse, CreateWorktreeRequest,
    DisablePluginRequest, DisablePluginResponse, EnablePluginRequest, EnablePluginResponse,
    ExecuteServiceCommandRequest, GetCommandRegistryResponse, GetIssueRequest, GetIssueResponse,
    GetMergeRequestRequest, GetMergeRequestResponse, GetPipelineRequest, GetPipelineResponse,
    GetPluginStatusRequest, GetPluginStatusResponse, GetRepoRequest, GetWorktreeRequest,
    GitRepoDetail, KeyExchangeRequest, ListAgentsRequest, ListAgentsResponse, ListIssuesRequest,
    ListIssuesResponse, ListMergeRequestsRequest, ListMergeRequestsResponse, ListPathRequest,
    ListPathResponse, ListPipelinesRequest, ListPipelinesResponse, ListPluginsRequest,
    ListPluginsResponse, ListReposRequest, ListReposResponse, ListSessionsRequest,
    ListSessionsResponse, ListWorktreesRequest, ListWorktreesResponse, RegisterRepoRequest,
    RemovePluginRequest, RemovePluginResponse, RemoveWorktreeRequest, RemoveWorktreeResponse,
    ResumeSessionRequest, ScanReposRequest, ServiceCommandOutput, UnregisterRepoRequest,
    UnregisterRepoResponse, UpdateRepoRequest, WorktreeDetail,
};

use betcode_crypto::{
    CryptoSession, FingerprintCheck, FingerprintStore, IdentityKeyPair, KeyExchangeState,
};

/// Attach relay authorization and machine-id headers to a gRPC request.
///
/// Intended for use inside spawned tasks where only cloned `token/machine_id`
/// strings are available (not a full `DaemonConnection` reference).
pub fn attach_relay_metadata<T>(
    request: &mut tonic::Request<T>,
    auth_token: Option<&str>,
    machine_id: Option<&str>,
) {
    if let Some(token) = auth_token {
        if let Ok(val) = format!("Bearer {token}").parse() {
            request.metadata_mut().insert("authorization", val);
        }
    }
    if let Some(mid) = machine_id {
        if let Ok(val) = mid.parse() {
            request.metadata_mut().insert("x-machine-id", val);
        }
    }
}

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
    /// Path to CA certificate for verifying the relay's TLS certificate.
    pub ca_cert_path: Option<std::path::PathBuf>,
}

impl ConnectionConfig {
    /// Whether this config targets a relay (has auth + `machine_id`).
    pub const fn is_relay(&self) -> bool {
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
            ca_cert_path: None,
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
    #![allow(clippy::ref_option)]
    if let Some(token) = auth_token {
        if let Ok(val) = format!("Bearer {token}").parse() {
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
    git_repo_client: Option<GitRepoServiceClient<Channel>>,
    command_client: Option<CommandServiceClient<Channel>>,
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
            git_repo_client: None,
            command_client: None,
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

        let mut endpoint = Endpoint::from_shared(self.config.addr.clone())
            .map_err(|e| ConnectionError::InvalidAddress(e.to_string()))?
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.request_timeout)
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10));

        // Configure TLS if CA cert is provided (for self-signed relay certs)
        if let Some(ca_path) = &self.config.ca_cert_path {
            let ca_pem = std::fs::read_to_string(ca_path).map_err(|e| {
                ConnectionError::ConnectFailed(format!(
                    "Failed to read CA cert {}: {}",
                    ca_path.display(),
                    e
                ))
            })?;
            let tls_config = ClientTlsConfig::new()
                .with_enabled_roots()
                .ca_certificate(Certificate::from_pem(ca_pem));
            endpoint = endpoint
                .tls_config(tls_config)
                .map_err(|e| ConnectionError::ConnectFailed(e.to_string()))?;
            info!(ca_cert = %ca_path.display(), "TLS configured with custom CA");
        }

        let channel = endpoint.connect().await.map_err(|e| {
            self.state = ConnectionState::Disconnected;
            ConnectionError::ConnectFailed(e.to_string())
        })?;

        self.client = Some(AgentServiceClient::new(channel.clone()));
        self.worktree_client = Some(WorktreeServiceClient::new(channel.clone()));
        self.gitlab_client = Some(GitLabServiceClient::new(channel.clone()));
        self.git_repo_client = Some(GitRepoServiceClient::new(channel.clone()));
        self.command_client = Some(CommandServiceClient::new(channel));
        self.state = ConnectionState::Connected;

        info!(addr = %self.config.addr, relay = self.config.is_relay(), "Connected");
        Ok(())
    }

    /// Perform E2E key exchange with the daemon via the relay.
    ///
    /// Generates an ephemeral X25519 keypair, sends the public key to the daemon,
    /// receives the daemon's ephemeral key, and derives a shared `CryptoSession`.
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

        let state = self.identity.as_ref().map_or_else(
            KeyExchangeState::new,
            |id| KeyExchangeState::with_identity(std::sync::Arc::clone(id)),
        );
        let our_pubkey = state.public_bytes();

        let (identity_pubkey, fingerprint_str) = self.identity.as_ref().map_or_else(
            || (Vec::new(), String::new()),
            |id| (id.public_bytes().to_vec(), id.fingerprint()),
        );

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
            .map_err(|e| ConnectionError::RpcFailed(format!("Key exchange failed: {e}")))?;

        let resp = response.into_inner();
        let session = state
            .complete(&resp.daemon_ephemeral_pubkey)
            .map_err(|e| ConnectionError::RpcFailed(format!("Key derivation failed: {e}")))?;

        let daemon_fingerprint = resp.daemon_fingerprint;

        // Check TOFU fingerprint store
        let fp_check = self
            .fingerprint_store
            .check(machine_id, &daemon_fingerprint);

        match &fp_check {
            FingerprintCheck::TrustOnFirstUse => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .try_into()
                    .unwrap_or(i64::MAX);
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
                    .as_secs()
                    .try_into()
                    .unwrap_or(i64::MAX);
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
            .as_secs()
            .try_into()
            .unwrap_or(i64::MAX);
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
    pub const fn has_crypto(&self) -> bool {
        self.crypto.is_some()
    }

    /// Get the client identity fingerprint, if loaded.
    pub fn client_fingerprint(&self) -> Option<String> {
        self.identity.as_ref().map(|id| id.fingerprint())
    }

    /// Get a reference to the fingerprint store.
    pub const fn fingerprint_store(&self) -> &FingerprintStore {
        &self.fingerprint_store
    }

    /// Whether this connection targets a relay (has auth + `machine_id` configured).
    pub const fn is_relay(&self) -> bool {
        self.config.is_relay()
    }

    /// Get the configured machine ID for relay routing.
    pub fn machine_id(&self) -> Option<&str> {
        self.config.machine_id.as_deref()
    }

    /// Get the auth token for relay connections.
    pub const fn auth_token(&self) -> Option<&String> {
        self.config.auth_token.as_ref()
    }

    /// Start a bidirectional conversation stream.
    ///
    /// Returns a sender for requests, a receiver for events, and a handle to
    /// the background stream reader task. Abort the handle on shutdown to avoid
    /// waiting for the server to close its end of the stream.
    ///
    /// For relay connections, outgoing requests are encrypted and incoming events
    /// are decrypted using the established `CryptoSession`. If no crypto session
    /// exists on a relay connection, returns `KeyExchangeRequired`.
    #[allow(clippy::too_many_lines)]
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
        // Guard: relay connections require E2E encryption
        if self.is_relay() && self.crypto.is_none() {
            return Err(ConnectionError::KeyExchangeRequired);
        }

        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let crypto = self.crypto.clone();
        let client = self.client.as_mut().ok_or(ConnectionError::NotConnected)?;

        // Channel for incoming events (daemon -> client).
        // Created early so the encrypt adapter can send errors through it.
        let (event_tx, event_rx) = mpsc::channel::<Result<AgentEvent, tonic::Status>>(128);

        // Channel for outgoing requests (client -> daemon)
        // If crypto is active, we insert an encrypting adapter between the
        // user-facing sender and the gRPC stream.
        let (user_tx, user_rx) = mpsc::channel::<AgentRequest>(32);

        let grpc_rx = if let Some(ref session) = crypto {
            let (enc_tx, enc_rx) = mpsc::channel::<AgentRequest>(32);
            let session = std::sync::Arc::clone(session);
            let err_tx = event_tx.clone();
            tokio::spawn(async move {
                let mut user_rx = user_rx;
                while let Some(req) = user_rx.recv().await {
                    match encrypt_agent_request(&session, &req) {
                        Ok(encrypted) => {
                            if enc_tx.send(encrypted).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Failed to encrypt outgoing request: {}", e);
                            let _ = err_tx
                                .send(Err(tonic::Status::internal(format!(
                                    "Encryption failed: {e}"
                                ))))
                                .await;
                            break;
                        }
                    }
                }
            });
            enc_rx
        } else {
            user_rx
        };

        let request_stream = ReceiverStream::new(grpc_rx);

        // Call the bidirectional streaming RPC
        let mut request = tonic::Request::new(request_stream);
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .converse(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        let mut event_stream = response.into_inner();

        // Spawn task to forward events from the stream, decrypting if needed
        let stream_handle = tokio::spawn(async move {
            const MAX_CONSECUTIVE_DECRYPT_FAILURES: u32 = 5;
            let mut consecutive_failures: u32 = 0;

            loop {
                match event_stream.message().await {
                    Ok(Some(event)) => {
                        let event = if let Some(ref session) = crypto {
                            match decrypt_agent_event(session, &event) {
                                Ok(decrypted) => {
                                    consecutive_failures = 0;
                                    decrypted
                                }
                                Err(e) => {
                                    consecutive_failures += 1;
                                    error!(
                                        "Failed to decrypt incoming event ({}/{}): {}",
                                        consecutive_failures, MAX_CONSECUTIVE_DECRYPT_FAILURES, e
                                    );
                                    if consecutive_failures >= MAX_CONSECUTIVE_DECRYPT_FAILURES {
                                        error!("Too many consecutive decryption failures, closing stream");
                                        let _ = event_tx
                                            .send(Err(tonic::Status::internal(
                                                "Persistent decryption failures — possible key mismatch",
                                            )))
                                            .await;
                                        break;
                                    }
                                    continue;
                                }
                            }
                        } else {
                            event
                        };
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

        Ok((user_tx, event_rx, stream_handle))
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
    /// When E2E crypto is active, events are decrypted before returning.
    pub async fn resume_session(
        &mut self,
        session_id: &str,
        from_sequence: u64,
    ) -> Result<Vec<AgentEvent>, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let crypto = self.crypto.clone();
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
        Ok(decrypt_resume_events(crypto.as_ref(), events))
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
        repo_id: &str,
        branch: &str,
        setup_script: Option<&str>,
    ) -> Result<WorktreeDetail, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(CreateWorktreeRequest {
            name: name.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            setup_script: setup_script.unwrap_or_default().to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .create_worktree(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Remove a worktree.
    pub async fn remove_worktree(
        &mut self,
        id: &str,
    ) -> Result<RemoveWorktreeResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(RemoveWorktreeRequest { id: id.to_string() });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .remove_worktree(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// List worktrees.
    pub async fn list_worktrees(
        &mut self,
        repo_id: Option<&str>,
    ) -> Result<ListWorktreesResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ListWorktreesRequest {
            repo_id: repo_id.unwrap_or_default().to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_worktrees(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Get a single worktree.
    pub async fn get_worktree(&mut self, id: &str) -> Result<WorktreeDetail, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .worktree_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(GetWorktreeRequest { id: id.to_string() });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .get_worktree(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    // =========================================================================
    // Git repo operations
    // =========================================================================

    /// Register a new git repository.
    #[allow(clippy::too_many_arguments)]
    pub async fn register_repo(
        &mut self,
        repo_path: &str,
        name: &str,
        worktree_mode: i32,
        local_subfolder: &str,
        custom_path: &str,
        setup_script: &str,
        auto_gitignore: bool,
    ) -> Result<GitRepoDetail, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .git_repo_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(RegisterRepoRequest {
            repo_path: repo_path.to_string(),
            name: name.to_string(),
            worktree_mode,
            local_subfolder: local_subfolder.to_string(),
            custom_path: custom_path.to_string(),
            setup_script: setup_script.to_string(),
            auto_gitignore,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .register_repo(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Unregister a repository.
    pub async fn unregister_repo(
        &mut self,
        id: &str,
        remove_worktrees: bool,
    ) -> Result<UnregisterRepoResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .git_repo_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(UnregisterRepoRequest {
            id: id.to_string(),
            remove_worktrees,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .unregister_repo(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// List registered repositories.
    pub async fn list_repos(
        &mut self,
        limit: u32,
        offset: u32,
    ) -> Result<ListReposResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .git_repo_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ListReposRequest { limit, offset });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_repos(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Get a single repository by ID.
    pub async fn get_repo(&mut self, id: &str) -> Result<GitRepoDetail, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .git_repo_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(GetRepoRequest { id: id.to_string() });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .get_repo(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Update repository configuration.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_repo(
        &mut self,
        id: &str,
        name: Option<&str>,
        worktree_mode: Option<i32>,
        local_subfolder: Option<&str>,
        custom_path: Option<&str>,
        setup_script: Option<&str>,
        auto_gitignore: Option<bool>,
    ) -> Result<GitRepoDetail, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .git_repo_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(UpdateRepoRequest {
            id: id.to_string(),
            name: name.map(String::from),
            worktree_mode,
            local_subfolder: local_subfolder.map(String::from),
            custom_path: custom_path.map(String::from),
            setup_script: setup_script.map(String::from),
            auto_gitignore,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .update_repo(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Scan a directory for git repositories.
    pub async fn scan_repos(
        &mut self,
        scan_path: &str,
        max_depth: u32,
    ) -> Result<ListReposResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .git_repo_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ScanReposRequest {
            scan_path: scan_path.to_string(),
            max_depth,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .scan_repos(request)
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
    // =========================================================================
    // Command service methods
    // =========================================================================

    /// Get a clone of the `CommandServiceClient` for use in background tasks.
    pub fn command_service_client(&self) -> Option<CommandServiceClient<Channel>> {
        self.command_client.clone()
    }

    /// Fetch the full command registry from the daemon.
    pub async fn get_command_registry(
        &mut self,
    ) -> Result<GetCommandRegistryResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(betcode_proto::v1::GetCommandRegistryRequest {});
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .get_command_registry(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// List agents matching a query for @-mention completion.
    pub async fn list_agents(
        &mut self,
        query: &str,
        max_results: u32,
    ) -> Result<ListAgentsResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ListAgentsRequest {
            query: query.to_string(),
            max_results,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_agents(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// List file paths matching a query for @file completion.
    pub async fn list_path(
        &mut self,
        query: &str,
        max_results: u32,
    ) -> Result<ListPathResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ListPathRequest {
            query: query.to_string(),
            max_results,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_path(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Execute a service command and return a stream of output.
    pub async fn execute_service_command(
        &mut self,
        command: &str,
        args: Vec<String>,
    ) -> Result<tonic::Streaming<ServiceCommandOutput>, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ExecuteServiceCommandRequest {
            command: command.to_string(),
            args,
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .execute_service_command(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// List all registered plugins.
    pub async fn list_plugins(&mut self) -> Result<ListPluginsResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(ListPluginsRequest {});
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .list_plugins(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Get the status of a specific plugin.
    pub async fn get_plugin_status(
        &mut self,
        name: &str,
    ) -> Result<GetPluginStatusResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(GetPluginStatusRequest {
            name: name.to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .get_plugin_status(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Register a new plugin.
    pub async fn add_plugin(
        &mut self,
        name: &str,
        socket_path: &str,
    ) -> Result<AddPluginResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(AddPluginRequest {
            name: name.to_string(),
            socket_path: socket_path.to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .add_plugin(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Remove a registered plugin.
    pub async fn remove_plugin(
        &mut self,
        name: &str,
    ) -> Result<RemovePluginResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(RemovePluginRequest {
            name: name.to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .remove_plugin(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Enable a disabled plugin.
    pub async fn enable_plugin(
        &mut self,
        name: &str,
    ) -> Result<EnablePluginResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(EnablePluginRequest {
            name: name.to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .enable_plugin(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    /// Disable a plugin without removing it.
    pub async fn disable_plugin(
        &mut self,
        name: &str,
    ) -> Result<DisablePluginResponse, ConnectionError> {
        let auth_token = self.config.auth_token.clone();
        let machine_id = self.config.machine_id.clone();
        let client = self
            .command_client
            .as_mut()
            .ok_or(ConnectionError::NotConnected)?;

        let mut request = tonic::Request::new(DisablePluginRequest {
            name: name.to_string(),
        });
        apply_relay_meta(&mut request, &auth_token, &machine_id);
        let response = client
            .disable_plugin(request)
            .await
            .map_err(|e| ConnectionError::RpcFailed(e.to_string()))?;

        Ok(response.into_inner())
    }

    // =========================================================================
    // Connection state
    // =========================================================================

    /// Get connection state.
    pub const fn state(&self) -> ConnectionState {
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

    #[error("Key exchange required: relay connections require E2E encryption")]
    KeyExchangeRequired,

    #[error("Fingerprint rejected: daemon fingerprint mismatch")]
    FingerprintRejected,
}

/// Encrypt an `AgentRequest` by serializing it, encrypting the bytes, and
/// wrapping in a new `AgentRequest` with the `Encrypted` oneof variant.
pub(crate) fn encrypt_agent_request(
    session: &betcode_crypto::CryptoSession,
    req: &AgentRequest,
) -> Result<AgentRequest, String> {
    use prost::Message;
    let mut buf = Vec::with_capacity(req.encoded_len());
    req.encode(&mut buf)
        .map_err(|e| format!("encode failed: {e}"))?;
    let encrypted = session
        .encrypt(&buf)
        .map_err(|e| format!("encrypt failed: {e}"))?;
    Ok(AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Encrypted(
            betcode_proto::v1::EncryptedEnvelope {
                ciphertext: encrypted.ciphertext,
                nonce: encrypted.nonce.to_vec(),
            },
        )),
    })
}

/// Decrypt an `AgentEvent` that contains an `Encrypted` variant.
/// Rejects non-encrypted events to prevent relay-injected plaintext attacks.
pub(crate) fn decrypt_agent_event(
    session: &betcode_crypto::CryptoSession,
    event: &AgentEvent,
) -> Result<AgentEvent, String> {
    use prost::Message;
    match event.event {
        Some(betcode_proto::v1::agent_event::Event::Encrypted(ref envelope)) => {
            let plaintext = session
                .decrypt(&envelope.ciphertext, &envelope.nonce)
                .map_err(|e| format!("decrypt failed: {e}"))?;
            let inner = AgentEvent::decode(plaintext.as_slice())
                .map_err(|e| format!("decode failed: {e}"))?;
            Ok(inner)
        }
        _ => Err("rejected plaintext event: E2E encryption active".to_string()),
    }
}

/// Decrypt a batch of resume events. When crypto is active, each event is
/// expected to be an `Encrypted` envelope; events that fail decryption are
/// skipped (logged at warn level). When crypto is `None`, events pass through
/// unmodified (local mode).
fn decrypt_resume_events(
    crypto: Option<&std::sync::Arc<CryptoSession>>,
    events: Vec<AgentEvent>,
) -> Vec<AgentEvent> {
    let Some(session) = crypto else {
        return events;
    };
    events
        .into_iter()
        .filter_map(|event| match decrypt_agent_event(session, &event) {
            Ok(decrypted) => Some(decrypted),
            Err(e) => {
                warn!("Failed to decrypt resume event: {}", e);
                None
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::default_trait_access
)]
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
        let dir = std::env::temp_dir().join(format!("betcode-fp-test-{}", uuid::Uuid::new_v4()));
        let config = ConnectionConfig {
            fingerprint_store_path: Some(dir.join("known_daemons.json")),
            ..Default::default()
        };
        let conn = DaemonConnection::new(config);
        assert!(conn.fingerprint_store().daemons.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn exchange_keys_without_connection_returns_error() {
        let mut conn = DaemonConnection::new(ConnectionConfig::default());
        let result = conn.exchange_keys("m1").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ConnectionError::NotConnected => {}
            other => panic!("Expected NotConnected, got {other:?}"),
        }
    }

    // =========================================================================
    // Encrypt/decrypt helper tests
    // =========================================================================

    #[test]
    fn encrypt_agent_request_wraps_in_envelope() {
        let secret = [42u8; 32];
        let session = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let req = AgentRequest {
            request: Some(betcode_proto::v1::agent_request::Request::Message(
                betcode_proto::v1::UserMessage {
                    content: "hello".into(),
                    attachments: vec![],
                },
            )),
        };
        let encrypted = super::encrypt_agent_request(&session, &req).unwrap();
        match encrypted.request {
            Some(betcode_proto::v1::agent_request::Request::Encrypted(ref env)) => {
                assert!(!env.ciphertext.is_empty());
                assert_eq!(env.nonce.len(), 12);
            }
            other => panic!("Expected Encrypted variant, got {other:?}"),
        }
    }

    #[test]
    fn decrypt_agent_event_unwraps_envelope() {
        use prost::Message;
        let secret = [42u8; 32];
        let session1 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let session2 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();

        let original = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "hello".into(),
                    is_complete: false,
                },
            )),
        };
        let mut buf = Vec::with_capacity(original.encoded_len());
        original.encode(&mut buf).unwrap();
        let encrypted = session1.encrypt(&buf).unwrap();
        let wrapped = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: encrypted.ciphertext,
                    nonce: encrypted.nonce.to_vec(),
                },
            )),
        };
        let decrypted = super::decrypt_agent_event(&session2, &wrapped).unwrap();
        match decrypted.event {
            Some(betcode_proto::v1::agent_event::Event::TextDelta(ref td)) => {
                assert_eq!(td.text, "hello");
            }
            other => panic!("Expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip_agent_request() {
        use prost::Message;
        let secret = [99u8; 32];
        let session1 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let session2 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();

        let original = AgentRequest {
            request: Some(betcode_proto::v1::agent_request::Request::Start(
                betcode_proto::v1::StartConversation {
                    session_id: "s1".into(),
                    working_directory: "/tmp".into(),
                    model: "test".into(),
                    allowed_tools: vec![],
                    plan_mode: false,
                    worktree_id: String::new(),
                    metadata: Default::default(),
                },
            )),
        };
        let encrypted = super::encrypt_agent_request(&session1, &original).unwrap();
        // Extract envelope and decrypt
        match encrypted.request {
            Some(betcode_proto::v1::agent_request::Request::Encrypted(ref env)) => {
                let plaintext = session2.decrypt(&env.ciphertext, &env.nonce).unwrap();
                let decoded = AgentRequest::decode(plaintext.as_slice()).unwrap();
                match decoded.request {
                    Some(betcode_proto::v1::agent_request::Request::Start(ref s)) => {
                        assert_eq!(s.session_id, "s1");
                        assert_eq!(s.working_directory, "/tmp");
                        assert_eq!(s.model, "test");
                    }
                    other => panic!("Expected Start, got {other:?}"),
                }
            }
            other => panic!("Expected Encrypted, got {other:?}"),
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip_agent_event() {
        use prost::Message;
        let secret = [77u8; 32];
        let session1 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let session2 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();

        // Test several event types
        let events: Vec<AgentEvent> = vec![
            AgentEvent {
                sequence: 1,
                timestamp: None,
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                    betcode_proto::v1::TextDelta {
                        text: "hello".into(),
                        is_complete: false,
                    },
                )),
            },
            AgentEvent {
                sequence: 2,
                timestamp: None,
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::ToolCallStart(
                    betcode_proto::v1::ToolCallStart {
                        tool_id: "t1".into(),
                        tool_name: "Bash".into(),
                        input: None,
                        description: "ls".into(),
                    },
                )),
            },
            AgentEvent {
                sequence: 3,
                timestamp: None,
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::TurnComplete(
                    betcode_proto::v1::TurnComplete {
                        stop_reason: "end_turn".into(),
                    },
                )),
            },
            AgentEvent {
                sequence: 4,
                timestamp: None,
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::Error(
                    betcode_proto::v1::ErrorEvent {
                        code: "E01".into(),
                        message: "test error".into(),
                        is_fatal: false,
                        details: Default::default(),
                    },
                )),
            },
        ];

        for original in events {
            let mut buf = Vec::with_capacity(original.encoded_len());
            original.encode(&mut buf).unwrap();
            let enc = session1.encrypt(&buf).unwrap();
            let wrapped = AgentEvent {
                sequence: 0,
                timestamp: None,
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                    betcode_proto::v1::EncryptedEnvelope {
                        ciphertext: enc.ciphertext,
                        nonce: enc.nonce.to_vec(),
                    },
                )),
            };
            let decrypted = super::decrypt_agent_event(&session2, &wrapped).unwrap();
            assert_eq!(
                std::mem::discriminant(&decrypted.event),
                std::mem::discriminant(&original.event)
            );
        }
    }

    #[test]
    fn decrypt_non_encrypted_event_rejected_when_crypto_active() {
        let secret = [42u8; 32];
        let session = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let event = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "injected".into(),
                    is_complete: true,
                },
            )),
        };
        let result = super::decrypt_agent_event(&session, &event);
        assert!(
            result.is_err(),
            "plaintext event should be rejected when crypto is active"
        );
        assert!(result.unwrap_err().contains("rejected plaintext"));
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        use prost::Message;
        let secret1 = [11u8; 32];
        let secret2 = [22u8; 32];
        let session1 = betcode_crypto::CryptoSession::from_shared_secret(&secret1).unwrap();
        let session2 = betcode_crypto::CryptoSession::from_shared_secret(&secret2).unwrap();

        let event = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "secret".into(),
                    is_complete: false,
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let enc = session1.encrypt(&buf).unwrap();
        let wrapped = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce.to_vec(),
                },
            )),
        };
        let result = super::decrypt_agent_event(&session2, &wrapped);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_with_corrupted_ciphertext_fails() {
        use prost::Message;
        let secret = [42u8; 32];
        let session = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();

        let event = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "test".into(),
                    is_complete: false,
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let mut enc = session.encrypt(&buf).unwrap();
        // Corrupt the ciphertext
        if let Some(byte) = enc.ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
        let wrapped = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce.to_vec(),
                },
            )),
        };
        let result = super::decrypt_agent_event(&session, &wrapped);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_with_truncated_nonce_fails() {
        use prost::Message;
        let secret = [42u8; 32];
        let session = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();

        let event = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "test".into(),
                    is_complete: false,
                },
            )),
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let enc = session.encrypt(&buf).unwrap();
        let wrapped = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce[..8].to_vec(), // Truncated: should be 12
                },
            )),
        };
        let result = super::decrypt_agent_event(&session, &wrapped);
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_empty_request() {
        use prost::Message;
        let secret = [42u8; 32];
        let session1 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let session2 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();

        let original = AgentRequest { request: None };
        let encrypted = super::encrypt_agent_request(&session1, &original).unwrap();
        match encrypted.request {
            Some(betcode_proto::v1::agent_request::Request::Encrypted(ref env)) => {
                let plaintext = session2.decrypt(&env.ciphertext, &env.nonce).unwrap();
                let decoded = AgentRequest::decode(plaintext.as_slice()).unwrap();
                assert!(decoded.request.is_none());
            }
            other => panic!("Expected Encrypted, got {other:?}"),
        }
    }

    // =========================================================================
    // Empty/malformed EncryptedEnvelope tests
    // =========================================================================

    #[test]
    fn decrypt_event_with_empty_ciphertext_fails() {
        let secret = [42u8; 32];
        let session = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let event = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: vec![],
                    nonce: vec![0u8; 12],
                },
            )),
        };
        let result = super::decrypt_agent_event(&session, &event);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_event_with_empty_nonce_fails() {
        let secret = [42u8; 32];
        let session = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let event = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: vec![0xDE, 0xAD],
                    nonce: vec![],
                },
            )),
        };
        let result = super::decrypt_agent_event(&session, &event);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_event_with_both_empty_fails() {
        let secret = [42u8; 32];
        let session = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();
        let event = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: vec![],
                    nonce: vec![],
                },
            )),
        };
        let result = super::decrypt_agent_event(&session, &event);
        assert!(result.is_err());
    }

    // =========================================================================
    // Connection accessor tests
    // =========================================================================

    #[test]
    fn is_relay_returns_true_with_auth_and_machine() {
        let config = ConnectionConfig {
            auth_token: Some("tok".into()),
            machine_id: Some("m1".into()),
            ..Default::default()
        };
        assert!(config.is_relay());
    }

    #[test]
    fn is_relay_returns_false_without_auth() {
        let config = ConnectionConfig {
            auth_token: None,
            machine_id: Some("m1".into()),
            ..Default::default()
        };
        assert!(!config.is_relay());
    }

    #[test]
    fn machine_id_returns_configured_value() {
        let config = ConnectionConfig {
            machine_id: Some("my-machine".into()),
            ..Default::default()
        };
        assert_eq!(config.machine_id.as_deref(), Some("my-machine"));
    }

    #[tokio::test]
    async fn converse_relay_without_crypto_returns_error() {
        let dir = std::env::temp_dir().join(format!("betcode-relay-test-{}", uuid::Uuid::new_v4()));
        let fp_path = dir.join("known_daemons.json");
        let key_path = dir.join("identity.key");
        let config = ConnectionConfig {
            auth_token: Some("tok".into()),
            machine_id: Some("m1".into()),
            identity_key_path: Some(key_path),
            fingerprint_store_path: Some(fp_path),
            ..Default::default()
        };
        let mut conn = DaemonConnection::new(config);
        // Manually set connected state without actual gRPC client
        // to test the guard clause (which checks before touching the client)
        let result = conn.converse().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ConnectionError::KeyExchangeRequired => {}
            other => panic!("Expected KeyExchangeRequired, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn daemon_connection_is_relay_accessor() {
        let config = ConnectionConfig {
            auth_token: Some("tok".into()),
            machine_id: Some("m1".into()),
            ..Default::default()
        };
        let conn = DaemonConnection::new(config);
        assert!(conn.is_relay());
        assert_eq!(conn.machine_id(), Some("m1"));
    }

    #[test]
    fn daemon_connection_not_relay_accessor() {
        let conn = DaemonConnection::new(ConnectionConfig::default());
        assert!(!conn.is_relay());
        assert!(conn.machine_id().is_none());
    }

    // =========================================================================
    // TLS / CA cert tests
    // =========================================================================

    #[test]
    fn default_config_has_no_ca_cert() {
        let config = ConnectionConfig::default();
        assert!(config.ca_cert_path.is_none());
    }

    #[tokio::test]
    async fn connect_with_nonexistent_ca_cert_fails() {
        let config = ConnectionConfig {
            addr: "https://127.0.0.1:9999".into(),
            ca_cert_path: Some(std::path::PathBuf::from("/nonexistent/ca.pem")),
            ..Default::default()
        };
        let mut conn = DaemonConnection::new(config);
        let result = conn.connect().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ConnectionError::ConnectFailed(msg) => {
                assert!(
                    msg.contains("Failed to read CA cert"),
                    "Expected CA cert read error, got: {msg}",
                );
            }
            other => panic!("Expected ConnectFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn connect_with_valid_ca_cert_configures_tls() {
        // Write a dummy PEM (not a real cert, but enough to test TLS config path).
        // Connection will fail at the TLS handshake level, not at file read.
        let dir = std::env::temp_dir().join(format!("betcode-tls-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ca_path = dir.join("ca.pem");
        // Use a minimal but structurally valid PEM to pass tonic's Certificate::from_pem.
        // The actual connection will fail (no server), but we verify the code path.
        std::fs::write(
            &ca_path,
            "-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIUEjRVnJ1234=\n-----END CERTIFICATE-----\n",
        )
        .unwrap();

        let config = ConnectionConfig {
            addr: "https://127.0.0.1:9999".into(),
            ca_cert_path: Some(ca_path),
            ..Default::default()
        };
        let mut conn = DaemonConnection::new(config);
        let result = conn.connect().await;
        // Should fail with connection error (not file read error), proving TLS was configured
        assert!(result.is_err());
        match result.unwrap_err() {
            ConnectionError::ConnectFailed(msg) => {
                assert!(
                    !msg.contains("Failed to read CA cert"),
                    "Should not be a CA read error, got: {msg}",
                );
            }
            other => panic!("Expected ConnectFailed, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    // =========================================================================
    // Resume session decryption tests
    // =========================================================================

    #[test]
    fn decrypt_resume_events_decrypts_encrypted_events() {
        use prost::Message;
        let secret = [55u8; 32];
        let session = std::sync::Arc::new(
            betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap(),
        );
        let session2 = betcode_crypto::CryptoSession::from_shared_secret(&secret).unwrap();

        // Simulate what the daemon sends: plain events wrapped in Encrypted envelopes
        let original = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "Hello from history".into(),
                    is_complete: true,
                },
            )),
        };
        let mut buf = Vec::with_capacity(original.encoded_len());
        original.encode(&mut buf).unwrap();
        let enc = session2.encrypt(&buf).unwrap();
        let wrapped = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce.to_vec(),
                },
            )),
        };

        // This simulates what resume_session should do: decrypt when crypto is active
        let decrypted = decrypt_resume_events(Some(&session), vec![wrapped]);
        assert_eq!(decrypted.len(), 1);
        match &decrypted[0].event {
            Some(betcode_proto::v1::agent_event::Event::TextDelta(td)) => {
                assert_eq!(td.text, "Hello from history");
                assert!(td.is_complete);
            }
            other => panic!("Expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn decrypt_resume_events_passes_through_without_crypto() {
        let event = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "plain".into(),
                    is_complete: false,
                },
            )),
        };

        let result = decrypt_resume_events(None, vec![event]);
        assert_eq!(result.len(), 1);
        match &result[0].event {
            Some(betcode_proto::v1::agent_event::Event::TextDelta(td)) => {
                assert_eq!(td.text, "plain");
            }
            other => panic!("Expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn decrypt_resume_events_skips_undecryptable() {
        use prost::Message;
        let secret1 = [11u8; 32];
        let secret2 = [22u8; 32];
        let session = std::sync::Arc::new(
            betcode_crypto::CryptoSession::from_shared_secret(&secret1).unwrap(),
        );
        let wrong_session = betcode_crypto::CryptoSession::from_shared_secret(&secret2).unwrap();

        // Encrypt with wrong key
        let original = AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "secret".into(),
                    is_complete: false,
                },
            )),
        };
        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();
        let enc = wrong_session.encrypt(&buf).unwrap();
        let wrapped = AgentEvent {
            sequence: 0,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                betcode_proto::v1::EncryptedEnvelope {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce.to_vec(),
                },
            )),
        };

        // Should skip the undecryptable event rather than crash
        let result = decrypt_resume_events(Some(&session), vec![wrapped]);
        assert_eq!(result.len(), 0);
    }
}
