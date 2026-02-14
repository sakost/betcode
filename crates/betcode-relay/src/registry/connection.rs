//! In-memory connection registry for tunnel management.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{debug, info, warn};

use betcode_proto::v1::TunnelFrame;

/// Holds an active tunnel connection to a daemon.
pub struct TunnelConnection {
    /// Machine ID this connection belongs to.
    pub machine_id: String,
    /// User who owns this machine.
    pub owner_id: String,
    /// Sender for pushing frames to the daemon through the tunnel.
    pub frame_tx: mpsc::Sender<TunnelFrame>,
    /// Pending response waiters for unary requests (single response per `request_id`).
    pub pending: Arc<RwLock<HashMap<String, oneshot::Sender<TunnelFrame>>>>,
    /// Pending stream channels for streaming requests (multiple frames per `request_id`).
    stream_pending: Arc<RwLock<HashMap<String, mpsc::Sender<TunnelFrame>>>>,
    /// Request IDs of streams whose receivers were dropped (client disconnected).
    /// Used to silently drop subsequent frames instead of warning.
    cancelled_streams: Arc<RwLock<HashSet<String>>>,
}

impl TunnelConnection {
    pub fn new(machine_id: String, owner_id: String, frame_tx: mpsc::Sender<TunnelFrame>) -> Self {
        Self {
            machine_id,
            owner_id,
            frame_tx,
            pending: Arc::new(RwLock::new(HashMap::new())),
            stream_pending: Arc::new(RwLock::new(HashMap::new())),
            cancelled_streams: Arc::new(RwLock::new(HashSet::new())),
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
        let tx = self.pending.write().await.remove(request_id);
        tx.is_some_and(|tx| tx.send(frame).is_ok())
    }

    // --- Stream pending (mpsc) ---

    /// Register a streaming pending request and return a receiver for multiple frames.
    pub async fn register_stream_pending(&self, request_id: String) -> mpsc::Receiver<TunnelFrame> {
        let (tx, rx) = mpsc::channel(128);
        self.stream_pending.write().await.insert(request_id, tx);
        rx
    }

    /// Send a frame to a streaming pending channel.
    /// Returns true if delivered, false if no stream channel or receiver dropped.
    /// Automatically cleans up dead senders when the receiver has been dropped.
    pub async fn send_stream_frame(&self, request_id: &str, frame: TunnelFrame) -> bool {
        let send_failed = {
            let guard = self.stream_pending.read().await;
            if let Some(tx) = guard.get(request_id) {
                if tx.send(frame).await.is_ok() {
                    return true;
                }
                true // entry exists but receiver dropped
            } else {
                false // no entry at all
            }
        };

        if send_failed {
            self.stream_pending.write().await.remove(request_id);
            self.cancelled_streams
                .write()
                .await
                .insert(request_id.to_string());
            debug!(request_id = %request_id, "Cleaned up dead stream sender (receiver dropped)");
        }
        false
    }

    /// Close a streaming channel by removing the sender.
    pub async fn complete_stream(&self, request_id: &str) -> bool {
        self.stream_pending
            .write()
            .await
            .remove(request_id)
            .is_some()
    }

    /// Check if a `request_id` has an active stream channel.
    pub async fn has_stream_pending(&self, request_id: &str) -> bool {
        self.stream_pending.read().await.contains_key(request_id)
    }

    /// Check if a stream was cancelled (receiver dropped by client disconnect).
    pub async fn is_cancelled_stream(&self, request_id: &str) -> bool {
        self.cancelled_streams.read().await.contains(request_id)
    }

    /// Remove a `request_id` from the cancelled set (e.g. after `StreamEnd` cleanup).
    pub async fn clear_cancelled_stream(&self, request_id: &str) {
        self.cancelled_streams.write().await.remove(request_id);
    }

    // --- Shared ---

    /// Cancel all pending requests (both unary and streaming).
    pub async fn cancel_all_pending(&self) {
        self.pending.write().await.clear();
        self.stream_pending.write().await.clear();
        self.cancelled_streams.write().await.clear();
        debug!(machine_id = %self.machine_id, "All pending requests cancelled");
    }

    /// Count of active unary pending requests.
    pub async fn pending_count(&self) -> usize {
        self.pending.read().await.len()
    }

    /// Count of active streaming pending requests.
    pub async fn stream_pending_count(&self) -> usize {
        self.stream_pending.read().await.len()
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
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use betcode_proto::v1::FrameType;

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

    #[tokio::test]
    async fn stream_pending_register_and_receive() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);
        let mut stream_rx = conn.register_stream_pending("req-s1".into()).await;
        assert!(conn.has_stream_pending("req-s1").await);
        assert_eq!(conn.stream_pending_count().await, 1);

        let f1 = TunnelFrame {
            request_id: "req-s1".into(),
            frame_type: FrameType::StreamData as i32,
            ..Default::default()
        };
        let f2 = TunnelFrame {
            request_id: "req-s1".into(),
            frame_type: FrameType::StreamData as i32,
            ..Default::default()
        };
        assert!(conn.send_stream_frame("req-s1", f1).await);
        assert!(conn.send_stream_frame("req-s1", f2).await);

        assert_eq!(stream_rx.recv().await.unwrap().request_id, "req-s1");
        assert_eq!(stream_rx.recv().await.unwrap().request_id, "req-s1");
    }

