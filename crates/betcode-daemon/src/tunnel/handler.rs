//! Tunnel request handler that routes incoming frames to local services.

use std::collections::HashMap;
use std::sync::Arc;

use prost::Message;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use betcode_proto::v1::{
    AgentRequest, CancelTurnRequest, CancelTurnResponse, CompactSessionRequest,
    CompactSessionResponse, EncryptedPayload, FrameType, InputLockRequest, InputLockResponse,
    KeyExchangeRequest, KeyExchangeResponse, ListSessionsRequest, ListSessionsResponse,
    ResumeSessionRequest, SessionSummary, StreamPayload, TunnelError, TunnelErrorCode, TunnelFrame,
};

use betcode_crypto::{CryptoSession, IdentityKeyPair, KeyExchangeState};

use crate::relay::SessionRelay;
use crate::session::SessionMultiplexer;
use crate::storage::Database;

/// Methods recognized by the tunnel handler for dispatching.
pub const METHOD_LIST_SESSIONS: &str = "AgentService/ListSessions";
pub const METHOD_COMPACT_SESSION: &str = "AgentService/CompactSession";
pub const METHOD_CANCEL_TURN: &str = "AgentService/CancelTurn";
pub const METHOD_REQUEST_INPUT_LOCK: &str = "AgentService/RequestInputLock";
pub const METHOD_CONVERSE: &str = "AgentService/Converse";
pub const METHOD_RESUME_SESSION: &str = "AgentService/ResumeSession";
pub const METHOD_EXCHANGE_KEYS: &str = "AgentService/ExchangeKeys";

/// Default maximum number of sessions returned by ListSessions.
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
    /// Deferred session config — subprocess is only started on first UserMessage.
    pending_config: Option<crate::relay::RelaySessionConfig>,
}

/// Handles incoming tunnel frames by dispatching to local gRPC services.
pub struct TunnelRequestHandler {
    machine_id: String,
    relay: Arc<SessionRelay>,
    multiplexer: Arc<SessionMultiplexer>,
    db: Database,
    /// Sender for pushing response frames back through the tunnel.
    outbound_tx: mpsc::Sender<TunnelFrame>,
    /// Active streaming sessions keyed by request_id.
    active_streams: Arc<RwLock<HashMap<String, ActiveStream>>>,
    /// E2E crypto session for encrypt/decrypt. None = passthrough (no encryption).
    crypto: Arc<RwLock<Option<Arc<CryptoSession>>>>,
    /// Identity keypair for key exchange. None = key exchange disabled.
    identity: Option<Arc<IdentityKeyPair>>,
}

