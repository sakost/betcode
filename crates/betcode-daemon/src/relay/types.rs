//! Relay module types.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use tokio::sync::RwLock;

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
    /// Pending AskUserQuestion original inputs keyed by request_id.
    /// Written by the stdout pipeline bridge, read by the handler to
    /// build the correct `control_response` with `updatedInput`.
    pub pending_question_inputs: Arc<RwLock<HashMap<String, serde_json::Value>>>,
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
