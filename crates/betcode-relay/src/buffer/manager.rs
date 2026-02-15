//! Buffer manager for offline machine message queuing.
//!
//! When a machine is offline, requests are buffered in the database.
//! When the machine reconnects, buffered messages are drained and forwarded.

use std::sync::Arc;

use tracing::{info, warn};

use crate::registry::ConnectionRegistry;
use crate::router::forwarder::build_request_frame;
use crate::storage::{BufferMessageParams, RelayDatabase};

/// Manages message buffering for offline machines.
pub struct BufferManager {
    db: RelayDatabase,
    registry: Arc<ConnectionRegistry>,
    /// Default TTL for buffered messages in seconds.
    default_ttl_secs: i64,
}

impl BufferManager {
    pub const fn new(db: RelayDatabase, registry: Arc<ConnectionRegistry>) -> Self {
        Self {
            db,
            registry,
            default_ttl_secs: 3600, // 1 hour default
        }
    }

    /// Buffer a request for an offline machine.
    pub async fn buffer_request(
        &self,
        machine_id: &str,
        request_id: &str,
        method: &str,
        data: &[u8],
        metadata_json: &str,
    ) -> Result<i64, BufferError> {
        let id = self
            .db
            .buffer_message(&BufferMessageParams {
                machine_id,
                request_id,
                method,
                payload: data,
                metadata: metadata_json,
                priority: 0,
                ttl_secs: self.default_ttl_secs,
            })
            .await
            .map_err(|e| BufferError::Storage(e.to_string()))?;

        info!(
            machine_id = %machine_id,
            request_id = %request_id,
            method = %method,
            "Request buffered for offline machine"
        );

        Ok(id)
    }

    /// Drain buffered messages for a machine and forward them through the tunnel.
    ///
    /// Called when a machine reconnects. Returns the number of messages drained.
    pub async fn drain_buffer(&self, machine_id: &str) -> Result<u64, BufferError> {
        let messages = self
            .db
            .drain_buffer(machine_id)
            .await
            .map_err(|e| BufferError::Storage(e.to_string()))?;

        if messages.is_empty() {
            return Ok(0);
        }

        let conn = self.registry.get(machine_id).await;
        let Some(conn) = conn else {
            warn!(
                machine_id = %machine_id,
                count = messages.len(),
                "Machine went offline again before buffer drain completed"
            );
            return Err(BufferError::MachineOffline(machine_id.to_string()));
        };

        let total = messages.len();
        let mut sent = 0u64;
        for msg in &messages {
            let metadata: std::collections::HashMap<String, String> =
                serde_json::from_str(&msg.metadata).unwrap_or_default();

            let frame =
                build_request_frame(&msg.request_id, &msg.method, msg.payload.clone(), metadata);

            if conn.send_frame(frame).await.is_err() {
                warn!(
                    machine_id = %machine_id,
                    request_id = %msg.request_id,
                    sent,
                    remaining = total as u64 - sent,
                    "Failed to send buffered frame, remaining messages preserved in DB"
                );
                break;
            }

            // Delete from DB only after successful send
            if let Err(e) = self.db.delete_buffered_message(msg.id).await {
                warn!(
                    machine_id = %machine_id,
                    buffer_id = msg.id,
                    error = %e,
                    "Failed to delete delivered buffer message"
                );
            }
            sent += 1;
        }

        if sent < total as u64 {
            warn!(
                machine_id = %machine_id,
                total,
                sent,
                retained = total as u64 - sent,
                "Buffer drain incomplete, unsent messages retained for next reconnect"
            );
        } else {
            info!(
                machine_id = %machine_id,
                count = sent,
                "Buffer drained successfully"
            );
        }

        Ok(sent)
    }

    /// Clean up expired buffered messages. Returns the count removed.
    pub async fn cleanup_expired(&self) -> Result<u64, BufferError> {
        let removed = self
            .db
            .cleanup_expired_buffer()
            .await
            .map_err(|e| BufferError::Storage(e.to_string()))?;

        if removed > 0 {
            info!(removed, "Cleaned up expired buffered messages");
        }

        Ok(removed)
    }