impl TunnelRequestHandler {
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
        }
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
                }
                vec![]
            }
            Ok(frame_type) => {
                warn!(request_id = %request_id, ?frame_type, "Unexpected frame type");
                vec![Self::error_response(
                    &request_id,
                    TunnelErrorCode::Internal,
                    &format!("Unexpected frame type: {:?}", frame_type),
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

    /// Decrypt an EncryptedPayload using the session key, or passthrough if no crypto.
    ///
    /// When the nonce is empty, the payload is treated as plaintext passthrough.
    /// This happens when the relay forwards data without tunnel-layer encryption
    /// (the relay doesn't have the crypto keys). App-layer encryption
    /// (EncryptedEnvelope) handles the actual E2E protection.
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
        match crypto {
            Some(crypto) => crypto
                .decrypt(&enc.ciphertext, &enc.nonce)
                .map_err(|e| format!("decryption failed: {e}")),
            None => {
                debug!("Tunnel-layer passthrough (no crypto session) — app-layer handles E2E");
                Ok(enc.ciphertext.clone())
            }
        }
    }

    /// Encrypt data into an EncryptedPayload using the session key, or passthrough.
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

    async fn handle_request(&self, request_id: String, frame: TunnelFrame) -> Vec<TunnelFrame> {
        let payload = match frame.payload {
            Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) => p,
            _ => {
                return vec![Self::error_response(
                    &request_id,
                    TunnelErrorCode::Internal,
                    "Missing StreamPayload",
                )]
            }
        };

        debug!(request_id = %request_id, method = %payload.method, machine_id = %self.machine_id, "Handling tunneled request");

        // ExchangeKeys is handled before decryption (no session key yet).
        if payload.method == METHOD_EXCHANGE_KEYS {
            let data = payload
                .encrypted
                .as_ref()
                .map(|e| e.ciphertext.clone())
                .unwrap_or_default();
            return self.handle_exchange_keys(&request_id, &data).await;
        }

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
            METHOD_LIST_SESSIONS => self.handle_list_sessions(&request_id, &data).await,
            METHOD_COMPACT_SESSION => self.handle_compact_session(&request_id, &data).await,
            METHOD_CANCEL_TURN => self.handle_cancel_turn(&request_id, &data).await,
            METHOD_REQUEST_INPUT_LOCK => self.handle_request_input_lock(&request_id, &data).await,
            METHOD_CONVERSE => {
                self.handle_converse(&request_id, &data).await;
                vec![] // Responses sent asynchronously via outbound_tx
            }
            METHOD_RESUME_SESSION => self.handle_resume_session(&request_id, &data).await,
            other => vec![Self::error_response(
                &request_id,
                TunnelErrorCode::NotFound,
                &format!("Unknown method: {}", other),
            )],
        }
    }

    async fn handle_list_sessions(&self, request_id: &str, data: &[u8]) -> Vec<TunnelFrame> {
        let req = match ListSessionsRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {}", e),
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
                let summaries: Vec<SessionSummary> = sessions
                    .into_iter()
                    .map(|s| SessionSummary {
                        id: s.id,
                        model: s.model,
                        working_directory: s.working_directory,
                        worktree_id: s.worktree_id.unwrap_or_default(),
                        status: s.status,
                        message_count: 0,
                        total_input_tokens: s.total_input_tokens as u32,
                        total_output_tokens: s.total_output_tokens as u32,
                        total_cost_usd: s.total_cost_usd,
                        created_at: Some(prost_types::Timestamp {
                            seconds: s.created_at,
                            nanos: 0,
                        }),
                        updated_at: Some(prost_types::Timestamp {
                            seconds: s.updated_at,
                            nanos: 0,
                        }),
                        last_message_preview: s.last_message_preview.unwrap_or_default(),
                    })
                    .collect();
                let total = summaries.len() as u32;
                vec![
                    self.response_frame_or_error(
                        request_id,
                        &ListSessionsResponse {
                            sessions: summaries,
                            total,
                        },
                    )
                    .await,
                ]
            }
            Err(e) => vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                &format!("ListSessions failed: {}", e),
            )],
        }
    }

    async fn handle_compact_session(&self, request_id: &str, data: &[u8]) -> Vec<TunnelFrame> {
        let req = match CompactSessionRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {}", e),
                )]
            }
        };
        let sid = &req.session_id;
        let messages_before = match self.db.count_messages(sid).await {
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
                self.response_frame_or_error(
                    request_id,
                    &CompactSessionResponse {
                        messages_before: 0,
                        messages_after: 0,
                        tokens_saved: 0,
                    },
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
        let cutoff = max_seq - keep_count as i64;
        if cutoff <= 0 {
            return vec![
                self.response_frame_or_error(
                    request_id,
                    &CompactSessionResponse {
                        messages_before,
                        messages_after: messages_before,
                        tokens_saved: 0,
                    },
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
        let _ = self.db.update_compaction_sequence(sid, cutoff).await;
        let messages_after = messages_before - deleted as u32;
        let tokens_saved = deleted as u32 * ESTIMATED_TOKENS_PER_MESSAGE;
        info!(session_id = %sid, messages_before, messages_after, "Session compacted via tunnel");
        vec![
            self.response_frame_or_error(
                request_id,
                &CompactSessionResponse {
                    messages_before,
                    messages_after,
                    tokens_saved,
                },
            )
            .await,
        ]
    }

    async fn handle_cancel_turn(&self, request_id: &str, data: &[u8]) -> Vec<TunnelFrame> {
        let req = match CancelTurnRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {}", e),
                )]
            }
        };
        let was_active = self
            .relay
            .cancel_session(&req.session_id)
            .await
            .unwrap_or(false);
        vec![
            self.response_frame_or_error(request_id, &CancelTurnResponse { was_active })
                .await,
        ]
    }

    async fn handle_request_input_lock(&self, request_id: &str, data: &[u8]) -> Vec<TunnelFrame> {
        let req = match InputLockRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {}", e),
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
                self.response_frame_or_error(
                    request_id,
                    &InputLockResponse {
                        granted: true,
                        previous_holder: previous.unwrap_or_default(),
                    },
                )
                .await,
            ],
            Err(e) => vec![Self::error_response(
                request_id,
                TunnelErrorCode::Internal,
                &format!("InputLock failed: {}", e),
            )],
        }
    }

    /// Handle an incoming StreamData frame for an active streaming session.
    /// Routes user messages, permissions, etc. to the relay.
    ///
    /// On the first `UserMessage`, if the subprocess hasn't been started yet
    /// (pending_config is Some), starts it and immediately sends the message.
    ///
    /// If E2E crypto is active, incoming `AgentRequest` messages must use the
    /// `Encrypted` oneof variant containing an `EncryptedEnvelope`. The envelope
    /// is decrypted and re-decoded as the real `AgentRequest`. Plaintext requests
    /// are rejected when crypto is active (prevents downgrade attacks).
    pub async fn handle_incoming_stream_data(&self, request_id: &str, data: &[u8]) {
        let sid = {
            let stream = self.active_streams.read().await;
            match stream.get(request_id) {
                Some(a) => a.session_id.clone(),
                None => {
                    debug!(request_id = %request_id, "StreamData for unknown active stream");
                    return;
                }
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
                        &format!("Start session failed: {}", e),
                    ))
                    .await;
                self.active_streams.write().await.remove(request_id);
                return;
            }
        }

        use betcode_proto::v1::agent_request::Request;
        match req.request {
            Some(Request::Message(msg)) => {
                if let Err(e) = self.relay.send_user_message(&sid, &msg.content).await {
                    warn!(session_id = %sid, error = %e, "Failed to send user message via tunnel");
                }
            }
            Some(Request::Permission(perm)) => {
                let granted = perm.decision
                    == betcode_proto::v1::PermissionDecision::AllowOnce as i32
                    || perm.decision == betcode_proto::v1::PermissionDecision::AllowSession as i32;
                if let Err(e) = self
                    .relay
                    .send_permission_response(&sid, &perm.request_id, granted)
                    .await
                {
                    warn!(session_id = %sid, error = %e, "Failed to send permission via tunnel");
                }
            }
            Some(Request::Cancel(_)) => {
                let _ = self.relay.cancel_session(&sid).await;
            }
            _ => {
                debug!(request_id = %request_id, "Ignoring non-actionable StreamData request");
            }
        }
    }

    /// Check if a request_id has an active streaming session.
    pub async fn has_active_stream(&self, request_id: &str) -> bool {
        self.active_streams.read().await.contains_key(request_id)
    }

    async fn handle_converse(&self, request_id: &str, data: &[u8]) {
        let outer_req = match AgentRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("Decode error: {}", e),
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

        let start_conv = match start.request {
            Some(betcode_proto::v1::agent_request::Request::Start(s)) => s,
            _ => {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        "First Converse message must be StartConversation",
                    ))
                    .await;
                return;
            }
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
        let resume_session = match self.db.get_session(&sid).await {
            Ok(existing) => existing.claude_session_id.filter(|s| !s.is_empty()),
            Err(_) => {
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
                            &format!("Create session failed: {}", e),
                        ))
                        .await;
                    return;
                }
                None
            }
        };

        let _ = self
            .db
            .update_session_status(&sid, crate::storage::SessionStatus::Active)
            .await;

        let client_id = generate_tunnel_client_id();
        let handle = match self.multiplexer.subscribe(&sid, &client_id, "tunnel").await {
            Ok(h) => h,
            Err(e) => {
                let _ = self
                    .outbound_tx
                    .send(Self::error_response(
                        request_id,
                        TunnelErrorCode::Internal,
                        &format!("Subscribe failed: {}", e),
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
                            if event.encode(&mut inner_buf).is_err() {
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
                            if wrapper.encode(&mut buf).is_err() {
                                continue;
                            }
                            buf
                        } else {
                            // No crypto: serialize event directly
                            let mut buf = Vec::with_capacity(event.encoded_len());
                            if event.encode(&mut buf).is_err() {
                                continue;
                            }
                            buf
                        };

                        // Tunnel-layer wrapping: skip tunnel-layer encryption when
                        // app-layer is active (same key, redundant work). Pass None
                        // so the bytes go through as a passthrough EncryptedPayload.
                        let tunnel_crypto = if crypto.is_some() { None } else { crypto.as_deref() };
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
                        continue;
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
    /// secret, and install the resulting CryptoSession. Returns the daemon's
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
            warn!(
                request_id = %request_id,
                "Replacing existing crypto session with new key exchange"
            );
        }

        // Generate our ephemeral keypair and complete the exchange
        let state = match &self.identity {
            Some(id) => KeyExchangeState::with_identity(Arc::clone(id)),
            None => KeyExchangeState::new(),
        };

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

        let (daemon_identity_pubkey, daemon_fingerprint) = match &self.identity {
            Some(id) => (id.public_bytes().to_vec(), id.fingerprint()),
            None => (Vec::new(), String::new()),
        };

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

    async fn handle_resume_session(&self, request_id: &str, data: &[u8]) -> Vec<TunnelFrame> {
        let req = match ResumeSessionRequest::decode(data) {
            Ok(r) => r,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Decode error: {}", e),
                )]
            }
        };

        let messages = match self
            .db
            .get_messages_from_sequence(&req.session_id, req.from_sequence as i64)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                return vec![Self::error_response(
                    request_id,
                    TunnelErrorCode::Internal,
                    &format!("Resume failed: {}", e),
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
                let bytes = match base64_decode(&msg.payload) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let encrypted = match make_encrypted_payload(crypto.as_deref(), &bytes) {
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
            .map_err(|e| format!("Prost encode failed: {}", e))?;
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

    pub fn relay(&self) -> &Arc<SessionRelay> {
        &self.relay
    }
    pub fn multiplexer(&self) -> &Arc<SessionMultiplexer> {
        &self.multiplexer
    }
    pub fn db(&self) -> &Database {
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
                .map_err(|e| format!("Encryption failed: {}", e))?;
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
#[path = "handler_tests.rs"]
mod tests;
