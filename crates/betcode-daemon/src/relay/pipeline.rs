//! Session relay pipeline: subprocess ↔ gRPC event bridging.
//!
//! Data flow:
//! ```text
//! subprocess stdout → NDJSON parser → EventBridge → event_forwarder → broadcast
//! gRPC UserMessage → JSON → subprocess stdin
//! gRPC PermissionResponse → control_response JSON → subprocess stdin
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use betcode_core::ndjson;
use betcode_proto::v1::AgentEvent;

use crate::session::SessionMultiplexer;
use crate::storage::Database;
use crate::subprocess::{EventBridge, SpawnConfig, SubprocessManager};

use super::types::{RelayHandle, RelaySessionConfig, RelayError};

/// Session relay manages the lifecycle of subprocess ↔ gRPC bridging.
pub struct SessionRelay {
    subprocess_manager: Arc<SubprocessManager>,
    multiplexer: Arc<SessionMultiplexer>,
    db: Database,
    /// Maps `session_id` → `RelayHandle` for active sessions.
    sessions: Arc<RwLock<HashMap<String, RelayHandle>>>,
}

impl SessionRelay {
    /// Create a new session relay.
    pub fn new(
        subprocess_manager: Arc<SubprocessManager>,
        multiplexer: Arc<SessionMultiplexer>,
        db: Database,
    ) -> Self {
        Self {
            subprocess_manager,
            multiplexer,
            db,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start a new relay session, spawning a subprocess and wiring up the
    /// NDJSON → `EventBridge` → Multiplexer pipeline.
    pub async fn start_session(
        &self,
        config: RelaySessionConfig,
    ) -> Result<RelayHandle, RelayError> {
        let session_id = config.session_id.clone();

        // Return existing handle if session already active
        {
            let sessions = self.sessions.read().await;
            if let Some(handle) = sessions.get(&session_id) {
                return Ok(handle.clone());
            }
        }

        // Create event forwarder from multiplexer
        let event_forwarder = self.multiplexer.create_event_forwarder(session_id.clone());

        // Create channel for subprocess stdout lines
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(256);

        // Spawn the Claude subprocess
        let spawn_config = SpawnConfig {
            working_directory: config.working_directory,
            prompt: None,
            resume_session: config.resume_session,
            model: config.model,
            max_processes: 5,
            ..Default::default()
        };

        let process_handle = self
            .subprocess_manager
            .spawn(spawn_config, stdout_tx)
            .await?;

        // Shared sequence counter between stdout pipeline and user message storage.
        // Initialized from DB max so resumed sessions don't collide.
        #[allow(clippy::cast_sign_loss)]
        let start_seq = self.db.max_message_sequence(&session_id).await.unwrap_or(0) as u64;
        let sequence_counter = Arc::new(AtomicU64::new(start_seq));

        let pending_question_inputs = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let pending_permissions = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let session_grants = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

        let relay_handle = RelayHandle {
            process_id: process_handle.id.clone(),
            session_id: session_id.clone(),
            stdin_tx: process_handle.stdin_tx.clone(),
            sequence_counter: Arc::clone(&sequence_counter),
            pending_question_inputs: Arc::clone(&pending_question_inputs),
            pending_permissions: Arc::clone(&pending_permissions),
            session_grants: Arc::clone(&session_grants),
        };

        // Spawn the NDJSON reader pipeline
        spawn_stdout_pipeline(StdoutPipelineContext {
            session_id: session_id.clone(),
            stdout_rx,
            event_forwarder,
            db: self.db.clone(),
            sessions: Arc::clone(&self.sessions),
            subprocess_manager: Arc::clone(&self.subprocess_manager),
            sequence_counter,
            pending_question_inputs,
            pending_permissions,
            session_grants,
            stdin_tx: process_handle.stdin_tx.clone(),
        });

        // Store the relay handle
        self.sessions
            .write()
            .await
            .insert(session_id.clone(), relay_handle.clone());

        info!(session_id, process_id = %process_handle.id, "Relay session started");
        Ok(relay_handle)
    }

    /// Send a user message to the subprocess via stdin.
    ///
    /// Also stores a `UserInput` event in the DB so the message appears on resume.
    /// When `agent_id` is non-empty the message targets a specific agent instance.
    pub async fn send_user_message(
        &self,
        session_id: &str,
        content: &str,
        agent_id: Option<&str>,
    ) -> Result<(), RelayError> {
        let handle = self.get_active_handle(session_id).await?;

        // Atomically allocate a sequence number for the user input event.
        let seq = handle.sequence_counter.fetch_add(1, Ordering::AcqRel) + 1;

        // Store a UserInput event so it appears in session history on resume.
        let event = AgentEvent {
            sequence: seq,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::UserInput(
                betcode_proto::v1::UserInput {
                    content: content.to_string(),
                },
            )),
        };
        if let Err(e) = store_event(&self.db, session_id, &event).await {
            warn!(session_id, error = %e, "Failed to store user input event");
        }

        // Claude Code --input-format stream-json expects this JSONL format on stdin.
        // See: https://github.com/anthropics/claude-code/issues/5034
        let mut msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": content,
            },
            "session_id": "default",
            "parent_tool_use_id": null,
        });
        if let Some(aid) = agent_id.filter(|s| !s.is_empty()) {
            msg["agent_id"] = serde_json::Value::String(aid.to_string());
        }
        let line =
            serde_json::to_string(&msg).map_err(|e| RelayError::Serialization(e.to_string()))?;

        handle
            .stdin_tx
            .send(line)
            .await
            .map_err(|_| RelayError::StdinClosed {
                session_id: session_id.to_string(),
            })?;

        debug!(session_id, seq, "Sent user message to subprocess");
        Ok(())
    }

    /// Send a permission response (`control_response`) to the subprocess.
    ///
    /// Format must match the Claude Agent SDK control protocol:
    /// allow → `{ behavior: "allow", updatedInput: <original_tool_input> }`
    /// deny  → `{ behavior: "deny", message: "...", interrupt: true }`
    pub async fn send_permission_response(
        &self,
        session_id: &str,
        request_id: &str,
        granted: bool,
        original_input: &serde_json::Value,
    ) -> Result<(), RelayError> {
        let handle = self.get_active_handle(session_id).await?;

        let line = build_permission_response_json(request_id, granted, original_input);

        handle
            .stdin_tx
            .send(line)
            .await
            .map_err(|_| RelayError::StdinClosed {
                session_id: session_id.to_string(),
            })?;

        debug!(session_id, request_id, granted, "Sent permission response");
        Ok(())
    }

    /// Send an `AskUserQuestion` response to the subprocess.
    ///
    /// Format matches the Claude Agent SDK control protocol for `AskUserQuestion`:
    /// ```json
    /// {"type":"control_response","response":{"subtype":"success","request_id":"req_002",
    ///   "response":{"behavior":"allow","updatedInput":{...original_input..., "answers":{...}}}}}
    /// ```
    pub async fn send_question_response(
        &self,
        session_id: &str,
        request_id: &str,
        answers: &std::collections::HashMap<String, String>,
        original_input: &serde_json::Value,
    ) -> Result<(), RelayError> {
        let handle = self.get_active_handle(session_id).await?;

        let line = build_question_response_json(request_id, answers, original_input);

        handle
            .stdin_tx
            .send(line)
            .await
            .map_err(|_| RelayError::StdinClosed {
                session_id: session_id.to_string(),
            })?;

        debug!(session_id, request_id, "Sent question response");
        Ok(())
    }

    /// Send a raw JSON line to the subprocess stdin.
    pub async fn send_raw_stdin(&self, session_id: &str, line: &str) -> Result<(), RelayError> {
        let handle = self.get_active_handle(session_id).await?;

        handle
            .stdin_tx
            .send(line.to_string())
            .await
            .map_err(|_| RelayError::StdinClosed {
                session_id: session_id.to_string(),
            })?;

        debug!(session_id, "Sent raw stdin line to subprocess");
        Ok(())
    }

    /// Cancel the current turn for a session.
    pub async fn cancel_session(&self, session_id: &str) -> Result<bool, RelayError> {
        let process_id = match self.sessions.read().await.get(session_id) {
            Some(h) => h.process_id.clone(),
            None => return Ok(false),
        };

        match self.subprocess_manager.terminate(&process_id).await {
            Ok(()) => {
                self.sessions.write().await.remove(session_id);
                // Update DB status so the pipeline cleanup doesn't override
                if let Err(e) = self
                    .db
                    .update_session_status(session_id, crate::storage::SessionStatus::Idle)
                    .await
                {
                    warn!(session_id, error = %e, "Failed to update status on cancel");
                }
                info!(session_id, "Relay session cancelled");
                Ok(true)
            }
            Err(e) => {
                warn!(session_id, error = %e, "Failed to terminate");
                Err(RelayError::Subprocess(e))
            }
        }
    }

    /// Get a relay handle for a session (if active).
    pub async fn get_handle(&self, session_id: &str) -> Option<RelayHandle> {
        self.sessions.read().await.get(session_id).cloned()
    }

    /// Check if a session has an active relay.
    pub async fn is_active(&self, session_id: &str) -> bool {
        self.sessions.read().await.contains_key(session_id)
    }

    async fn get_active_handle(&self, session_id: &str) -> Result<RelayHandle, RelayError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| RelayError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }
}

