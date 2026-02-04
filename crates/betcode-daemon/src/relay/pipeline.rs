//! Session relay pipeline: subprocess ↔ gRPC event bridging.
//!
//! Data flow:
//! ```text
//! subprocess stdout → NDJSON parser → EventBridge → event_forwarder → broadcast
//! gRPC UserMessage → JSON → subprocess stdin
//! gRPC PermissionResponse → control_response JSON → subprocess stdin
//! ```

use std::collections::HashMap;
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

        let relay_handle = RelayHandle {
            process_id: process_handle.id.clone(),
            session_id: session_id.clone(),
            stdin_tx: process_handle.stdin_tx.clone(),
        };

        // Spawn the NDJSON reader pipeline
        spawn_stdout_pipeline(
            session_id.clone(),
            stdout_rx,
            event_forwarder,
            self.db.clone(),
            Arc::clone(&self.sessions),
            Arc::clone(&self.subprocess_manager),
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
    pub async fn send_user_message(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<(), RelayError> {
        let handle = self.get_active_handle(session_id).await?;

        let msg = serde_json::json!({
            "type": "user_message",
            "content": content,
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

        debug!(session_id, "Sent user message to subprocess");
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

    /// Cancel the current turn for a session.
    pub async fn cancel_session(&self, session_id: &str) -> Result<bool, RelayError> {
        let process_id = match self.sessions.read().await.get(session_id) {
            Some(h) => h.process_id.clone(),
            None => return Ok(false),
        };

        match self.subprocess_manager.terminate(&process_id).await {
            Ok(()) => {
                self.sessions.write().await.remove(session_id);
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
) {
    tokio::spawn(async move {
        let mut bridge = EventBridge::new();
        let sid = session_id.clone();

        while let Some(line) = stdout_rx.recv().await {
            let msg = match ndjson::parse_line(&line) {
                Ok(msg) => msg,
                Err(e) => {
                    warn!(session_id = %sid, error = %e, "NDJSON parse error");
                    continue;
                }
            };

            let events = bridge.convert(msg);

            for event in events {
                if let Err(e) = store_event(&db, &sid, &event).await {
                    warn!(session_id = %sid, error = %e, "Failed to store event");
                }
                if event_forwarder.send(event).await.is_err() {
                    warn!(session_id = %sid, "Event forwarder closed");
                    return;
                }
            }

            // Update subprocess with Claude's session ID from SystemInit
            if let Some(info) = bridge.session_info() {
                if let Some(handle) = sessions.write().await.get_mut(&sid) {
                    let _ = subprocess_manager
                        .set_session_id(&handle.process_id, info.session_id.clone())
                        .await;
                }
            }
        }

        info!(session_id = %sid, "Stdout pipeline finished");
        sessions.write().await.remove(&sid);
    });
}

/// Store an event in the database for replay support.
async fn store_event(db: &Database, session_id: &str, event: &AgentEvent) -> Result<(), String> {
    use prost::Message;
    let mut buf = Vec::new();
    event.encode(&mut buf).map_err(|e| e.to_string())?;

    let payload = String::from_utf8_lossy(&buf).to_string();

    db.insert_message(session_id, event.sequence as i64, "agent_event", &payload)
        .await
        .map_err(|e| e.to_string())
        .map(|_| ())
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
