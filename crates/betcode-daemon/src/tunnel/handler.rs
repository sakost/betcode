//! Tunnel request handler that routes incoming frames to local services.

use std::collections::HashMap;
use std::sync::Arc;

use prost::Message;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use tokio_stream::StreamExt as _;
use tonic::Request;

use betcode_proto::v1::command_service_server::CommandService as CommandServiceTrait;
use betcode_proto::v1::config_service_server::ConfigService as ConfigServiceTrait;
use betcode_proto::v1::git_lab_service_server::GitLabService as GitLabServiceTrait;
use betcode_proto::v1::git_repo_service_server::GitRepoService as GitRepoServiceTrait;
use betcode_proto::v1::worktree_service_server::WorktreeService as WorktreeServiceTrait;
use betcode_proto::v1::{
    AddPluginRequest, AgentRequest, CancelTurnRequest, CancelTurnResponse,
    ClearSessionGrantsRequest, ClearSessionGrantsResponse, CompactSessionRequest,
    CompactSessionResponse, CreateWorktreeRequest, DisablePluginRequest, EnablePluginRequest,
    EncryptedPayload, ExecuteServiceCommandRequest, FrameType, GetCommandRegistryRequest,
    GetIssueRequest, GetMergeRequestRequest, GetPermissionsRequest, GetPipelineRequest,
    GetPluginStatusRequest, GetRepoRequest, GetSettingsRequest, GetWorktreeRequest,
    InputLockRequest, InputLockResponse, KeyExchangeRequest, KeyExchangeResponse,
    ListAgentsRequest, ListIssuesRequest, ListMcpServersRequest, ListMergeRequestsRequest,
    ListPathRequest, ListPipelinesRequest, ListPluginsRequest, ListReposRequest,
    ListSessionGrantsRequest, ListSessionGrantsResponse, ListSessionsRequest,
    ListSessionsResponse, ListWorktreesRequest, RegisterRepoRequest, RemovePluginRequest,
    RemoveWorktreeRequest, RenameSessionRequest, RenameSessionResponse, ResumeSessionRequest,
    ScanReposRequest, SessionGrantEntry, SessionSummary, SetSessionGrantRequest,
    SetSessionGrantResponse, StreamPayload, TunnelError, TunnelErrorCode, TunnelFrame,
    UnregisterRepoRequest, UpdateRepoRequest, UpdateSettingsRequest,
};

use betcode_crypto::{CryptoSession, IdentityKeyPair, KeyExchangeState};

use crate::relay::{is_granted, SessionRelay};
use crate::server::{
    CommandServiceImpl, ConfigServiceImpl, GitLabServiceImpl, GitRepoServiceImpl,
    WorktreeServiceImpl,
};
use crate::session::SessionMultiplexer;
use crate::storage::Database;

// Re-export method constants from betcode-proto so that tests (which use `use super::*`)
// and any other in-crate consumers continue to see them at the same path.
pub use betcode_proto::methods::{
    METHOD_ADD_PLUGIN, METHOD_CANCEL_TURN, METHOD_CLEAR_SESSION_GRANTS, METHOD_COMPACT_SESSION,
    METHOD_CONVERSE, METHOD_CREATE_WORKTREE, METHOD_DISABLE_PLUGIN, METHOD_ENABLE_PLUGIN,
    METHOD_EXCHANGE_KEYS, METHOD_EXECUTE_SERVICE_COMMAND, METHOD_GET_COMMAND_REGISTRY,
    METHOD_GET_ISSUE, METHOD_GET_MERGE_REQUEST, METHOD_GET_PERMISSIONS, METHOD_GET_PIPELINE,
    METHOD_GET_PLUGIN_STATUS, METHOD_GET_REPO, METHOD_GET_SETTINGS, METHOD_GET_WORKTREE,
    METHOD_LIST_AGENTS, METHOD_LIST_ISSUES, METHOD_LIST_MCP_SERVERS, METHOD_LIST_MERGE_REQUESTS,
    METHOD_LIST_PATH, METHOD_LIST_PIPELINES, METHOD_LIST_PLUGINS, METHOD_LIST_REPOS,
    METHOD_LIST_SESSIONS, METHOD_LIST_SESSION_GRANTS, METHOD_LIST_WORKTREES,
    METHOD_REGISTER_REPO, METHOD_REMOVE_PLUGIN, METHOD_REMOVE_WORKTREE, METHOD_RENAME_SESSION,
    METHOD_REQUEST_INPUT_LOCK, METHOD_RESUME_SESSION, METHOD_SCAN_REPOS,
    METHOD_SET_SESSION_GRANT, METHOD_UNREGISTER_REPO, METHOD_UPDATE_REPO, METHOD_UPDATE_SETTINGS,
};

/// Default maximum number of sessions returned by `ListSessions`.
const DEFAULT_LIST_SESSIONS_LIMIT: u32 = 50;

/// Minimum number of messages to keep during session compaction.
const MIN_COMPACTION_KEEP: u32 = 10;

/// Estimated tokens saved per deleted message during compaction.
const ESTIMATED_TOKENS_PER_MESSAGE: u32 = 100;

/// Expected X25519 public key length in bytes.
const X25519_PUBKEY_LEN: usize = 32;

/// Prefix for tunnel client IDs.
const TUNNEL_CLIENT_ID_PREFIX: &str = "tunnel";

/// Generate a unique client ID for tunnel connections.
fn generate_tunnel_client_id() -> String {
    format!("{}-{}", TUNNEL_CLIENT_ID_PREFIX, uuid::Uuid::new_v4())
}

/// Info about an active streaming session routed through the tunnel.
struct ActiveStream {
    session_id: String,
    /// Deferred session config — subprocess is only started on first `UserMessage`.
    pending_config: Option<crate::relay::RelaySessionConfig>,
}