/// Shared context for the stdout pipeline task.
struct StdoutPipelineContext {
    session_id: String,
    stdout_rx: mpsc::Receiver<String>,
    event_forwarder: mpsc::Sender<AgentEvent>,
    db: Database,
    sessions: Arc<RwLock<HashMap<String, RelayHandle>>>,
    subprocess_manager: Arc<SubprocessManager>,
    sequence_counter: Arc<AtomicU64>,
    pending_question_inputs: Arc<tokio::sync::RwLock<HashMap<String, serde_json::Value>>>,
    pending_permissions: Arc<tokio::sync::RwLock<HashMap<String, super::types::PendingPermission>>>,
    session_grants: Arc<tokio::sync::RwLock<HashMap<String, bool>>>,
    stdin_tx: tokio::sync::mpsc::Sender<String>,
}

/// Spawn the stdout → NDJSON parser → `EventBridge` → forwarder pipeline.
#[allow(clippy::too_many_lines)]
fn spawn_stdout_pipeline(ctx: StdoutPipelineContext) {
    let StdoutPipelineContext {
        session_id,
        mut stdout_rx,
        event_forwarder,
        db,
        sessions,
        subprocess_manager,
        sequence_counter,
        pending_question_inputs,
        pending_permissions,
        session_grants,
        stdin_tx,
    } = ctx;
    tokio::spawn(async move {
        let sid = session_id.clone();
        // Read the shared counter (may have been advanced by send_user_message).
        let start_seq = sequence_counter.load(Ordering::Acquire);
        let mut bridge = EventBridge::with_start_sequence(start_seq);
        let mut event_count = 0u64;
        let mut had_session_error = false;
        // Track which permission request IDs were auto-responded so we
        // can skip forwarding them to the client.
        let mut auto_responded_requests: HashSet<String> = HashSet::new();

        while let Some(line) = stdout_rx.recv().await {
            auto_responded_requests.clear();
            let msg = match ndjson::parse_line(&line) {
                Ok(msg) => msg,
                Err(e) => {
                    warn!(session_id = %sid, error = %e, "NDJSON parse error");
                    continue;
                }
            };

            // Check if send_user_message advanced the counter since our last batch.
            let latest_seq = sequence_counter.load(Ordering::Acquire);
            if latest_seq > bridge.sequence() {
                // Re-initialize bridge to skip past user input sequences.
                // Safe: user messages only arrive between turns when pending_tools is empty.
                bridge = EventBridge::with_start_sequence(latest_seq);
            }

            let events = bridge.convert(msg);

            for event in &events {
                // Transfer pending question inputs from bridge → shared map
                if let Some(betcode_proto::v1::agent_event::Event::UserQuestion(ref q)) =
                    event.event
                {
                    if let Some(input) = bridge.take_question_input(&q.question_id) {
                        pending_question_inputs
                            .write()
                            .await
                            .insert(q.question_id.clone(), input);
                    }
                }
                // Transfer pending permission inputs from bridge → shared map,
                // or auto-respond if a session grant exists for this tool.
                if let Some(betcode_proto::v1::agent_event::Event::PermissionRequest(ref p)) =
                    event.event
                {
                    if let Some(input) = bridge.take_permission_input(&p.request_id) {
                        // Check session_grants for a cached decision on this tool:
                        //   Some(true)  → auto-allow (skip prompt, grant immediately)
                        //   Some(false) → auto-deny  (skip prompt, deny immediately)
                        //   None        → no cached decision, forward to client for user prompt
                        let grant = session_grants.read().await.get(&p.tool_name).copied();
                        if let Some(granted) = grant {
                            // Auto-respond: send permission response directly to stdin
                            let line = build_permission_response_json(
                                &p.request_id,
                                granted,
                                &input,
                            );
                            if let Err(e) = stdin_tx.send(line).await {
                                warn!(
                                    session_id = %sid,
                                    request_id = %p.request_id,
                                    error = %e,
                                    "Failed to send auto-permission response"
                                );
                            } else {
                                debug!(
                                    session_id = %sid,
                                    request_id = %p.request_id,
                                    tool_name = %p.tool_name,
                                    granted,
                                    "Auto-responded to permission request from session grant"
                                );
                            }
                            auto_responded_requests.insert(p.request_id.clone());
                        } else {
                            // No grant — store for handler to process
                            pending_permissions
                                .write()
                                .await
                                .insert(p.request_id.clone(), super::types::PendingPermission {
                                    input,
                                    tool_name: p.tool_name.clone(),
                                });
                        }
                    }
                }
            }

            for event in events {
                // Skip forwarding auto-responded permission requests to the client
                if let Some(betcode_proto::v1::agent_event::Event::PermissionRequest(ref p)) =
                    event.event
                {
                    if auto_responded_requests.contains(&p.request_id) {
                        // Still store the event for replay, but don't forward to client
                        if let Err(e) = store_event(&db, &sid, &event).await {
                            warn!(session_id = %sid, error = %e, "Failed to store auto-responded event");
                        }
                        continue;
                    }
                }
                event_count += 1;

                // Update usage stats in DB when we get a UsageReport
                if let Some(betcode_proto::v1::agent_event::Event::Usage(ref usage)) = event.event {
                    if let Err(e) = db
                        .update_session_usage(
                            &sid,
                            i64::from(usage.input_tokens),
                            i64::from(usage.output_tokens),
                            usage.cost_usd,
                        )
                        .await
                    {
                        warn!(session_id = %sid, error = %e, "Failed to update usage");
                    }
                }

                // Track if we received a session error (e.g. resume failure)
                if let Some(betcode_proto::v1::agent_event::Event::Error(ref err)) = event.event {
                    if err.code == "session_error" {
                        had_session_error = true;
                    }
                }

                if let Err(e) = store_event(&db, &sid, &event).await {
                    warn!(session_id = %sid, error = %e, "Failed to store event");
                }
                if event_forwarder.send(event).await.is_err() {
                    warn!(session_id = %sid, "Event forwarder closed");
                    return;
                }
            }

            // Sync the shared counter so send_user_message sees the latest sequence.
            sequence_counter.store(bridge.sequence(), Ordering::Release);

            // Update subprocess and DB with Claude's session ID from SystemInit.
            // Skip if we had a session error (e.g. resume failure) — the new session
            // Claude started has no context, so don't persist its ID for future resumes.
            if let Some(info) = bridge.session_info() {
                if had_session_error {
                    warn!(
                        session_id = %sid,
                        "Skipping claude_session_id update — session error was detected"
                    );
                    // Clear stale session ID so next attempt starts fresh
                    if let Err(e) = db.update_claude_session_id(&sid, "").await {
                        warn!(session_id = %sid, error = %e, "Failed to clear claude_session_id");
                    }
                } else {
                    if let Some(handle) = sessions.write().await.get_mut(&sid) {
                        if let Err(e) = subprocess_manager
                            .set_session_id(&handle.process_id, info.session_id.clone())
                            .await
                        {
                            warn!(session_id = %sid, error = %e, "Failed to set subprocess session ID");
                        }
                    }
                    // Persist Claude session ID for future resume operations
                    if let Err(e) = db.update_claude_session_id(&sid, &info.session_id).await {
                        warn!(session_id = %sid, error = %e, "Failed to update claude_session_id");
                    }
                }
            }
        }

        info!(session_id = %sid, event_count, "Stdout pipeline finished");

        // If subprocess exited without producing any events, send an error to the client
        // so it doesn't hang waiting forever. Common causes: missing CLI flags, bad config.
        if event_count == 0 {
            warn!(session_id = %sid, "Subprocess exited with zero events — sending error to client");
            let error_event = AgentEvent {
                sequence: 0,
                timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::Error(
                    betcode_proto::v1::ErrorEvent {
                        code: "subprocess_failed".to_string(),
                        message: "Claude subprocess exited without producing output. Check daemon logs for stderr details.".to_string(),
                        is_fatal: true,
                        details: HashMap::default(),
                    },
                )),
            };
            let _ = event_forwarder.send(error_event).await;
        }

        // Only update DB status if the session is still in the active map
        // (cancel_session removes it and updates status itself)
        let was_active = sessions.write().await.remove(&sid).is_some();
        if was_active {
            if let Err(e) = db
                .update_session_status(&sid, crate::storage::SessionStatus::Idle)
                .await
            {
                warn!(session_id = %sid, error = %e, "Failed to mark session idle");
            }
        }
    });
}

