//! Session state management.

use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::broadcast;

use betcode_proto::v1::AgentEvent;

use super::types::ClientState;

/// Session state with connected clients.
pub(crate) struct SessionState {
    /// Session identifier.
    #[allow(dead_code)] // Used for logging and future APIs
    pub session_id: String,
    /// Broadcast sender for events.
    pub event_tx: broadcast::Sender<AgentEvent>,
    /// Connected clients.
    pub clients: HashMap<String, ClientState>,
    /// Client that currently holds input lock (if any).
    pub input_lock_holder: Option<String>,
    /// Event sequence counter.
    sequence: u64,
}

impl SessionState {
    pub fn new(session_id: String, broadcast_capacity: usize) -> Self {
        let (event_tx, _) = broadcast::channel(broadcast_capacity);
        Self {
            session_id,
            event_tx,
            clients: HashMap::new(),
            input_lock_holder: None,
            sequence: 0,
        }
    }

    pub fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    pub fn add_client(&mut self, client_id: String, client_type: String) {
        let client_state = ClientState {
            client_id: client_id.clone(),
            client_type,
            last_heartbeat: Instant::now(),
            has_input_lock: false,
        };
        self.clients.insert(client_id, client_state);
    }

    pub fn remove_client(&mut self, client_id: &str) -> bool {
        if self.clients.remove(client_id).is_some() {
            // Release input lock if this client held it
            if self.input_lock_holder.as_deref() == Some(client_id) {
                self.input_lock_holder = None;
            }
            true
        } else {
            false
        }
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}
