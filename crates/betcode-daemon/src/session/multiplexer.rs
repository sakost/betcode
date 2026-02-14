//! Session multiplexer for multi-client event fan-out.
//!
//! Each session can have multiple connected clients. Events from the Claude
//! subprocess are broadcast to all subscribed clients. Only one client can
//! hold the input lock at a time to send messages.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, info, warn};

use betcode_proto::v1::AgentEvent;

use super::state::SessionState;
use super::types::{
    ClientHandle, InputLockResult, MultiplexerConfig, MultiplexerError, MultiplexerStats,
};

/// Session multiplexer manages multiple client connections to sessions.
pub struct SessionMultiplexer {
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
    config: MultiplexerConfig,
}

impl SessionMultiplexer {
    /// Create a new session multiplexer.
    pub fn new(config: MultiplexerConfig) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(MultiplexerConfig::default())
    }

    /// Get or create a session.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn get_or_create_session(&self, session_id: &str) -> broadcast::Sender<AgentEvent> {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get(session_id) {
            return session.event_tx.clone();
        }

        let session = SessionState::new(session_id.to_string(), self.config.broadcast_capacity);
        let tx = session.event_tx.clone();
        sessions.insert(session_id.to_string(), session);
        drop(sessions);

        info!(session_id, "Created new session");
        tx
    }

    /// Subscribe a client to a session.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn subscribe(
        &self,
        session_id: &str,
        client_id: &str,
        client_type: &str,
    ) -> Result<ClientHandle, MultiplexerError> {
        let mut sessions = self.sessions.write().await;

        let session = sessions.entry(session_id.to_string()).or_insert_with(|| {
            SessionState::new(session_id.to_string(), self.config.broadcast_capacity)
        });

        if session.client_count() >= self.config.max_clients {
            return Err(MultiplexerError::TooManyClients {
                session_id: session_id.to_string(),
                max: self.config.max_clients,
            });
        }

        if session.clients.contains_key(client_id) {
            return Err(MultiplexerError::ClientAlreadyConnected {
                client_id: client_id.to_string(),
            });
        }

        session.add_client(client_id.to_string(), client_type.to_string());
        let event_rx = session.event_tx.subscribe();

        info!(session_id, client_id, client_type, "Client subscribed");

        Ok(ClientHandle {
            client_id: client_id.to_string(),
            session_id: session_id.to_string(),
            event_rx,
            has_input_lock: false,
        })
    }

    /// Unsubscribe a client from a session.
    pub async fn unsubscribe(&self, session_id: &str, client_id: &str) {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get_mut(session_id)
            && session.remove_client(client_id)
        {
            info!(session_id, client_id, "Client unsubscribed");

            if session.is_empty() {
                debug!(session_id, "Removing empty session");
                sessions.remove(session_id);
            }
        }
    }

    /// Request input lock for a client.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn request_input_lock(
        &self,
        session_id: &str,
        client_id: &str,
    ) -> Result<InputLockResult, MultiplexerError> {
        let mut sessions = self.sessions.write().await;

        let session =
            sessions
                .get_mut(session_id)
                .ok_or_else(|| MultiplexerError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if !session.clients.contains_key(client_id) {
            return Err(MultiplexerError::ClientNotConnected {
                client_id: client_id.to_string(),
            });
        }

        if let Some(ref holder) = session.input_lock_holder {
            if holder == client_id {
                return Ok(InputLockResult {
                    granted: true,
                    previous_holder: None,
                });
            }
            return Ok(InputLockResult {
                granted: false,
                previous_holder: Some(holder.clone()),
            });
        }

        session.input_lock_holder = Some(client_id.to_string());
        if let Some(client) = session.clients.get_mut(client_id) {
            client.has_input_lock = true;
        }

        info!(session_id, client_id, "Input lock granted");

        Ok(InputLockResult {
            granted: true,
            previous_holder: None,
        })
    }

    /// Release input lock for a client.
    pub async fn release_input_lock(&self, session_id: &str, client_id: &str) {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get_mut(session_id)
            && session.input_lock_holder.as_deref() == Some(client_id)
        {
            session.input_lock_holder = None;
            if let Some(client) = session.clients.get_mut(client_id) {
                client.has_input_lock = false;
            }
            info!(session_id, client_id, "Input lock released");
        }
    }

    /// Broadcast an event to all clients in a session.
    pub async fn broadcast(&self, session_id: &str, mut event: AgentEvent) {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get_mut(session_id) {
            event.sequence = session.next_sequence();

            if let Ok(count) = session.event_tx.send(event) {
                debug!(session_id, receivers = count, "Event broadcast");
            } else {
                debug!(session_id, "No receivers for broadcast");
            }
        }
    }

    /// Update client heartbeat.
    pub async fn heartbeat(&self, session_id: &str, client_id: &str) {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get_mut(session_id)
            && let Some(client) = session.clients.get_mut(client_id)
        {
            client.last_heartbeat = std::time::Instant::now();
        }
    }

    /// Remove stale clients that haven't sent heartbeats.
    pub async fn cleanup_stale_clients(&self) -> Vec<(String, String)> {
        let mut sessions = self.sessions.write().await;
        let mut removed = Vec::new();

        for (session_id, session) in sessions.iter_mut() {
            let stale: Vec<(String, String)> = session
                .clients
                .iter()
                .filter(|(_, c)| c.last_heartbeat.elapsed() > self.config.heartbeat_timeout)
                .map(|(_, c)| (c.client_id.clone(), c.client_type.clone()))
                .collect();

            for (client_id, client_type) in stale {
                session.remove_client(&client_id);
                warn!(session_id, client_id, client_type, "Removed stale client");
                removed.push((session_id.clone(), client_id));
            }
        }

        sessions.retain(|_, session| !session.is_empty());
        removed
    }

    /// Get session statistics.
    pub async fn stats(&self) -> MultiplexerStats {
        let sessions = self.sessions.read().await;
        let total_clients: usize = sessions
            .values()
            .map(super::state::SessionState::client_count)
            .sum();

        MultiplexerStats {
            session_count: sessions.len(),
            total_clients,
        }
    }

    /// Check if a client holds the input lock.
    pub async fn has_input_lock(&self, session_id: &str, client_id: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .is_some_and(|s| s.input_lock_holder.as_deref() == Some(client_id))
    }

    /// Create a sender channel for forwarding subprocess events.
    ///
    /// Events forwarded through this channel are assigned sequence numbers
    /// before being broadcast, ensuring correct ordering for all clients.
    pub fn create_event_forwarder(&self, session_id: String) -> mpsc::Sender<AgentEvent> {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(128);
        let sessions = Arc::clone(&self.sessions);

        tokio::spawn(async move {
            while let Some(mut event) = rx.recv().await {
                let mut sessions = sessions.write().await;
                if let Some(session) = sessions.get_mut(&session_id) {
                    event.sequence = session.next_sequence();
                    let _ = session.event_tx.send(event);
                }
            }
        });

        tx
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_multiplexer() {
        let mux = SessionMultiplexer::with_defaults();
        let stats = mux.stats().await;
        assert_eq!(stats.session_count, 0);
        assert_eq!(stats.total_clients, 0);
    }

    #[tokio::test]
    async fn subscribe_and_unsubscribe() {
        let mux = SessionMultiplexer::with_defaults();

        let handle = mux.subscribe("session-1", "client-1", "cli").await.unwrap();
        assert_eq!(handle.session_id, "session-1");
        assert_eq!(handle.client_id, "client-1");

        let stats = mux.stats().await;
        assert_eq!(stats.session_count, 1);
        assert_eq!(stats.total_clients, 1);

        mux.unsubscribe("session-1", "client-1").await;

        let stats = mux.stats().await;
        assert_eq!(stats.session_count, 0);
        assert_eq!(stats.total_clients, 0);
    }

    #[tokio::test]
    async fn input_lock_management() {
        let mux = SessionMultiplexer::with_defaults();

        mux.subscribe("session-1", "client-1", "cli").await.unwrap();
        mux.subscribe("session-1", "client-2", "app").await.unwrap();

        let result = mux
            .request_input_lock("session-1", "client-1")
            .await
            .unwrap();
        assert!(result.granted);

        let result = mux
            .request_input_lock("session-1", "client-2")
            .await
            .unwrap();
        assert!(!result.granted);
        assert_eq!(result.previous_holder, Some("client-1".to_string()));

        mux.release_input_lock("session-1", "client-1").await;

        let result = mux
            .request_input_lock("session-1", "client-2")
            .await
            .unwrap();
        assert!(result.granted);
    }

    #[tokio::test]
    async fn max_clients_enforced() {
        let config = MultiplexerConfig {
            max_clients: 2,
            ..Default::default()
        };
        let mux = SessionMultiplexer::new(config);

        mux.subscribe("session-1", "client-1", "cli").await.unwrap();
        mux.subscribe("session-1", "client-2", "cli").await.unwrap();

        let result = mux.subscribe("session-1", "client-3", "cli").await;
        assert!(matches!(
            result,
            Err(MultiplexerError::TooManyClients { .. })
        ));
    }
}