/// Determine the `message_type` string from an `AgentEvent` for DB storage.
const fn event_message_type(event: &AgentEvent) -> &'static str {
    use betcode_proto::v1::agent_event::Event;
    match &event.event {
        Some(Event::ToolCallResult(_) | Event::Usage(_) | Event::TurnComplete(_)) => "result",
        Some(Event::PermissionRequest(_) | Event::UserQuestion(_)) => "control_request",
        Some(Event::SessionInfo(_)) => "system",
        Some(Event::UserInput(_)) => "user",
        Some(
            Event::TextDelta(_)
            | Event::ToolCallStart(_)
            | Event::StatusChange(_)
            | Event::Error(_)
            | Event::TodoUpdate(_)
            | Event::PlanMode(_)
            | Event::Encrypted(_),
        )
        | None => "stream_event",
    }
}

/// Store an event in the database for replay support.
/// Events are serialized as JSON for readable storage and reliable deserialization.
async fn store_event(db: &Database, session_id: &str, event: &AgentEvent) -> Result<(), String> {
    use prost::Message;

    // Encode to protobuf bytes, then base64 for safe text storage
    let mut buf = Vec::new();
    event.encode(&mut buf).map_err(|e| e.to_string())?;
    let payload = betcode_core::db::base64_encode(&buf);
    let msg_type = event_message_type(event);

    #[allow(clippy::cast_possible_wrap)]
    let sequence = event.sequence as i64;
    db.insert_message(session_id, sequence, msg_type, &payload)
        .await
        .map_err(|e| e.to_string())
        .map(|_| ())
}

