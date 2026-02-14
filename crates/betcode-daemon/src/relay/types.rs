//! Relay module types.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::info;

/// Configuration for starting a relay session.
#[derive(Debug, Clone)]
pub struct RelaySessionConfig {
    /// Session ID.
    pub session_id: String,
    /// Working directory for the Claude subprocess.
    pub working_directory: PathBuf,
    /// Model to use.
    pub model: Option<String>,
    /// Session to resume (if any).
    pub resume_session: Option<String>,
    /// Worktree ID for multi-worktree support.
    pub worktree_id: String,
}

/// Handle returned after a relay session is started.
/// Used to send messages and permissions to the subprocess.
#[derive(Debug, Clone)]
pub struct RelayHandle {
    /// The subprocess process ID.
    pub process_id: String,
    /// Session ID.
    pub session_id: String,
    /// Sender for stdin lines to the subprocess.
    pub stdin_tx: tokio::sync::mpsc::Sender<String>,
    /// Shared sequence counter for interleaving user input and agent events.
    pub sequence_counter: Arc<AtomicU64>,
    /// Pending `AskUserQuestion` original inputs keyed by `request_id`.
    /// Written by the stdout pipeline bridge, read by the handler to
    /// build the correct `control_response` with `updatedInput`.
    pub pending_question_inputs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    /// Pending permission (`CanUseTool`) original inputs keyed by `request_id`.
    /// Written by the stdout pipeline bridge, read by the handler to
    /// build the correct `control_response` with `updatedInput`.
    pub pending_permission_inputs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    /// Session-scoped permission grants keyed by `tool_name` → granted.
    /// Written by the handler on `AllowSession`, read by the stdout pipeline
    /// to auto-respond to subsequent matching permission requests.
    pub session_grants: Arc<RwLock<HashMap<String, bool>>>,
    /// Maps `request_id` → `tool_name` for pending permission requests.
    /// Used by the handler to look up which tool was granted when
    /// processing `AllowSession` decisions.
    pub pending_permission_tool_names: Arc<RwLock<HashMap<String, String>>>,
}

impl RelayHandle {
    /// Process a permission response: remove pending entries and cache `AllowSession` grants.
    /// Returns `(granted, original_input)`.
    pub async fn process_permission_response(
        &self,
        request_id: &str,
        decision: betcode_proto::v1::PermissionDecision,
    ) -> (bool, serde_json::Value) {
        let granted = matches!(
            decision,
            betcode_proto::v1::PermissionDecision::AllowOnce
                | betcode_proto::v1::PermissionDecision::AllowSession
        );

        let input = self
            .pending_permission_inputs
            .write()
            .await
            .remove(request_id)
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::default()));

        let tool = self
            .pending_permission_tool_names
            .write()
            .await
            .remove(request_id);

        // On AllowSession, cache the grant for this tool
        if decision == betcode_proto::v1::PermissionDecision::AllowSession {
            if let Some(ref tool_name) = tool {
                self.session_grants
                    .write()
                    .await
                    .insert(tool_name.clone(), true);
                info!(
                    session_id = %self.session_id,
                    tool_name = %tool_name,
                    "Cached AllowSession grant"
                );
            }
        }

        (granted, input)
    }
}

/// Errors from relay operations.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("Subprocess error: {0}")]
    Subprocess(#[from] crate::subprocess::SubprocessError),

    #[error("Session not found: {session_id}")]
    SessionNotFound { session_id: String },

    #[error("Failed to serialize message: {0}")]
    Serialization(String),

    #[error("Subprocess stdin closed for session: {session_id}")]
    StdinClosed { session_id: String },

    #[error("Storage error: {0}")]
    Storage(String),
}
