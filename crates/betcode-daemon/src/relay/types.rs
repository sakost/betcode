//! Relay module types.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use betcode_proto::v1::SessionGrantEntry;
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
    ///
    /// Three-state semantics (keyed by tool name):
    /// - `Some(true)`  — auto-allow: skip the permission prompt, grant immediately.
    /// - `Some(false)` — auto-deny: skip the permission prompt, deny immediately.
    /// - `None` (absent) — no cached decision; prompt the user as normal.
    ///
    /// Written by the handler on `AllowSession` (sets `true`) or
    /// `SetSessionGrant` API (can set `true` **or** `false`).
    /// Read by the stdout pipeline to auto-respond to subsequent matching
    /// permission requests without forwarding them to the client.
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
        source: &str,
    ) -> (bool, serde_json::Value) {
        let granted = is_granted(decision);

        let input = self
            .pending_permission_inputs
            .write()
            .await
            .remove(request_id)
            .unwrap_or_else(|| serde_json::json!({}));

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
                    source,
                    "Cached AllowSession grant"
                );
            }
        }

        (granted, input)
    }

    /// List all session grants as proto entries.
    pub async fn list_grants(&self) -> Vec<SessionGrantEntry> {
        self.session_grants
            .read()
            .await
            .iter()
            .map(|(tool_name, granted)| SessionGrantEntry {
                tool_name: tool_name.clone(),
                granted: *granted,
            })
            .collect()
    }

    /// Clear session grants. If `tool_name` is empty, clears all.
    /// Returns the tool names that were cleared.
    pub async fn clear_grants(&self, tool_name: &str) -> Vec<String> {
        let mut grants = self.session_grants.write().await;
        if tool_name.is_empty() {
            let names: Vec<String> = grants.keys().cloned().collect();
            grants.clear();
            drop(grants);
            names
        } else {
            grants.remove(tool_name);
            drop(grants);
            vec![tool_name.to_string()]
        }
    }

    /// Set a session grant for a tool.
    pub async fn set_grant(&self, tool_name: String, granted: bool) {
        self.session_grants.write().await.insert(tool_name, granted);
    }
}

/// Returns `true` if the given [`PermissionDecision`] grants the requested permission.
pub const fn is_granted(decision: betcode_proto::v1::PermissionDecision) -> bool {
    matches!(
        decision,
        betcode_proto::v1::PermissionDecision::AllowOnce
            | betcode_proto::v1::PermissionDecision::AllowSession
            | betcode_proto::v1::PermissionDecision::AllowWithEdit
    )
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
