//! Session relay pipeline: subprocess ↔ gRPC event bridging.
//!
//! Data flow:
//! ```text
//! subprocess stdout → NDJSON parser → EventBridge → event_forwarder → broadcast
//! gRPC UserMessage → JSON → subprocess stdin
//! gRPC PermissionResponse → control_response JSON → subprocess stdin
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use betcode_core::ndjson;
use betcode_proto::v1::AgentEvent;

use crate::session::SessionMultiplexer;
use crate::storage::Database;
use crate::subprocess::{EventBridge, SpawnConfig, SubprocessManager};

use super::types::*;

/// Session relay manages the lifecycle of subprocess ↔ gRPC bridging.
pub struct SessionRelay {
    subprocess_manager: Arc<SubprocessManager>,
    multiplexer: Arc<SessionMultiplexer>,
    db: Database,
    /// Maps session_id → RelayHandle for active sessions.
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
    /// NDJSON → EventBridge → Multiplexer pipeline.
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
        };

        let process_handle = self
            .subprocess_manager
            .spawn(spawn_config, stdout_tx)
            .await?;

        // Shared sequence counter between stdout pipeline and user message storage.
        // Initialized from DB max so resumed sessions don't collide.
        let start_seq = self
            .db
            .max_message_sequence(&session_id)
            .await
            .unwrap_or(0) as u64;
        let sequence_counter = Arc::new(AtomicU64::new(start_seq));

        let relay_handle = RelayHandle {
            process_id: process_handle.id.clone(),
            session_id: session_id.clone(),
            stdin_tx: process_handle.stdin_tx.clone(),
            sequence_counter: Arc::clone(&sequence_counter),
        };

        // Spawn the NDJSON reader pipeline
        spawn_stdout_pipeline(
            session_id.clone(),
            stdout_rx,
            event_forwarder,
            self.db.clone(),
            Arc::clone(&self.sessions),
            Arc::clone(&self.subprocess_manager),
            sequence_counter,
        );

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
    pub async fn send_user_message(
        &self,
        session_id: &str,
        content: &str,
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
        let msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": content,
            },
            "session_id": "default",
            "parent_tool_use_id": null,
        });
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

    /// Send a permission response (control_response) to the subprocess.
    pub async fn send_permission_response(
        &self,
        session_id: &str,
        request_id: &str,
        granted: bool,
    ) -> Result<(), RelayError> {
        let handle = self.get_active_handle(session_id).await?;
        let behavior = if granted { "allow" } else { "deny" };

        let msg = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": { "behavior": behavior }
            }
        });
        let line =
            serde_json::to_string(&msg).map_err(|e| RelayError::Serialization(e.to_string()))?;

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

/// Spawn the stdout → NDJSON parser → EventBridge → forwarder pipeline.
fn spawn_stdout_pipeline(
    session_id: String,
    mut stdout_rx: mpsc::Receiver<String>,
    event_forwarder: mpsc::Sender<AgentEvent>,
    db: Database,
    sessions: Arc<RwLock<HashMap<String, RelayHandle>>>,
    subprocess_manager: Arc<SubprocessManager>,
    sequence_counter: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let sid = session_id.clone();
        // Read the shared counter (may have been advanced by send_user_message).
        let start_seq = sequence_counter.load(Ordering::Acquire);
        let mut bridge = EventBridge::with_start_sequence(start_seq);
        let mut event_count = 0u64;

        while let Some(line) = stdout_rx.recv().await {
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

            for event in events {
                event_count += 1;

                // Update usage stats in DB when we get a UsageReport
                if let Some(betcode_proto::v1::agent_event::Event::Usage(ref usage)) = event.event {
                    if let Err(e) = db
                        .update_session_usage(
                            &sid,
                            usage.input_tokens as i64,
                            usage.output_tokens as i64,
                            usage.cost_usd,
                        )
                        .await
                    {
                        warn!(session_id = %sid, error = %e, "Failed to update usage");
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

            // Update subprocess and DB with Claude's session ID from SystemInit
            if let Some(info) = bridge.session_info() {
                if let Some(handle) = sessions.write().await.get_mut(&sid) {
                    let _ = subprocess_manager
                        .set_session_id(&handle.process_id, info.session_id.clone())
                        .await;
                }
                // Persist Claude session ID for future resume operations
                if let Err(e) = db.update_claude_session_id(&sid, &info.session_id).await {
                    warn!(session_id = %sid, error = %e, "Failed to update claude_session_id");
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
                        details: Default::default(),
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

/// Determine the message_type string from an AgentEvent for DB storage.
fn event_message_type(event: &AgentEvent) -> &'static str {
    use betcode_proto::v1::agent_event::Event;
    match &event.event {
        Some(Event::TextDelta(_)) => "stream_event",
        Some(Event::ToolCallStart(_)) => "stream_event",
        Some(Event::ToolCallResult(_)) => "result",
        Some(Event::PermissionRequest(_)) => "control_request",
        Some(Event::StatusChange(_)) => "stream_event",
        Some(Event::SessionInfo(_)) => "system",
        Some(Event::Error(_)) => "stream_event",
        Some(Event::Usage(_)) => "result",
        Some(Event::TurnComplete(_)) => "result",
        Some(Event::UserQuestion(_)) => "control_request",
        Some(Event::TodoUpdate(_)) => "stream_event",
        Some(Event::PlanMode(_)) => "stream_event",
        Some(Event::UserInput(_)) => "user",
        None => "stream_event",
    }
}

/// Store an event in the database for replay support.
/// Events are serialized as JSON for readable storage and reliable deserialization.
async fn store_event(db: &Database, session_id: &str, event: &AgentEvent) -> Result<(), String> {
    use prost::Message;

    // Encode to protobuf bytes, then base64 for safe text storage
    let mut buf = Vec::new();
    event.encode(&mut buf).map_err(|e| e.to_string())?;
    let payload = base64_encode(&buf);
    let msg_type = event_message_type(event);

    db.insert_message(session_id, event.sequence as i64, msg_type, &payload)
        .await
        .map_err(|e| e.to_string())
        .map(|_| ())
}

/// Simple base64 encoding (no external dependency needed).
fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;

        let _ = result.write_char(CHARS[(n >> 18 & 0x3F) as usize] as char);
        let _ = result.write_char(CHARS[(n >> 12 & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            let _ = result.write_char(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            let _ = result.write_char(CHARS[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let result = relay.send_user_message("nonexistent", "hello").await;
        assert!(matches!(result, Err(RelayError::SessionNotFound { .. })));
    }

    #[tokio::test]
    async fn send_permission_fails_for_unknown_session() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = SessionRelay::new(subprocess_mgr, multiplexer, db);
        let result = relay
            .send_permission_response("nonexistent", "req-1", true)
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
}
