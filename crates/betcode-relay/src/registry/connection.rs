//! In-memory connection registry for tunnel management.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{info, warn};

use betcode_proto::v1::TunnelFrame;

/// Holds an active tunnel connection to a daemon.
pub struct TunnelConnection {
    /// Machine ID this connection belongs to.
    pub machine_id: String,
    /// User who owns this machine.
    pub owner_id: String,
    /// Sender for pushing frames to the daemon through the tunnel.
    pub frame_tx: mpsc::Sender<TunnelFrame>,
    /// Pending response waiters keyed by request_id.
    pub pending: Arc<RwLock<HashMap<String, oneshot::Sender<TunnelFrame>>>>,
}

impl TunnelConnection {
    pub fn new(machine_id: String, owner_id: String, frame_tx: mpsc::Sender<TunnelFrame>) -> Self {
        Self {
            machine_id,
            owner_id,
            frame_tx,
            pending: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Send a frame to the daemon through the tunnel.
    pub async fn send_frame(
        &self,
        frame: TunnelFrame,
    ) -> Result<(), mpsc::error::SendError<TunnelFrame>> {
        self.frame_tx.send(frame).await
    }

    /// Register a pending request and return a receiver for the response.
    pub async fn register_pending(&self, request_id: String) -> oneshot::Receiver<TunnelFrame> {
        let (tx, rx) = oneshot::channel();
        self.pending.write().await.insert(request_id, tx);
        rx
    }

    /// Complete a pending request with a response frame.
    pub async fn complete_pending(&self, request_id: &str, frame: TunnelFrame) -> bool {
        if let Some(tx) = self.pending.write().await.remove(request_id) {
            tx.send(frame).is_ok()
        } else {
            false
        }
    }

    /// Cancel all pending requests.
    pub async fn cancel_all_pending(&self) {
        self.pending.write().await.clear();
    }
}

/// Thread-safe registry of active tunnel connections.
#[derive(Clone)]
pub struct ConnectionRegistry {
    connections: Arc<RwLock<HashMap<String, Arc<TunnelConnection>>>>,
}

impl ConnectionRegistry {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a tunnel connection for a machine.
    pub async fn register(
        &self,
        machine_id: String,
        owner_id: String,
        frame_tx: mpsc::Sender<TunnelFrame>,
    ) -> Arc<TunnelConnection> {
        let conn = Arc::new(TunnelConnection::new(
            machine_id.clone(),
            owner_id,
            frame_tx,
        ));
        self.connections
            .write()
            .await
            .insert(machine_id.clone(), Arc::clone(&conn));
        info!(machine_id = %machine_id, "Tunnel connection registered");
        conn
    }

    /// Remove a tunnel connection.
    pub async fn unregister(&self, machine_id: &str) -> Option<Arc<TunnelConnection>> {
        let conn = self.connections.write().await.remove(machine_id);
        if conn.is_some() {
            info!(machine_id = %machine_id, "Tunnel connection unregistered");
        } else {
            warn!(machine_id = %machine_id, "Tried to unregister unknown connection");
        }
        conn
    }

    /// Get a tunnel connection by machine ID.
    pub async fn get(&self, machine_id: &str) -> Option<Arc<TunnelConnection>> {
        self.connections.read().await.get(machine_id).cloned()
    }

    /// Check if a machine is connected.
    pub async fn is_connected(&self, machine_id: &str) -> bool {
        self.connections.read().await.contains_key(machine_id)
    }

    /// Get all connected machine IDs.
    pub async fn connected_machines(&self) -> Vec<String> {
        self.connections.read().await.keys().cloned().collect()
    }

    /// Count of active connections.
    pub async fn connection_count(&self) -> usize {
        self.connections.read().await.len()
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_and_get_connection() {
        let registry = ConnectionRegistry::new();
        let (tx, _rx) = mpsc::channel(16);

        registry.register("m1".into(), "u1".into(), tx).await;

        assert!(registry.is_connected("m1").await);
        assert!(!registry.is_connected("m2").await);

        let conn = registry.get("m1").await.unwrap();
        assert_eq!(conn.machine_id, "m1");
        assert_eq!(conn.owner_id, "u1");
    }

    #[tokio::test]
    async fn unregister_connection() {
        let registry = ConnectionRegistry::new();
        let (tx, _rx) = mpsc::channel(16);

        registry.register("m1".into(), "u1".into(), tx).await;
        assert_eq!(registry.connection_count().await, 1);

        let removed = registry.unregister("m1").await;
        assert!(removed.is_some());
        assert_eq!(registry.connection_count().await, 0);
        assert!(!registry.is_connected("m1").await);
    }

    #[tokio::test]
    async fn pending_request_lifecycle() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);

        let response_rx = conn.register_pending("req-1".into()).await;

        let response_frame = TunnelFrame {
            request_id: "req-1".into(),
            ..Default::default()
        };

        assert!(conn.complete_pending("req-1", response_frame).await);

        let received = response_rx.await.unwrap();
        assert_eq!(received.request_id, "req-1");
    }

    #[tokio::test]
    async fn complete_unknown_pending_returns_false() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);

        let frame = TunnelFrame::default();
        assert!(!conn.complete_pending("nonexistent", frame).await);
    }

    #[tokio::test]
    async fn connected_machines_list() {
        let registry = ConnectionRegistry::new();
        let (tx1, _) = mpsc::channel(16);
        let (tx2, _) = mpsc::channel(16);

        registry.register("m1".into(), "u1".into(), tx1).await;
        registry.register("m2".into(), "u1".into(), tx2).await;

        let mut machines = registry.connected_machines().await;
        machines.sort();
        assert_eq!(machines, vec!["m1", "m2"]);
    }
}