/// Build the JSON line for a permission `control_response`.
///
/// For allow: include `updatedInput` with the original tool arguments.
/// Claude Code's Zod schema REQUIRES `updatedInput` to be a record (object).
/// Omitting it causes a `ZodError`; sending `{}` replaces all args with empty.
/// The correct behavior is to pass back the original tool input unchanged.
fn build_permission_response_json(
    request_id: &str,
    granted: bool,
    original_input: &serde_json::Value,
) -> String {
    let response = if granted {
        serde_json::json!({
            "behavior": "allow",
            "updatedInput": original_input
        })
    } else {
        serde_json::json!({
            "behavior": "deny",
            "message": "User denied permission",
            "interrupt": true
        })
    };

    let msg = serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response
        }
    });
    #[allow(clippy::expect_used)]
    serde_json::to_string(&msg).expect("permission response serialization should not fail")
}

/// Build the JSON line for an `AskUserQuestion` `control_response`.
///
/// The response includes `updatedInput` with both the original questions and the user's answers,
/// matching the Claude Agent SDK protocol spec.
fn build_question_response_json(
    request_id: &str,
    answers: &std::collections::HashMap<String, String>,
    original_input: &serde_json::Value,
) -> String {
    let mut updated_input = original_input.clone();
    if let serde_json::Value::Object(ref mut map) = updated_input {
        map.insert(
            "answers".to_string(),
            serde_json::to_value(answers).unwrap_or_default(),
        );
    }

    let msg = serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {
                "behavior": "allow",
                "updatedInput": updated_input
            }
        }
    });
    #[allow(clippy::expect_used)]
    serde_json::to_string(&msg).expect("question response serialization should not fail")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used, clippy::iter_on_single_items)]