    /// Get count of buffered messages for a machine.
    pub async fn buffered_count(&self, machine_id: &str) -> Result<i64, BufferError> {
        self.db
            .count_buffered_messages(machine_id)
            .await
            .map_err(|e| BufferError::Storage(e.to_string()))
    }
}

/// Buffer operation errors.
#[derive(Debug, thiserror::Error)]
pub enum BufferError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Machine offline: {0}")]
    MachineOffline(String),
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use betcode_proto::v1::FrameType;
    use tokio::sync::mpsc;

    async fn setup() -> (BufferManager, Arc<ConnectionRegistry>) {
        let db = RelayDatabase::open_in_memory().await.unwrap();
        // Create user and machine to satisfy foreign key constraints
        db.create_user("u1", "alice", "alice@example.com", "hash")
            .await
            .unwrap();
        db.create_machine("m1", "test-machine", "u1", "{}")
            .await
            .unwrap();
        let registry = Arc::new(ConnectionRegistry::new());
        let manager = BufferManager::new(db, Arc::clone(&registry));
        (manager, registry)
    }

    #[tokio::test]
    async fn buffer_and_drain_for_online_machine() {
        let (manager, registry) = setup().await;
        let (tx, mut rx) = mpsc::channel(16);

        // Buffer a request while machine is offline
        manager
            .buffer_request("m1", "req-1", "Test/Method", b"hello", "{}")
            .await
            .unwrap();

        assert_eq!(manager.buffered_count("m1").await.unwrap(), 1);

        // Machine comes online
        registry.register("m1".into(), "u1".into(), tx).await;

        // Drain buffer
        let sent = manager.drain_buffer("m1").await.unwrap();
        assert_eq!(sent, 1);

        // Verify frame was sent through tunnel
        let frame = rx.recv().await.unwrap();
        assert_eq!(frame.request_id, "req-1");
        assert_eq!(frame.frame_type, FrameType::Request as i32);

        // Buffer should be empty now
        assert_eq!(manager.buffered_count("m1").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn drain_offline_machine_returns_error() {
        let (manager, _registry) = setup().await;

        manager
            .buffer_request("m1", "req-1", "Test", b"data", "{}")
            .await
            .unwrap();

        let result = manager.drain_buffer("m1").await;
        assert!(matches!(result, Err(BufferError::MachineOffline(_))));
    }

    #[tokio::test]
    async fn drain_empty_buffer_returns_zero() {
        let (manager, registry) = setup().await;
        let (tx, _rx) = mpsc::channel(16);
        registry.register("m1".into(), "u1".into(), tx).await;

        let sent = manager.drain_buffer("m1").await.unwrap();
        assert_eq!(sent, 0);
    }

    #[tokio::test]
    async fn cleanup_expired_messages() {
        let (manager, _registry) = setup().await;

        // Buffer with very short TTL (already expired by using negative TTL trick)
        manager
            .db
            .buffer_message(&BufferMessageParams {
                machine_id: "m1",
                request_id: "req-1",
                method: "Test",
                payload: b"data",
                metadata: "{}",
                priority: 0,
                ttl_secs: -1,
            })
            .await
            .unwrap();

        let removed = manager.cleanup_expired().await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(manager.buffered_count("m1").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn multiple_messages_ordered_by_priority() {
        let (manager, registry) = setup().await;
        let (tx, mut rx) = mpsc::channel(16);

        // Buffer messages with different priorities
        manager
            .db
            .buffer_message(&BufferMessageParams {
                machine_id: "m1",
                request_id: "low",
                method: "Test",
                payload: b"low",
                metadata: "{}",
                priority: 0,
                ttl_secs: 3600,
            })
            .await
            .unwrap();
        manager
            .db
            .buffer_message(&BufferMessageParams {
                machine_id: "m1",
                request_id: "high",
                method: "Test",
                payload: b"high",
                metadata: "{}",
                priority: 10,
                ttl_secs: 3600,
            })
            .await
            .unwrap();

        registry.register("m1".into(), "u1".into(), tx).await;
        let sent = manager.drain_buffer("m1").await.unwrap();
        assert_eq!(sent, 2);

        // High priority should come first
        let first = rx.recv().await.unwrap();
        assert_eq!(first.request_id, "high");
        let second = rx.recv().await.unwrap();
        assert_eq!(second.request_id, "low");
    }
}