    #[tokio::test]
    async fn stream_pending_complete_closes_channel() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);
        let mut stream_rx = conn.register_stream_pending("req-s2".into()).await;

        let frame = TunnelFrame {
            request_id: "req-s2".into(),
            frame_type: FrameType::StreamData as i32,
            ..Default::default()
        };
        assert!(conn.send_stream_frame("req-s2", frame).await);
        assert!(conn.complete_stream("req-s2").await);
        assert!(!conn.has_stream_pending("req-s2").await);

        // Drain buffered frame, then channel closes
        assert!(stream_rx.recv().await.is_some());
        assert!(stream_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn stream_pending_unknown_id_returns_false() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);
        assert!(!conn.send_stream_frame("nope", TunnelFrame::default()).await);
        assert!(!conn.complete_stream("nope").await);
    }

    #[tokio::test]
    async fn cancel_all_clears_both_maps() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);
        let _u = conn.register_pending("u1".into()).await;
        let _s = conn.register_stream_pending("s1".into()).await;
        assert_eq!(conn.pending_count().await, 1);
        assert_eq!(conn.stream_pending_count().await, 1);
        conn.cancel_all_pending().await;
        assert_eq!(conn.pending_count().await, 0);
        assert_eq!(conn.stream_pending_count().await, 0);
    }

    #[tokio::test]
    async fn stream_and_unary_coexist() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);
        let unary_rx = conn.register_pending("u1".into()).await;
        let mut stream_rx = conn.register_stream_pending("s1".into()).await;

        // Complete unary
        let uf = TunnelFrame {
            request_id: "u1".into(),
            frame_type: FrameType::Response as i32,
            ..Default::default()
        };
        assert!(conn.complete_pending("u1", uf).await);
        assert_eq!(
            unary_rx.await.unwrap().frame_type,
            FrameType::Response as i32
        );

        // Stream still works
        let sf = TunnelFrame {
            request_id: "s1".into(),
            frame_type: FrameType::StreamData as i32,
            ..Default::default()
        };
        assert!(conn.send_stream_frame("s1", sf).await);
        assert_eq!(
            stream_rx.recv().await.unwrap().frame_type,
            FrameType::StreamData as i32
        );
        assert!(conn.complete_stream("s1").await);
    }

    #[tokio::test]
    async fn multiple_concurrent_streams() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);
        let mut rx1 = conn.register_stream_pending("s1".into()).await;
        let mut rx2 = conn.register_stream_pending("s2".into()).await;
        assert_eq!(conn.stream_pending_count().await, 2);

        for id in &["s1", "s2"] {
            let f = TunnelFrame {
                request_id: id.to_string(),
                frame_type: FrameType::StreamData as i32,
                ..Default::default()
            };
            assert!(conn.send_stream_frame(id, f).await);
        }
        assert_eq!(rx1.recv().await.unwrap().request_id, "s1");
        assert_eq!(rx2.recv().await.unwrap().request_id, "s2");

        conn.complete_stream("s1").await;
        assert_eq!(conn.stream_pending_count().await, 1);
        assert!(!conn.has_stream_pending("s1").await);
        assert!(conn.has_stream_pending("s2").await);
    }

    #[tokio::test]
    async fn stream_dropped_receiver_returns_false_and_cleans_up() {
        let (tx, _rx) = mpsc::channel(16);
        let conn = TunnelConnection::new("m1".into(), "u1".into(), tx);
        let stream_rx = conn.register_stream_pending("drop".into()).await;
        assert!(conn.has_stream_pending("drop").await);
        assert_eq!(conn.stream_pending_count().await, 1);

        drop(stream_rx);
        assert!(!conn.send_stream_frame("drop", TunnelFrame::default()).await);

        // Dead sender should be cleaned up automatically
        assert!(!conn.has_stream_pending("drop").await);
        assert_eq!(conn.stream_pending_count().await, 0);

        // Should be tracked as cancelled
        assert!(conn.is_cancelled_stream("drop").await);

        // Subsequent sends still return false, but stream is known-cancelled
        assert!(!conn.send_stream_frame("drop", TunnelFrame::default()).await);
        assert!(conn.is_cancelled_stream("drop").await);

        // clear_cancelled_stream removes the tracking
        conn.clear_cancelled_stream("drop").await;
        assert!(!conn.is_cancelled_stream("drop").await);
    }
}