mod tests {
    use super::*;

    /// Create a minimal `RelayHandle` for tests. The `stdin_tx` end is connected
    /// to the returned `mpsc::Receiver`.
    fn test_relay_handle() -> (RelayHandle, tokio::sync::mpsc::Receiver<String>) {
        use std::sync::atomic::AtomicU64;

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let handle = RelayHandle {
            process_id: "pid-test".into(),
            session_id: "sid-test".into(),
            stdin_tx: tx,
            sequence_counter: Arc::new(AtomicU64::new(0)),
            pending_question_inputs: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            pending_permissions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            session_grants: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        };
        (handle, rx)
    }

    #[tokio::test]
    async fn relay_creation() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = SessionRelay::new(subprocess_mgr, multiplexer, db);
        assert!(!relay.is_active("test-session").await);
    }

    #[tokio::test]
    async fn get_handle_returns_none_for_unknown() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = SessionRelay::new(subprocess_mgr, multiplexer, db);
        assert!(relay.get_handle("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn send_message_fails_for_unknown_session() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = SessionRelay::new(subprocess_mgr, multiplexer, db);
        let result = relay.send_user_message("nonexistent", "hello", None).await;
        assert!(matches!(result, Err(RelayError::SessionNotFound { .. })));
    }

    #[tokio::test]
    async fn send_permission_fails_for_unknown_session() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = SessionRelay::new(subprocess_mgr, multiplexer, db);
        let result = relay
            .send_permission_response("nonexistent", "req-1", true, &serde_json::json!({}))
            .await;
        assert!(matches!(result, Err(RelayError::SessionNotFound { .. })));
    }

    #[tokio::test]
    async fn cancel_unknown_session_returns_false() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = SessionRelay::new(subprocess_mgr, multiplexer, db);
        let result = relay.cancel_session("nonexistent").await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn send_question_response_fails_for_unknown_session() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = SessionRelay::new(subprocess_mgr, multiplexer, db);
        let answers: HashMap<String, String> =
            [("Which database?".to_string(), "SQLite".to_string())]
                .into_iter()
                .collect();
        let original_input = serde_json::json!({
            "questions": [{"question": "Which database?", "options": [{"label": "SQLite"}]}]
        });
        let result = relay
            .send_question_response("nonexistent", "req_q1", &answers, &original_input)
            .await;
        assert!(matches!(result, Err(RelayError::SessionNotFound { .. })));
    }

    /// Verify the permission allow JSON includes updatedInput with original tool args.
    ///
    /// Claude Code's Zod schema REQUIRES `updatedInput` to be a record (object).
    /// We must pass back the original tool input so the tool executes with the
    /// correct arguments.
    #[test]
    fn permission_allow_json_includes_original_input() {
        let original_input = serde_json::json!({"command": "cargo test", "timeout": 30000});
        let msg = build_permission_response_json("req_p1", true, &original_input);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();

        assert_eq!(parsed["type"], "control_response");
        assert_eq!(parsed["response"]["subtype"], "success");
        assert_eq!(parsed["response"]["request_id"], "req_p1");

        let response = &parsed["response"]["response"];
        assert_eq!(response["behavior"], "allow");
        // updatedInput must contain the original tool arguments
        assert_eq!(response["updatedInput"]["command"], "cargo test");
        assert_eq!(response["updatedInput"]["timeout"], 30000);
    }

    /// Verify that permission allow with empty `original_input` still sends empty object.
    #[test]
    fn permission_allow_json_with_empty_input() {
        let original_input = serde_json::json!({});
        let msg = build_permission_response_json("req_p3", true, &original_input);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();

        let response = &parsed["response"]["response"];
        assert_eq!(response["behavior"], "allow");
        assert!(response["updatedInput"].is_object());
    }

    /// Verify the permission deny JSON includes interrupt and message.
    #[test]
    fn permission_deny_json_includes_message() {
        let original_input = serde_json::json!({"command": "rm -rf /"});
        let msg = build_permission_response_json("req_p2", false, &original_input);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();

        assert_eq!(parsed["type"], "control_response");
        assert_eq!(parsed["response"]["subtype"], "success");
        assert_eq!(parsed["response"]["request_id"], "req_p2");

        let response = &parsed["response"]["response"];
        assert_eq!(response["behavior"], "deny");
        assert_eq!(response["interrupt"], true);
        assert!(response["message"].is_string());
    }

    /// Verify the JSON format sent to the subprocess matches the protocol spec.
    #[test]
    fn question_response_json_format_matches_protocol() {
        let answers: HashMap<String, String> =
            [("Which database?".to_string(), "SQLite".to_string())]
                .into_iter()
                .collect();
        let original_input = serde_json::json!({
            "questions": [{"question": "Which database?", "options": [
                {"label": "PostgreSQL", "description": "Full-featured"},
                {"label": "SQLite", "description": "Embedded"}
            ], "multi_select": false}]
        });

        let msg = build_question_response_json("req_q1", &answers, &original_input);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();

        // Must be a control_response
        assert_eq!(parsed["type"], "control_response");
        assert_eq!(parsed["response"]["subtype"], "success");
        assert_eq!(parsed["response"]["request_id"], "req_q1");

        // Must have behavior: "allow" with updatedInput containing answers
        let response = &parsed["response"]["response"];
        assert_eq!(response["behavior"], "allow");
        assert_eq!(
            response["updatedInput"]["answers"]["Which database?"],
            "SQLite"
        );

        // updatedInput must also preserve the original questions
        assert!(response["updatedInput"]["questions"].is_array());
    }

    /// Verify that RelayHandle session_grants and pending_permissions
    /// are properly initialized as empty.
    #[tokio::test]
    async fn relay_handle_session_grants_initialized_empty() {
        let (handle, _rx) = test_relay_handle();

        assert!(handle.session_grants.read().await.is_empty());
        assert!(handle.pending_permissions.read().await.is_empty());
    }

    /// Verify AllowSession populates session_grants via the handle.
    #[tokio::test]
    async fn allow_session_populates_grant() {
        use crate::relay::PendingPermission;
        let (handle, _rx) = test_relay_handle();

        // Simulate what the pipeline does: store PendingPermission
        handle
            .pending_permissions
            .write()
            .await
            .insert("req-1".into(), PendingPermission {
                input: serde_json::json!({"command": "ls"}),
                tool_name: "Bash".into(),
            });

        // Simulate AllowSession grant caching via process_permission_response
        let pending = handle
            .pending_permissions
            .write()
            .await
            .remove("req-1");
        let tool_name = pending.map(|p| p.tool_name);
        assert_eq!(tool_name, Some("Bash".to_string()));

        handle
            .session_grants
            .write()
            .await
            .insert(tool_name.unwrap(), true);

        // Verify grant is cached
        let grants = handle.session_grants.read().await;
        assert_eq!(grants.get("Bash"), Some(&true));
    }

    /// Verify AllowOnce does NOT populate session_grants.
    #[tokio::test]
    async fn allow_once_does_not_populate_grant() {
        use crate::relay::PendingPermission;
        let (handle, _rx) = test_relay_handle();

        // Set up pending state
        handle
            .pending_permissions
            .write()
            .await
            .insert("req-1".into(), PendingPermission {
                input: serde_json::json!({}),
                tool_name: "Bash".into(),
            });

        // AllowOnce: remove from pending but do NOT insert into session_grants
        let _ = handle
            .pending_permissions
            .write()
            .await
            .remove("req-1");

        // session_grants should remain empty
        assert!(handle.session_grants.read().await.is_empty());
    }

    /// Verify that session_grants auto-respond sends to stdin_tx when a grant exists.
    #[tokio::test]
    async fn session_grant_auto_respond_sends_to_stdin() {
        let (handle, mut rx) = test_relay_handle();

        // Add a grant for "Bash"
        handle
            .session_grants
            .write()
            .await
            .insert("Bash".into(), true);

        // Simulate auto-respond: check grant and send via stdin_tx
        let grant = handle.session_grants.read().await.get("Bash").copied();
        assert_eq!(grant, Some(true));

        let original_input = serde_json::json!({"command": "ls"});
        let line = build_permission_response_json("req-auto", true, &original_input);
        handle.stdin_tx.send(line.clone()).await.unwrap();

        // Verify the line was sent
        let received = rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&received).unwrap();
        assert_eq!(parsed["type"], "control_response");
        assert_eq!(parsed["response"]["response"]["behavior"], "allow");
        drop(handle);
    }

    /// Verify that with no matching grant, the permission request is
    /// stored in pending_permissions for the handler to process.
    #[tokio::test]
    async fn no_grant_stores_in_pending_permissions() {
        use crate::relay::PendingPermission;
        let (handle, _rx) = test_relay_handle();

        // No grants — simulate what pipeline does: store in pending_permissions
        let grant = handle.session_grants.read().await.get("Write").copied();
        assert!(grant.is_none());

        let input = serde_json::json!({"file": "/tmp/test.txt"});
        handle
            .pending_permissions
            .write()
            .await
            .insert("req-2".into(), PendingPermission {
                input: input.clone(),
                tool_name: "Write".into(),
            });

        // Verify stored
        let pending = handle.pending_permissions.read().await;
        let entry = pending.get("req-2").unwrap();
        assert_eq!(entry.input, input);
        assert_eq!(entry.tool_name, "Write");
    }

    /// Helper to create a `RelayHandle` with pre-populated pending permissions.
    async fn make_handle_with_pending(
        request_id: &str,
        tool_name: &str,
        original_input: serde_json::Value,
    ) -> RelayHandle {
        use crate::relay::PendingPermission;
        let (handle, _rx) = test_relay_handle();
        handle
            .pending_permissions
            .write()
            .await
            .insert(request_id.into(), PendingPermission {
                input: original_input,
                tool_name: tool_name.into(),
            });
        handle
    }

    /// AllowSession should cache the grant in session_grants and clean pending maps.
    #[tokio::test]
    async fn process_permission_allow_session_caches_grant() {
        let input = serde_json::json!({"command": "cargo test"});
        let handle = make_handle_with_pending("req-as", "Bash", input.clone()).await;

        let (granted, returned_input) = handle
            .process_permission_response(
                "req-as",
                betcode_proto::v1::PermissionDecision::AllowSession,
                "test",
            )
            .await;

        assert!(granted);
        assert_eq!(returned_input, input);
        // session_grants should contain the cached grant
        let grants = handle.session_grants.read().await;
        assert_eq!(grants.get("Bash"), Some(&true));
        // pending maps should be cleaned
        assert!(handle.pending_permissions.read().await.is_empty());
    }

    /// AllowOnce should NOT cache the grant in session_grants but should clean pending maps.
    #[tokio::test]
    async fn process_permission_allow_once_does_not_cache() {
        let input = serde_json::json!({"file": "/tmp/test.txt"});
        let handle = make_handle_with_pending("req-ao", "Write", input.clone()).await;

        let (granted, returned_input) = handle
            .process_permission_response(
                "req-ao",
                betcode_proto::v1::PermissionDecision::AllowOnce,
                "test",
            )
            .await;

        assert!(granted);
        assert_eq!(returned_input, input);
        // session_grants should be empty (AllowOnce does not cache)
        assert!(handle.session_grants.read().await.is_empty());
        // pending maps should be cleaned
        assert!(handle.pending_permissions.read().await.is_empty());
    }

    /// Deny should NOT cache the grant and should clean pending maps.
    #[tokio::test]
    async fn process_permission_deny_cleans_pending() {
        let input = serde_json::json!({"command": "rm -rf /"});
        let handle = make_handle_with_pending("req-d", "Bash", input.clone()).await;

        let (granted, returned_input) = handle
            .process_permission_response(
                "req-d",
                betcode_proto::v1::PermissionDecision::Deny,
                "test",
            )
            .await;

        assert!(!granted);
        assert_eq!(returned_input, input);
        // session_grants should be empty (Deny does not cache)
        assert!(handle.session_grants.read().await.is_empty());
        // pending maps should be cleaned
        assert!(handle.pending_permissions.read().await.is_empty());
    }

    /// AllowWithEdit should grant but NOT cache in session_grants, and should clean pending maps.
    #[tokio::test]
    async fn process_permission_allow_with_edit_grants_without_caching() {
        let input = serde_json::json!({"command": "cargo build"});
        let handle = make_handle_with_pending("req-awe", "Bash", input.clone()).await;

        let (granted, returned_input) = handle
            .process_permission_response(
                "req-awe",
                betcode_proto::v1::PermissionDecision::AllowWithEdit,
                "test",
            )
            .await;

        assert!(granted);
        assert_eq!(returned_input, input);
        // session_grants should be empty (AllowWithEdit does not cache — user wants to review each time)
        assert!(handle.session_grants.read().await.is_empty());
        // pending maps should be cleaned
        assert!(handle.pending_permissions.read().await.is_empty());
    }
}
