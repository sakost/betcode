//! Session multiplexer types.

use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use betcode_proto::v1::AgentEvent;

/// Configuration for session multiplexer.
#[derive(Debug, Clone)]
pub struct MultiplexerConfig {
    /// Maximum clients per session.
    pub max_clients: usize,
    /// Event broadcast channel capacity.
    pub broadcast_capacity: usize,
    /// Heartbeat timeout for client connections.
    pub heartbeat_timeout: Duration,
}

impl Default for MultiplexerConfig {
    fn default() -> Self {
        Self {
            max_clients: 5,
            broadcast_capacity: 256,
            heartbeat_timeout: Duration::from_secs(30),
        }
    }
}

/// Handle to a connected client.
#[derive(Debug)]
pub struct ClientHandle {
    /// Unique client identifier.
    pub client_id: String,
    /// Session this client is connected to.
    pub session_id: String,
    /// Receiver for broadcast events.
    pub event_rx: broadcast::Receiver<AgentEvent>,
    /// Whether this client holds the input lock.
    pub has_input_lock: bool,
}

/// Client state tracked by the multiplexer.
pub(crate) struct ClientState {
    pub client_id: String,
    pub client_type: String,
    pub last_heartbeat: Instant,
    pub has_input_lock: bool,
}

/// Result of input lock request.
#[derive(Debug)]
pub struct InputLockResult {
    /// Whether the lock was granted.
    pub granted: bool,
    /// Previous lock holder (if denied).
    pub previous_holder: Option<String>,
}

/// Multiplexer statistics.
#[derive(Debug)]
pub struct MultiplexerStats {
    /// Number of active sessions.
    pub session_count: usize,
    /// Total connected clients across all sessions.
    pub total_clients: usize,
}

/// Multiplexer errors.
#[derive(Debug, thiserror::Error)]
pub enum MultiplexerError {
    #[error("Session not found: {session_id}")]
    SessionNotFound { session_id: String },

    #[error("Too many clients for session {session_id} (max: {max})")]
    TooManyClients { session_id: String, max: usize },

    #[error("Client already connected: {client_id}")]
    ClientAlreadyConnected { client_id: String },

    #[error("Client not connected: {client_id}")]
    ClientNotConnected { client_id: String },
}