/// Dispatch a unary gRPC call through the tunnel.
///
/// Decodes `$data` into `$req_ty`, calls `$svc.$method(...)`, and wraps the
/// response (or error) into a `Vec<TunnelFrame>`.
macro_rules! dispatch_rpc {
    ($self:ident, $svc:expr, $request_id:expr, $data:expr, $relay_forwarded:expr, $req_ty:ty, $method:ident) => {{
        let req = match <$req_ty>::decode($data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    $request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        match $svc.$method(Request::new(req)).await {
            Ok(resp) => vec![
                $self
                    .unary_response_frame($request_id, &resp.into_inner(), $relay_forwarded)
                    .await,
            ],
            Err(status) => vec![Self::error_response(
                $request_id,
                TunnelErrorCode::Internal,
                status.message(),
            )],
        }
    }};
}

/// Handles incoming tunnel frames by dispatching to local gRPC services.
pub struct TunnelRequestHandler {
    machine_id: String,
    relay: Arc<SessionRelay>,
    multiplexer: Arc<SessionMultiplexer>,
    db: Database,
    /// Sender for pushing response frames back through the tunnel.
    outbound_tx: mpsc::Sender<TunnelFrame>,
    /// Active streaming sessions keyed by `request_id`.
    active_streams: Arc<RwLock<HashMap<String, ActiveStream>>>,
    /// E2E crypto session for encrypt/decrypt. None = passthrough (no encryption).
    crypto: Arc<RwLock<Option<Arc<CryptoSession>>>>,
    /// Identity keypair for key exchange. None = key exchange disabled.
    identity: Option<Arc<IdentityKeyPair>>,
    /// `CommandService` implementation for handling command-related RPCs through the tunnel.
    command_service: Option<Arc<CommandServiceImpl>>,
    /// `GitLabService` implementation for handling GitLab-related RPCs through the tunnel.
    gitlab_service: Option<Arc<GitLabServiceImpl>>,
    /// `GitRepoService` implementation for handling repo RPCs through the tunnel.
    repo_service: Option<Arc<GitRepoServiceImpl>>,
    /// `WorktreeService` implementation for handling worktree-related RPCs through the tunnel.
    worktree_service: Option<Arc<WorktreeServiceImpl>>,
    /// `ConfigService` implementation for handling config RPCs through the tunnel.
    config_service: Option<Arc<ConfigServiceImpl>>,
}

impl TunnelRequestHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        machine_id: String,
        relay: Arc<SessionRelay>,
        multiplexer: Arc<SessionMultiplexer>,
        db: Database,
        outbound_tx: mpsc::Sender<TunnelFrame>,
        crypto: Option<Arc<CryptoSession>>,
        identity: Option<Arc<IdentityKeyPair>>,
    ) -> Self {
        Self {
            machine_id,
            relay,
            multiplexer,
            db,
            outbound_tx,
            active_streams: Arc::new(RwLock::new(HashMap::new())),
            crypto: Arc::new(RwLock::new(crypto)),
            identity,
            command_service: None,
            gitlab_service: None,
            repo_service: None,
            worktree_service: None,
            config_service: None,
        }
    }

    /// Set the `CommandService` implementation for handling command RPCs through the tunnel.
    pub fn set_command_service(&mut self, service: Arc<CommandServiceImpl>) {
        self.command_service = Some(service);
    }

    /// Set the `GitLabService` implementation for handling GitLab RPCs through the tunnel.
    pub fn set_gitlab_service(&mut self, service: Arc<GitLabServiceImpl>) {
        self.gitlab_service = Some(service);
    }

    /// Set the `GitRepoService` implementation for handling repo RPCs through the tunnel.
    pub fn set_repo_service(&mut self, service: Arc<GitRepoServiceImpl>) {
        self.repo_service = Some(service);
    }

    /// Set the `WorktreeService` implementation for handling worktree RPCs through the tunnel.
    pub fn set_worktree_service(&mut self, service: Arc<WorktreeServiceImpl>) {
        self.worktree_service = Some(service);
    }

    /// Set the `ConfigService` implementation for handling config RPCs through the tunnel.
    pub fn set_config_service(&mut self, svc: Arc<ConfigServiceImpl>) {
        self.config_service = Some(svc);
    }

    /// Process an incoming frame and produce zero or more response frames.
    pub async fn handle_frame(&self, frame: TunnelFrame) -> Vec<TunnelFrame> {
        let request_id = frame.request_id.clone();
        match FrameType::try_from(frame.frame_type) {
            Ok(FrameType::Request) => self.handle_request(request_id, frame).await,
            Ok(FrameType::Control) => {
                debug!(request_id = %request_id, "Received control frame");
                vec![]
            }
            Ok(FrameType::Error) => {
                warn!(request_id = %request_id, "Received error frame from relay");
                vec![]
            }
            Ok(FrameType::StreamData) => {
                if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(ref p)) =
                    frame.payload
                {
                    let data = match p.encrypted.as_ref() {
                        Some(enc) => match self.decrypt_payload(enc).await {
                            Ok(d) => d,
                            Err(e) => {
                                warn!(request_id = %request_id, error = %e, "StreamData decryption failed");
                                return vec![];
                            }
                        },
                        None => Vec::new(),
                    };
                    self.handle_incoming_stream_data(&request_id, &data).await;
                } else {
                    warn!(request_id = %request_id, "StreamData frame missing StreamPayload");
                }
                vec![]
            }
            Ok(frame_type) => {
                warn!(request_id = %request_id, ?frame_type, "Unexpected frame type");
                vec![Self::error_response(
                    &request_id,
                    TunnelErrorCode::Internal,
                    &format!("Unexpected frame type: {frame_type:?}"),
                )]
            }
            Err(_) => {
                error!(request_id = %request_id, "Unknown frame type");
                vec![Self::error_response(
                    &request_id,
                    TunnelErrorCode::Internal,
                    "Unknown frame type",
                )]
            }
        }
    }

    /// Decrypt an `EncryptedPayload` using the session key, or passthrough if no crypto.
    ///
    /// When the nonce is empty, the payload is treated as plaintext passthrough.
    /// This happens when the relay forwards data without tunnel-layer encryption
    /// (the relay doesn't have the crypto keys). App-layer encryption
    /// (`EncryptedEnvelope`) handles the actual E2E protection.
    ///
    /// Clones the `Arc<CryptoSession>` and drops the read lock immediately so
    /// the actual decryption (CPU-bound) does not hold the lock.
    async fn decrypt_payload(&self, enc: &EncryptedPayload) -> Result<Vec<u8>, String> {
        // Empty nonce = relay passthrough (relay doesn't tunnel-encrypt).
        // App-layer EncryptedEnvelope handles E2E protection.
        if enc.nonce.is_empty() {
            debug!("Tunnel-layer passthrough (empty nonce) — relay-forwarded data");
            return Ok(enc.ciphertext.clone());
        }
        let crypto = self.crypto.read().await.clone();
        crypto.map_or_else(
            || {
                debug!("Tunnel-layer passthrough (no crypto session) — app-layer handles E2E");
                Ok(enc.ciphertext.clone())
            },
            |crypto| {
                crypto
                    .decrypt(&enc.ciphertext, &enc.nonce)
                    .map_err(|e| format!("decryption failed: {e}"))
            },
        )
    }

    /// Encrypt data into an `EncryptedPayload` using the session key, or passthrough.
    ///
    /// Clones the `Arc<CryptoSession>` and drops the read lock immediately.
    async fn encrypt_payload(&self, data: &[u8]) -> Result<EncryptedPayload, String> {
        let crypto = self.crypto.read().await.clone();
        if crypto.is_none() {
            debug!("Tunnel-layer passthrough (no crypto session) — app-layer handles E2E");
        }
        make_encrypted_payload(crypto.as_deref(), data)
    }

    /// Get a snapshot of the current crypto session for use in spawned tasks.
    async fn crypto_snapshot(&self) -> Option<Arc<CryptoSession>> {
        self.crypto.read().await.clone()
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_request(&self, request_id: String, frame: TunnelFrame) -> Vec<TunnelFrame> {
        let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(payload)) = frame.payload
        else {
            return vec![Self::error_response(
                &request_id,
                TunnelErrorCode::Internal,
                "Missing StreamPayload",
            )];
        };

        info!(request_id = %request_id, method = %payload.method, machine_id = %self.machine_id, "Handling tunneled request");

        // ExchangeKeys is handled before decryption (no session key yet).
        if payload.method == METHOD_EXCHANGE_KEYS {
            let data = payload
                .encrypted
                .as_ref()
                .map(|e| e.ciphertext.clone())
                .unwrap_or_default();
            return self.handle_exchange_keys(&request_id, &data).await;
        }

        // Detect relay-forwarded requests (no tunnel-layer encryption).
        // When the relay forwards data, the nonce is empty because the relay
        // doesn't have crypto keys. Responses to relay-forwarded requests must
        // also skip tunnel-layer encryption so the relay can decode the protobuf.
        let relay_forwarded = payload
            .encrypted
            .as_ref()
            .is_none_or(|e| e.nonce.is_empty());

        let data = match payload.encrypted.as_ref() {
            Some(enc) => match self.decrypt_payload(enc).await {
                Ok(d) => d,
                Err(e) => {
                    return vec![Self::error_response(
                        &request_id,
                        TunnelErrorCode::Internal,
                        &e,
                    )]
                }
            },
            None => Vec::new(),
        };

        match payload.method.as_str() {
            METHOD_LIST_SESSIONS => {
                self.handle_list_sessions(&request_id, &data, relay_forwarded)
                    .await
            }
            METHOD_COMPACT_SESSION => {
                self.handle_compact_session(&request_id, &data, relay_forwarded)
                    .await
            }
            METHOD_CANCEL_TURN => {
                self.handle_cancel_turn(&request_id, &data, relay_forwarded)
                    .await
            }
            METHOD_REQUEST_INPUT_LOCK => {
                self.handle_request_input_lock(&request_id, &data, relay_forwarded)
                    .await
            }
            METHOD_CONVERSE => {
                self.handle_converse(&request_id, &data, relay_forwarded)
                    .await;
                vec![] // Responses sent asynchronously via outbound_tx
            }
            METHOD_RESUME_SESSION => {
                self.handle_resume_session(&request_id, &data, relay_forwarded)
                    .await
            }
            // CommandService RPCs
            METHOD_GET_COMMAND_REGISTRY
            | METHOD_LIST_AGENTS
            | METHOD_LIST_PATH
            | METHOD_LIST_PLUGINS
            | METHOD_GET_PLUGIN_STATUS
            | METHOD_ADD_PLUGIN
            | METHOD_REMOVE_PLUGIN
            | METHOD_ENABLE_PLUGIN
            | METHOD_DISABLE_PLUGIN => {
                self.dispatch_command_rpc(
                    &request_id,
                    payload.method.as_str(),
                    &data,
                    relay_forwarded,
                )
                .await
            }
            METHOD_EXECUTE_SERVICE_COMMAND => {
                self.handle_execute_service_command(&request_id, &data, relay_forwarded)
                    .await;
                vec![] // Responses sent asynchronously via outbound_tx
            }
            // GitLabService RPCs
            METHOD_LIST_MERGE_REQUESTS
            | METHOD_GET_MERGE_REQUEST
            | METHOD_LIST_PIPELINES
            | METHOD_GET_PIPELINE
            | METHOD_LIST_ISSUES
            | METHOD_GET_ISSUE => {
                self.dispatch_gitlab_rpc(
                    &request_id,
                    payload.method.as_str(),
                    &data,
                    relay_forwarded,
                )
                .await
            }
            // GitRepoService RPCs
            METHOD_REGISTER_REPO
            | METHOD_UNREGISTER_REPO
            | METHOD_LIST_REPOS
            | METHOD_GET_REPO
            | METHOD_UPDATE_REPO
            | METHOD_SCAN_REPOS => {
                self.dispatch_repo_rpc(
                    &request_id,
                    payload.method.as_str(),
                    &data,
                    relay_forwarded,
                )
                .await
            }
            // WorktreeService RPCs
            METHOD_CREATE_WORKTREE
            | METHOD_REMOVE_WORKTREE
            | METHOD_LIST_WORKTREES
            | METHOD_GET_WORKTREE => {
                self.dispatch_worktree_rpc(
                    &request_id,
                    payload.method.as_str(),
                    &data,
                    relay_forwarded,
                )
                .await
            }
            // ConfigService RPCs
            METHOD_GET_SETTINGS
            | METHOD_UPDATE_SETTINGS
            | METHOD_LIST_MCP_SERVERS
            | METHOD_GET_PERMISSIONS => {
                self.dispatch_config_rpc(
                    &request_id,
                    payload.method.as_str(),
                    &data,
                    relay_forwarded,
                )
                .await
            }
            // Session grant management RPCs
            METHOD_LIST_SESSION_GRANTS => {
                self.handle_list_session_grants(&request_id, &data, relay_forwarded)
                    .await
            }
            METHOD_CLEAR_SESSION_GRANTS => {
                self.handle_clear_session_grants(&request_id, &data, relay_forwarded)
                    .await
            }
            METHOD_SET_SESSION_GRANT => {
                self.handle_set_session_grant(&request_id, &data, relay_forwarded)
                    .await
            }
            // Session rename RPC
            METHOD_RENAME_SESSION => {
                self.handle_rename_session(&request_id, &data, relay_forwarded)
                    .await
            }
            other => vec![Self::error_response(
                &request_id,
                TunnelErrorCode::NotFound,
                &format!("Unknown method: {other}"),
            )],
        }
    }

    async fn handle_list_sessions(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match ListSessionsRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        let working_dir = if req.working_directory.is_empty() {
            None
        } else {
            Some(req.working_directory.as_str())
        };
        let limit = if req.limit == 0 {
            DEFAULT_LIST_SESSIONS_LIMIT
        } else {
            req.limit
        };

        match self.db.list_sessions(working_dir, limit, req.offset).await {
            Ok(sessions) => {
                let summaries: Vec<SessionSummary> =
                    sessions.into_iter().map(SessionSummary::from).collect();
                #[allow(clippy::cast_possible_truncation)]
                let total = summaries.len() as u32;
                vec![
                    self.unary_response_frame(
                        request_id,
                        &ListSessionsResponse {
                            sessions: summaries,
                            total,
                        },
                        relay_forwarded,
                    )
                    .await,
                ]
            }
            Err(e) => vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                &format!("ListSessions failed: {e}"),
            )],
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_compact_session(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match CompactSessionRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        let sid = &req.session_id;
        let messages_before = match self.db.count_messages(sid).await {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok(c) => c as u32,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &e.to_string(),
                )]
            }
        };
        if messages_before == 0 {
            return vec![
                self.unary_response_frame(
                    request_id,
                    &CompactSessionResponse {
                        messages_before: 0,
                        messages_after: 0,
                        tokens_saved: 0,
                    },
                    relay_forwarded,
                )
                .await,
            ];
        }
        let max_seq = match self.db.max_message_sequence(sid).await {
            Ok(s) => s,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &e.to_string(),
                )]
            }
        };
        let keep_count = (messages_before / 2)
            .max(MIN_COMPACTION_KEEP)
            .min(messages_before);
        let cutoff = max_seq - i64::from(keep_count);
        if cutoff <= 0 {
            return vec![
                self.unary_response_frame(
                    request_id,
                    &CompactSessionResponse {
                        messages_before,
                        messages_after: messages_before,
                        tokens_saved: 0,
                    },
                    relay_forwarded,
                )
                .await,
            ];
        }
        let deleted = match self.db.delete_messages_before_sequence(sid, cutoff).await {
            Ok(d) => d,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &e.to_string(),
                )]
            }
        };
        if let Err(e) = self.db.update_compaction_sequence(sid, cutoff).await {
            warn!(session_id = %sid, error = %e, "Failed to update compaction sequence");
        }
        #[allow(clippy::cast_possible_truncation)]
        let messages_after = messages_before - deleted as u32;
        #[allow(clippy::cast_possible_truncation)]
        let tokens_saved = deleted as u32 * ESTIMATED_TOKENS_PER_MESSAGE;
        info!(session_id = %sid, messages_before, messages_after, "Session compacted via tunnel");
        vec![
            self.unary_response_frame(
                request_id,
                &CompactSessionResponse {
                    messages_before,
                    messages_after,
                    tokens_saved,
                },
                relay_forwarded,
            )
            .await,
        ]
    }

    async fn handle_cancel_turn(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match CancelTurnRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        let was_active = self
            .relay
            .cancel_session(&req.session_id)
            .await
            .unwrap_or(false);
        vec![
            self.unary_response_frame(
                request_id,
                &CancelTurnResponse { was_active },
                relay_forwarded,
            )
            .await,
        ]
    }

    async fn handle_request_input_lock(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match InputLockRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        let client_id = generate_tunnel_client_id();
        match self
            .db
            .acquire_input_lock(&req.session_id, &client_id)
            .await
        {
            Ok(previous) => vec![
                self.unary_response_frame(
                    request_id,
                    &InputLockResponse {
                        granted: true,
                        previous_holder: previous.unwrap_or_default(),
                    },
                    relay_forwarded,
                )
                .await,
            ],
            Err(e) => vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                &format!("InputLock failed: {e}"),
            )],
        }
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn handle_list_session_grants(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match ListSessionGrantsRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        let Some(handle) = self.relay.get_handle(&req.session_id).await else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Session {} not active", req.session_id),
            )];
        };
        let grants = handle.session_grants.read().await;
        let entries: Vec<SessionGrantEntry> = grants
            .iter()
            .map(|(tool_name, granted)| SessionGrantEntry {
                tool_name: tool_name.clone(),
                granted: *granted,
            })
            .collect();
        drop(grants);
        vec![
            self.unary_response_frame(
                request_id,
                &ListSessionGrantsResponse { grants: entries },
                relay_forwarded,
            )
            .await,
        ]
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn handle_clear_session_grants(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match ClearSessionGrantsRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        let Some(handle) = self.relay.get_handle(&req.session_id).await else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Session {} not active", req.session_id),
            )];
        };
        let mut grants = handle.session_grants.write().await;
        if req.tool_name.is_empty() {
            grants.clear();
            drop(grants);
            info!(session_id = %req.session_id, "Cleared all session grants via tunnel");
        } else {
            grants.remove(&req.tool_name);
            drop(grants);
            info!(session_id = %req.session_id, tool_name = %req.tool_name, "Cleared session grant via tunnel");
        }
        vec![
            self.unary_response_frame(
                request_id,
                &ClearSessionGrantsResponse {},
                relay_forwarded,
            )
            .await,
        ]
    }

    async fn handle_set_session_grant(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match SetSessionGrantRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        if req.tool_name.is_empty() {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::InvalidArgument,
                "tool_name must not be empty",
            )];
        }
        let Some(handle) = self.relay.get_handle(&req.session_id).await else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Session {} not active", req.session_id),
            )];
        };
        let tool_name = req.tool_name.clone();
        let granted = req.granted;
        handle
            .session_grants
            .write()
            .await
            .insert(req.tool_name, req.granted);
        info!(session_id = %req.session_id, tool_name = %tool_name, granted = granted, "Set session grant via tunnel");
        vec![
            self.unary_response_frame(
                request_id,
                &SetSessionGrantResponse {},
                relay_forwarded,
            )
            .await,
        ]
    }

    async fn handle_rename_session(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match RenameSessionRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };
        match self
            .db
            .update_session_name(&req.session_id, &req.name)
            .await
        {
            Ok(()) => {
                info!(session_id = %req.session_id, name = %req.name, "Session renamed via tunnel");
                vec![
                    self.unary_response_frame(
                        request_id,
                        &RenameSessionResponse {},
                        relay_forwarded,
                    )
                    .await,
                ]
            }
            Err(crate::storage::DatabaseError::NotFound(_)) => vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Session {} not found", req.session_id),
            )],
            Err(e) => vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                &format!("RenameSession failed: {e}"),
            )],
        }
    }

    /// Handle an incoming `StreamData` frame for an active streaming session.
    /// Routes user messages, permissions, etc. to the relay.
    ///
    /// On the first `UserMessage`, if the subprocess hasn't been started yet
    /// (`pending_config` is Some), starts it and immediately sends the message.
    ///
    /// If E2E crypto is active, incoming `AgentRequest` messages must use the
    /// `Encrypted` oneof variant containing an `EncryptedEnvelope`. The envelope
    /// is decrypted and re-decoded as the real `AgentRequest`. Plaintext requests
    /// are rejected when crypto is active (prevents downgrade attacks).
    #[allow(clippy::too_many_lines, clippy::items_after_statements)]
    pub async fn handle_incoming_stream_data(&self, request_id: &str, data: &[u8]) {
        let sid = {
            let stream = self.active_streams.read().await;
            if let Some(a) = stream.get(request_id) { a.session_id.clone() } else {
                warn!(request_id = %request_id, "StreamData for unknown active stream");
                return;
            }
        };

        let outer_req = match AgentRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                warn!(request_id = %request_id, error = %e, "Failed to decode AgentRequest");
                return;
            }
        };

        // Application-layer E2E decryption
        let crypto = self.crypto.read().await.clone();
        let req = match (&crypto, &outer_req.request) {
            // Encrypted request with active crypto → decrypt
            (Some(session), Some(betcode_proto::v1::agent_request::Request::Encrypted(env))) => {
                match session.decrypt(&env.ciphertext, &env.nonce) {
                    Ok(plaintext) => match AgentRequest::decode(plaintext.as_slice()) {
                        Ok(inner) => inner,
                        Err(e) => {
                            warn!(request_id = %request_id, error = %e, "Failed to decode decrypted AgentRequest");
                            return;
                        }
                    },
                    Err(e) => {
                        warn!(request_id = %request_id, error = %e, "Failed to decrypt EncryptedEnvelope in AgentRequest");
                        return;
                    }
                }
            }
            // Plaintext request with active crypto → reject (downgrade attack prevention)
            (Some(_), _) => {
                warn!(request_id = %request_id, "Rejected plaintext AgentRequest when E2E crypto is active");
                return;
            }
            // No crypto, not encrypted → passthrough (local mode)
            (None, _) => outer_req,
        };

        // Check if we need to start the subprocess (deferred from handle_converse).
        // Take the config under a write lock so only the first message triggers start.
        let pending = {
            let mut streams = self.active_streams.write().await;
            streams
                .get_mut(request_id)
                .and_then(|a| a.pending_config.take())
        };
        if let Some(config) = pending {
            debug!(
                request_id = %request_id,
                session_id = %sid,
                "Starting deferred subprocess on first user input"
            );
            if let Err(e) = self.relay.start_session(config).await {
                error!(
                    request_id = %request_id,
                    session_id = %sid,
                    error = %e,
                    "Deferred start_session failed"
                );
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("Start session failed: {e}"),
                    ))
                    .await;
                self.active_streams.write().await.remove(request_id);
                return;
            }
        }

        use betcode_proto::v1::agent_request::Request;
        match req.request {
            Some(Request::Message(msg)) => {
                let agent_id = if msg.agent_id.is_empty() {
                    None
                } else {
                    Some(msg.agent_id.as_str())
                };
                if let Err(e) = self
                    .relay
                    .send_user_message(&sid, &msg.content, agent_id)
                    .await
                {
                    warn!(session_id = %sid, error = %e, "Failed to send user message via tunnel");
                }
            }
            Some(Request::Permission(perm)) => {
                let decision =
                    betcode_proto::v1::PermissionDecision::try_from(perm.decision)
                        .unwrap_or_else(|_| {
                            warn!(raw_decision = perm.decision, "Unknown PermissionDecision, defaulting to Deny");
                            betcode_proto::v1::PermissionDecision::Deny
                        });

                let (granted, original_input) =
                    if let Some(handle) = self.relay.get_handle(&sid).await {
                        handle
                            .process_permission_response(&perm.request_id, decision, "tunnel")
                            .await
                    } else {
                        (
                            is_granted(decision),
                            serde_json::json!({}),
                        )
                    };

                info!(session_id = %sid, request_id = %perm.request_id, granted, ?decision, "Permission via tunnel");

                if let Err(e) = self
                    .relay
                    .send_permission_response(&sid, &perm.request_id, granted, &original_input)
                    .await
                {
                    warn!(session_id = %sid, error = %e, "Failed to send permission via tunnel");
                }
            }
            Some(Request::QuestionResponse(qr)) => {
                info!(session_id = %sid, question_id = %qr.question_id, "Question response via tunnel");
                let answers: std::collections::HashMap<String, String> =
                    qr.answers.into_iter().collect();

                // Look up original AskUserQuestion input from pending map.
                let original_input = if let Some(handle) = self.relay.get_handle(&sid).await {
                    handle
                        .pending_question_inputs
                        .write()
                        .await
                        .remove(&qr.question_id)
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    serde_json::json!({})
                };

                if let Err(e) = self
                    .relay
                    .send_question_response(&sid, &qr.question_id, &answers, &original_input)
                    .await
                {
                    warn!(session_id = %sid, error = %e, "Failed to send question response via tunnel");
                }
            }
            Some(Request::Cancel(_)) => {
                if let Err(e) = self.relay.cancel_session(&sid).await {
                    warn!(session_id = %sid, error = %e, "Failed to cancel session");
                }
            }
            other => {
                warn!(request_id = %request_id, request_type = ?other.map(|_| "unknown"), "Ignoring non-actionable StreamData request");
            }
        }
    }

    /// Check if a `request_id` has an active streaming session.
    pub async fn has_active_stream(&self, request_id: &str) -> bool {
        self.active_streams.read().await.contains_key(request_id)
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_converse(&self, request_id: &str, data: &[u8], relay_forwarded: bool) {
        let outer_req = match AgentRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("Decode error: {e}"),
                    ))
                    .await;
                return;
            }
        };

        // Application-layer E2E decryption (same logic as handle_incoming_stream_data)
        let crypto = self.crypto.read().await.clone();
        let start = match (&crypto, &outer_req.request) {
            (Some(session), Some(betcode_proto::v1::agent_request::Request::Encrypted(env))) => {
                match session.decrypt(&env.ciphertext, &env.nonce) {
                    Ok(plaintext) => match AgentRequest::decode(plaintext.as_slice()) {
                        Ok(inner) => inner,
                        Err(e) => {
                            warn!(request_id = %request_id, error = %e, "Failed to decode decrypted StartConversation");
                            let _ = self
                                .outbound_tx
                                .send(Self::error_response(
                                    request_id,
                                    TunnelErrorCode::Internal,
                                    "Failed to decode decrypted StartConversation",
                                ))
                                .await;
                            return;
                        }
                    },
                    Err(e) => {
                        warn!(request_id = %request_id, error = %e, "Failed to decrypt StartConversation envelope");
                        let _ = self
                            .outbound_tx
                            .send(Self::error_response(
                                request_id,
                                TunnelErrorCode::Internal,
                                "Failed to decrypt StartConversation",
                            ))
                            .await;
                        return;
                    }
                }
            }
            (Some(_), _) => {
                warn!(request_id = %request_id, "Rejected plaintext StartConversation when E2E crypto is active");
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        "E2E encryption required",
                    ))
                    .await;
                return;
            }
            (None, _) => outer_req,
        };

        let Some(betcode_proto::v1::agent_request::Request::Start(start_conv)) = start.request
        else {
            let _ = self
                .outbound_tx
                .send(Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    "First Converse message must be StartConversation",
                ))
                .await;
            return;
        };

        let sid = start_conv.session_id.clone();
        // Only pass a model to the subprocess if the client explicitly requests one.
        // When None, Claude Code uses its own default based on the user's API key.
        let model = if start_conv.model.is_empty() {
            None
        } else {
            Some(start_conv.model.clone())
        };
        let working_dir: std::path::PathBuf = start_conv.working_directory.clone().into();

        // Create or resume session in DB
        let resume_session = if let Ok(existing) = self.db.get_session(&sid).await { existing.claude_session_id.filter(|s| !s.is_empty()) } else {
            if let Err(e) = self
                .db
                .create_session(
                    &sid,
                    model.as_deref().unwrap_or("default"),
                    &start_conv.working_directory,
                )
                .await
            {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("Create session failed: {e}"),
                    ))
                    .await;
                return;
            }
            None
        };

        if let Err(e) = self
            .db
            .update_session_status(&sid, crate::storage::SessionStatus::Active)
            .await
        {
            warn!(session_id = %sid, error = %e, "Failed to update session status to Active");
        }

        let client_id = generate_tunnel_client_id();
        let handle = match self.multiplexer.subscribe(&sid, &client_id, "tunnel").await {
            Ok(h) => h,
            Err(e) => {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("Subscribe failed: {e}"),
                    ))
                    .await;
                return;
            }
        };

        // Build session config but defer subprocess start until first UserMessage.
        // Without deferral the subprocess starts with no prompt, times out waiting
        // for stdin, and exits before the user types anything in TUI mode.
        let config = crate::relay::RelaySessionConfig {
            session_id: sid.clone(),
            working_directory: working_dir,
            model,
            resume_session,
            worktree_id: start_conv.worktree_id,
        };

        // Track active stream with pending config
        self.active_streams.write().await.insert(
            request_id.to_string(),
            ActiveStream {
                session_id: sid.clone(),
                pending_config: Some(config),
            },
        );

        // Spawn task to forward broadcast events as StreamData frames
        let tx = self.outbound_tx.clone();
        let rid = request_id.to_string();
        let active_streams = Arc::clone(&self.active_streams);
        let mux = Arc::clone(&self.multiplexer);
        let crypto = self.crypto_snapshot().await;
        let mut event_rx = handle.event_rx;
        let sid_spawn = sid.clone();
        let client_id_spawn = client_id.clone();

        tokio::spawn(async move {
            let mut seq = 0u64;
            debug!(request_id = %rid, session_id = %sid_spawn, "Event forwarder task started, waiting for broadcast events");
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        debug!(
                            request_id = %rid,
                            seq,
                            event_type = ?event.event.as_ref().map(std::mem::discriminant),
                            "Forwarding broadcast event to tunnel"
                        );

                        // Application-layer encryption: if crypto is active,
                        // serialize the real event → encrypt → wrap in an
                        // AgentEvent with Encrypted variant → serialize wrapper.
                        // The relay sees valid protobuf but cannot read the content.
                        let wire_bytes = if let Some(ref session) = crypto {
                            // Serialize the real event
                            let mut inner_buf = Vec::with_capacity(event.encoded_len());
                            if let Err(e) = event.encode(&mut inner_buf) {
                                warn!(request_id = %rid, error = %e, "Failed to encode event for app-layer encryption");
                                continue;
                            }
                            // Encrypt the serialized event
                            let enc_data = match session.encrypt(&inner_buf) {
                                Ok(ed) => ed,
                                Err(e) => {
                                    error!(request_id = %rid, error = %e, "App-layer encryption failed");
                                    continue;
                                }
                            };
                            // Wrap in AgentEvent { encrypted: EncryptedEnvelope }
                            let wrapper = betcode_proto::v1::AgentEvent {
                                sequence: event.sequence,
                                timestamp: event.timestamp,
                                parent_tool_use_id: String::new(),
                                event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                                    betcode_proto::v1::EncryptedEnvelope {
                                        ciphertext: enc_data.ciphertext,
                                        nonce: enc_data.nonce.to_vec(),
                                    },
                                )),
                            };
                            let mut buf = Vec::with_capacity(wrapper.encoded_len());
                            if let Err(e) = wrapper.encode(&mut buf) {
                                warn!(request_id = %rid, error = %e, "Failed to encode encrypted wrapper event");
                                continue;
                            }
                            buf
                        } else {
                            // No crypto: serialize event directly
                            let mut buf = Vec::with_capacity(event.encoded_len());
                            if let Err(e) = event.encode(&mut buf) {
                                warn!(request_id = %rid, error = %e, "Failed to encode plaintext event");
                                continue;
                            }
                            buf
                        };

                        // Tunnel-layer wrapping: skip tunnel-layer encryption when
                        // app-layer is active (same key, redundant work) or when
                        // the request was relay-forwarded (relay needs to decode).
                        let tunnel_crypto = if relay_forwarded || crypto.is_some() {
                            None
                        } else {
                            crypto.as_deref()
                        };
                        let encrypted = match make_encrypted_payload(tunnel_crypto, &wire_bytes) {
                            Ok(enc) => enc,
                            Err(e) => {
                                error!(request_id = %rid, error = %e, "Tunnel-layer encryption failed in event forwarder");
                                continue;
                            }
                        };
                        let frame = TunnelFrame {
                            request_id: rid.clone(),
                            frame_type: FrameType::StreamData as i32,
                            timestamp: Some(prost_types::Timestamp::from(
                                std::time::SystemTime::now(),
                            )),
                            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                                StreamPayload {
                                    method: String::new(),
                                    encrypted: Some(encrypted),
                                    sequence: seq,
                                    metadata: HashMap::new(),
                                },
                            )),
                        };
                        seq += 1;
                        if tx.send(frame).await.is_err() {
                            warn!(request_id = %rid, "Outbound channel closed, stopping event forwarder");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(request_id = %rid, skipped = n, "Broadcast receiver lagged, events lost");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        debug!(request_id = %rid, "Broadcast channel closed, ending event forwarder");
                        break;
                    }
                }
            }

            // Send StreamEnd
            let end_frame = TunnelFrame {
                request_id: rid.clone(),
                frame_type: FrameType::StreamEnd as i32,
                timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                payload: None,
            };
            let _ = tx.send(end_frame).await;

            // Cleanup
            active_streams.write().await.remove(&rid);
            mux.unsubscribe(&sid_spawn, &client_id_spawn).await;
            info!(request_id = %rid, session_id = %sid_spawn, "Converse tunnel stream ended");
        });

        info!(request_id = %request_id, session_id = %sid, "Converse started via tunnel");
    }

    /// Handle a key exchange request: generate ephemeral keypair, compute shared
    /// secret, and install the resulting `CryptoSession`. Returns the daemon's
    /// ephemeral public key (and identity info) unencrypted.
    async fn handle_exchange_keys(&self, request_id: &str, data: &[u8]) -> Vec<TunnelFrame> {
        let req = match KeyExchangeRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                warn!(request_id = %request_id, error = %e, "Failed to decode KeyExchangeRequest");
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    "Invalid key exchange request",
                )];
            }
        };

        if req.ephemeral_pubkey.len() != X25519_PUBKEY_LEN {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                "Invalid ephemeral public key length",
            )];
        }

        if !req.identity_pubkey.is_empty() && req.identity_pubkey.len() != X25519_PUBKEY_LEN {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                "Invalid identity public key length",
            )];
        }

        // Acquire write lock early to serialize concurrent key exchange attempts.
        // This prevents a race where two simultaneous exchanges interleave and
        // one side ends up with a different session than the other.
        let mut crypto_guard = self.crypto.write().await;
        if crypto_guard.is_some() {
            info!(
                request_id = %request_id,
                "Replacing crypto session (new client connection)"
            );
        }

        // Generate our ephemeral keypair and complete the exchange
        let state = self.identity.as_ref().map_or_else(
            KeyExchangeState::new,
            |id| KeyExchangeState::with_identity(Arc::clone(id)),
        );

        let daemon_ephemeral_pub = state.public_bytes();
        let session = match state.complete(&req.ephemeral_pubkey) {
            Ok(s) => s,
            Err(e) => {
                warn!(request_id = %request_id, error = %e, "Key exchange failed");
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    "Key exchange failed",
                )];
            }
        };

        // Install the new session key (write lock already held)
        *crypto_guard = Some(Arc::new(session));
        // Release write lock early so encrypt/decrypt operations aren't blocked
        // while we build the response frame below.
        drop(crypto_guard);

        let (daemon_identity_pubkey, daemon_fingerprint) = self.identity.as_ref().map_or_else(
            || (Vec::new(), String::new()),
            |id| (id.public_bytes().to_vec(), id.fingerprint()),
        );

        info!(
            request_id = %request_id,
            client_fingerprint = %req.fingerprint,
            daemon_fingerprint = %daemon_fingerprint,
            "Key exchange completed"
        );

        match Self::plaintext_response_frame(
            request_id,
            &KeyExchangeResponse {
                daemon_identity_pubkey,
                daemon_fingerprint,
                daemon_ephemeral_pubkey: daemon_ephemeral_pub.to_vec(),
            },
        ) {
            Ok(frame) => vec![frame],
            Err(e) => vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                &e,
            )],
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_resume_session(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let req = match ResumeSessionRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {e}"),
                )]
            }
        };

        #[allow(clippy::cast_possible_wrap)]
        let from_seq = req.from_sequence as i64;
        let messages = match self
            .db
            .get_messages_from_sequence(&req.session_id, from_seq)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Resume failed: {e}"),
                )]
            }
        };

        // Encode stored events as StreamData frames, then a StreamEnd
        // For large replays, we send async via outbound_tx to avoid huge Vec
        let tx = self.outbound_tx.clone();
        let rid = request_id.to_string();
        let sid = req.session_id.clone();
        let crypto = self.crypto_snapshot().await;

        tokio::spawn(async move {
            let mut seq = 0u64;
            for msg in messages {
                let raw_bytes = match base64_decode(&msg.payload) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!(request_id = %rid, session_id = %sid, error = %e, "Failed to decode base64 message payload in resume replay");
                        continue;
                    }
                };

                // Match the converse event forwarder: when crypto is active,
                // wrap in app-layer EncryptedEnvelope so the relay sees valid
                // protobuf. Skip tunnel-layer encryption (redundant with app-layer).
                let wire_bytes = if let Some(ref session) = crypto {
                    let enc_data = match session.encrypt(&raw_bytes) {
                        Ok(ed) => ed,
                        Err(e) => {
                            error!(request_id = %rid, error = %e, "App-layer encryption failed in resume");
                            continue;
                        }
                    };
                    let wrapper = betcode_proto::v1::AgentEvent {
                        sequence: seq,
                        timestamp: None,
                        parent_tool_use_id: String::new(),
                        event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
                            betcode_proto::v1::EncryptedEnvelope {
                                ciphertext: enc_data.ciphertext,
                                nonce: enc_data.nonce.to_vec(),
                            },
                        )),
                    };
                    let mut buf = Vec::with_capacity(wrapper.encoded_len());
                    if let Err(e) = wrapper.encode(&mut buf) {
                        warn!(request_id = %rid, error = %e, "Failed to encode encrypted wrapper in resume replay");
                        continue;
                    }
                    buf
                } else {
                    raw_bytes
                };

                let tunnel_crypto = if relay_forwarded || crypto.is_some() {
                    None
                } else {
                    crypto.as_deref()
                };
                let encrypted = match make_encrypted_payload(tunnel_crypto, &wire_bytes) {
                    Ok(enc) => enc,
                    Err(e) => {
                        error!(request_id = %rid, error = %e, "Encryption failed in resume replay");
                        continue;
                    }
                };
                let frame = TunnelFrame {
                    request_id: rid.clone(),
                    frame_type: FrameType::StreamData as i32,
                    timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                    payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                        StreamPayload {
                            method: String::new(),
                            encrypted: Some(encrypted),
                            sequence: seq,
                            metadata: HashMap::new(),
                        },
                    )),
                };
                seq += 1;
                if tx.send(frame).await.is_err() {
                    break;
                }
            }
            let end = TunnelFrame {
                request_id: rid.clone(),
                frame_type: FrameType::StreamEnd as i32,
                timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                payload: None,
            };
            let _ = tx.send(end).await;
            info!(request_id = %rid, session_id = %sid, "ResumeSession replay completed via tunnel");
        });

        vec![] // Frames sent async
    }

    // --- CommandService dispatch ---

    #[allow(clippy::too_many_lines)]
    async fn dispatch_command_rpc(
        &self,
        request_id: &str,
        method: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let Some(svc) = &self.command_service else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                "CommandService not available in tunnel handler",
            )];
        };
        match method {
            METHOD_GET_COMMAND_REGISTRY => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetCommandRegistryRequest,
                get_command_registry
            ),
            METHOD_LIST_AGENTS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListAgentsRequest,
                list_agents
            ),
            METHOD_LIST_PATH => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListPathRequest,
                list_path
            ),
            METHOD_LIST_PLUGINS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListPluginsRequest,
                list_plugins
            ),
            METHOD_GET_PLUGIN_STATUS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetPluginStatusRequest,
                get_plugin_status
            ),
            METHOD_ADD_PLUGIN => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                AddPluginRequest,
                add_plugin
            ),
            METHOD_REMOVE_PLUGIN => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                RemovePluginRequest,
                remove_plugin
            ),
            METHOD_ENABLE_PLUGIN => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                EnablePluginRequest,
                enable_plugin
            ),
            METHOD_DISABLE_PLUGIN => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                DisablePluginRequest,
                disable_plugin
            ),
            _ => vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Unknown Command method: {method}"),
            )],
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_execute_service_command(
        &self,
        request_id: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) {
        let Some(cmd_svc) = &self.command_service else {
            let _ = self
                .outbound_tx
                .send(Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    "CommandService not available in tunnel handler",
                ))
                .await;
            return;
        };
        let req = match ExecuteServiceCommandRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("Decode error: {e}"),
                    ))
                    .await;
                return;
            }
        };
        let stream_resp = match cmd_svc.execute_service_command(Request::new(req)).await {
            Ok(resp) => resp,
            Err(status) => {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("ExecuteServiceCommand failed: {}", status.message()),
                    ))
                    .await;
                return;
            }
        };

        let outbound_tx = self.outbound_tx.clone();
        let rid = request_id.to_string();
        // Skip tunnel-layer encryption for relay-forwarded requests (relay can't decrypt)
        let crypto_for_response = if relay_forwarded {
            None
        } else {
            self.crypto_snapshot().await
        };
        tokio::spawn(async move {
            let mut stream = stream_resp.into_inner();
            let mut seq = 0u64;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(output) => {
                        let mut buf = Vec::with_capacity(output.encoded_len());
                        if let Err(e) = output.encode(&mut buf) {
                            warn!(request_id = %rid, error = %e, "Failed to encode command output");
                            continue;
                        }
                        let encrypted = match make_encrypted_payload(crypto_for_response.as_deref(), &buf) {
                            Ok(enc) => enc,
                            Err(e) => {
                                warn!(request_id = %rid, error = %e, "Failed to encrypt command output");
                                continue;
                            }
                        };
                        let frame = TunnelFrame {
                            request_id: rid.clone(),
                            frame_type: FrameType::StreamData as i32,
                            timestamp: Some(prost_types::Timestamp::from(
                                std::time::SystemTime::now(),
                            )),
                            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                                StreamPayload {
                                    method: String::new(),
                                    encrypted: Some(encrypted),
                                    sequence: seq,
                                    metadata: HashMap::new(),
                                },
                            )),
                        };
                        seq += 1;
                        if outbound_tx.send(frame).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, request_id = %rid, "ExecuteServiceCommand stream error");
                        let _ = outbound_tx
                            .send(Self::error_response(
                                &rid,
                                TunnelErrorCode::Internal,
                                &format!("Stream error: {e}"),
                            ))
                            .await;
                        break;
                    }
                }
            }
            // Send StreamEnd
            let _ = outbound_tx
                .send(TunnelFrame {
                    request_id: rid.clone(),
                    frame_type: FrameType::StreamEnd as i32,
                    timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                    payload: None,
                })
                .await;
            info!(request_id = %rid, "ExecuteServiceCommand stream ended");
        });
    }

    // --- GitLabService / WorktreeService tunnel handlers ---

    async fn dispatch_gitlab_rpc(
        &self,
        request_id: &str,
        method: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let Some(svc) = &self.gitlab_service else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                "GitLabService not available in tunnel handler",
            )];
        };
        match method {
            METHOD_LIST_MERGE_REQUESTS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListMergeRequestsRequest,
                list_merge_requests
            ),
            METHOD_GET_MERGE_REQUEST => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetMergeRequestRequest,
                get_merge_request
            ),
            METHOD_LIST_PIPELINES => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListPipelinesRequest,
                list_pipelines
            ),
            METHOD_GET_PIPELINE => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetPipelineRequest,
                get_pipeline
            ),
            METHOD_LIST_ISSUES => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListIssuesRequest,
                list_issues
            ),
            METHOD_GET_ISSUE => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetIssueRequest,
                get_issue
            ),
            _ => vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Unknown GitLab method: {method}"),
            )],
        }
    }

    async fn dispatch_repo_rpc(
        &self,
        request_id: &str,
        method: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let Some(svc) = &self.repo_service else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                "GitRepoService not available in tunnel handler",
            )];
        };
        match method {
            METHOD_REGISTER_REPO => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                RegisterRepoRequest,
                register_repo
            ),
            METHOD_UNREGISTER_REPO => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                UnregisterRepoRequest,
                unregister_repo
            ),
            METHOD_LIST_REPOS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListReposRequest,
                list_repos
            ),
            METHOD_GET_REPO => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetRepoRequest,
                get_repo
            ),
            METHOD_UPDATE_REPO => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                UpdateRepoRequest,
                update_repo
            ),
            METHOD_SCAN_REPOS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ScanReposRequest,
                scan_repos
            ),
            _ => vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Unknown GitRepo method: {method}"),
            )],
        }
    }

    async fn dispatch_worktree_rpc(
        &self,
        request_id: &str,
        method: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let Some(svc) = &self.worktree_service else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                "WorktreeService not available in tunnel handler",
            )];
        };
        match method {
            METHOD_CREATE_WORKTREE => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                CreateWorktreeRequest,
                create_worktree
            ),
            METHOD_REMOVE_WORKTREE => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                RemoveWorktreeRequest,
                remove_worktree
            ),
            METHOD_LIST_WORKTREES => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListWorktreesRequest,
                list_worktrees
            ),
            METHOD_GET_WORKTREE => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetWorktreeRequest,
                get_worktree
            ),
            _ => vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Unknown Worktree method: {method}"),
            )],
        }
    }

    // --- ConfigService dispatch ---

    async fn dispatch_config_rpc(
        &self,
        request_id: &str,
        method: &str,
        data: &[u8],
        relay_forwarded: bool,
    ) -> Vec<TunnelFrame> {
        let Some(svc) = &self.config_service else {
            return vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                "ConfigService not available in tunnel handler",
            )];
        };
        match method {
            METHOD_GET_SETTINGS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetSettingsRequest,
                get_settings
            ),
            METHOD_UPDATE_SETTINGS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                UpdateSettingsRequest,
                update_settings
            ),
            METHOD_LIST_MCP_SERVERS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                ListMcpServersRequest,
                list_mcp_servers
            ),
            METHOD_GET_PERMISSIONS => dispatch_rpc!(
                self,
                svc,
                request_id,
                data,
                relay_forwarded,
                GetPermissionsRequest,
                get_permissions
            ),
            _ => vec![Self::error_response(
                request_id,
                TunnelErrorCode::NotFound,
                &format!("Unknown Config method: {method}"),
            )],
        }
    }

    /// Build a unary response frame, skipping tunnel-layer encryption for
    /// relay-forwarded requests (so the relay can decode the protobuf).
    async fn unary_response_frame<M: Message>(
        &self,
        request_id: &str,
        msg: &M,
        relay_forwarded: bool,
    ) -> TunnelFrame {
        if relay_forwarded {
            match Self::plaintext_response_frame(request_id, msg) {
                Ok(f) => f,
                Err(e) => Self::error_response(request_id, TunnelErrorCode::Internal, &e),
            }
        } else {
            self.response_frame_or_error(request_id, msg).await
        }
    }

    /// Build a unary response frame, returning an error frame if encryption fails.
    async fn response_frame_or_error<M: Message>(&self, request_id: &str, msg: &M) -> TunnelFrame {
        match self.response_frame(request_id, msg).await {
            Ok(frame) => frame,
            Err(e) => Self::error_response(request_id, TunnelErrorCode::Internal, &e),
        }
    }

    /// Build a unary response frame from a prost message, encrypting if crypto is set.
    async fn response_frame<M: Message>(
        &self,
        request_id: &str,
        msg: &M,
    ) -> Result<TunnelFrame, String> {
        let mut buf = Vec::with_capacity(msg.encoded_len());
        msg.encode(&mut buf)
            .map_err(|e| format!("Prost encode failed: {e}"))?;
        Ok(TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Response as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                StreamPayload {
                    method: String::new(),
                    encrypted: Some(self.encrypt_payload(&buf).await?),
                    sequence: 0,
                    metadata: HashMap::new(),
                },
            )),
        })
    }

    /// Build a plaintext (unencrypted) response frame. Used for key exchange
    /// responses that happen before a session key is established.
    fn plaintext_response_frame<M: Message>(
        request_id: &str,
        msg: &M,
    ) -> Result<TunnelFrame, String> {
        let mut buf = Vec::with_capacity(msg.encoded_len());
        msg.encode(&mut buf)
            .map_err(|e| format!("encode failed: {e}"))?;
        let encrypted = make_encrypted_payload(None, &buf)?;
        Ok(TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Response as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                StreamPayload {
                    method: String::new(),
                    encrypted: Some(encrypted),
                    sequence: 0,
                    metadata: HashMap::new(),
                },
            )),
        })
    }

    /// Create an error response frame.
    pub fn error_response(request_id: &str, code: TunnelErrorCode, message: &str) -> TunnelFrame {
        TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Error as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::Error(
                TunnelError {
                    code: code as i32,
                    message: message.to_string(),
                    details: HashMap::new(),
                },
            )),
        }
    }

    pub const fn relay(&self) -> &Arc<SessionRelay> {
        &self.relay
    }
    pub const fn multiplexer(&self) -> &Arc<SessionMultiplexer> {
        &self.multiplexer
    }
    pub const fn db(&self) -> &Database {
        &self.db
    }
}

/// Encrypt data with the session if available, or wrap raw bytes (passthrough).
fn make_encrypted_payload(
    crypto: Option<&CryptoSession>,
    data: &[u8],
) -> Result<EncryptedPayload, String> {
    match crypto {
        Some(c) => {
            let enc = c
                .encrypt(data)
                .map_err(|e| format!("Encryption failed: {e}"))?;
            Ok(EncryptedPayload {
                ciphertext: enc.ciphertext,
                nonce: enc.nonce.to_vec(),
                ephemeral_pubkey: Vec::new(),
            })
        }
        None => Ok(EncryptedPayload {
            ciphertext: data.to_vec(),
            nonce: Vec::new(),
            ephemeral_pubkey: Vec::new(),
        }),
    }
}

use betcode_core::db::base64_decode;

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::redundant_clone,
    clippy::implicit_clone,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::doc_markdown,
    clippy::single_match_else,
    clippy::significant_drop_tightening,
    clippy::map_unwrap_or,
    clippy::uninlined_format_args,
    clippy::iter_on_single_items,
)]
#[path = "handler_tests.rs"]
mod tests;
