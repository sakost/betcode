//! Request router that forwards requests through tunnels to daemons.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{info, warn};

use betcode_proto::v1::{
    EncryptedPayload, FrameType, StreamPayload, TunnelError, TunnelErrorCode, TunnelFrame,
};

use crate::buffer::BufferManager;
use crate::registry::ConnectionRegistry;

/// Routes requests through tunnel connections to daemons.
#[derive(Clone)]
pub struct RequestRouter {
    registry: Arc<ConnectionRegistry>,
    buffer: Arc<BufferManager>,
    request_timeout: Duration,
}

impl RequestRouter {
    pub fn new(
        registry: Arc<ConnectionRegistry>,
        buffer: Arc<BufferManager>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            registry,
            buffer,
            request_timeout,
        }
    }

    /// Forward a request to a machine and wait for the response.
    ///
    /// If the machine is offline, the request is buffered for later delivery
    /// and a `Buffered` error is returned.
    pub async fn forward_request(
        &self,
        machine_id: &str,
        request_id: &str,
        method: &str,
        data: Vec<u8>,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<TunnelFrame, RouterError> {
        let conn = match self.registry.get(machine_id).await {
            Some(c) => c,
            None => {
                // Buffer the request for when the machine reconnects
                let metadata_json =
                    serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
                match self
                    .buffer
                    .buffer_request(machine_id, request_id, method, &data, &metadata_json)
                    .await
                {
                    Ok(buf_id) => {
                        info!(
                            machine_id = %machine_id,
                            request_id = %request_id,
                            buffer_id = buf_id,
                            "Request buffered for offline machine"
                        );
                        return Err(RouterError::Buffered(machine_id.to_string()));
                    }
                    Err(e) => {
                        warn!(
                            machine_id = %machine_id,
                            error = %e,
                            "Failed to buffer request for offline machine"
                        );
                        return Err(RouterError::MachineOffline(machine_id.to_string()));
                    }
                }
            }
        };

        // Build request frame — relay forwards encrypted payload opaquely
        let frame = TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Request as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                StreamPayload {
                    method: method.to_string(),
                    encrypted: Some(EncryptedPayload {
                        ciphertext: data,
                        nonce: Vec::new(),
                        ephemeral_pubkey: Vec::new(),
                    }),
                    sequence: 0,
                    metadata,
                },
            )),
        };

        // Register pending response before sending
        let response_rx = conn.register_pending(request_id.to_string()).await;

        // Send through tunnel
        conn.send_frame(frame)
            .await
            .map_err(|_| RouterError::SendFailed(machine_id.to_string()))?;

        // Wait for response with timeout
        match timeout(self.request_timeout, response_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(RouterError::ResponseDropped(request_id.to_string())),
            Err(_) => {
                warn!(
                    machine_id = %machine_id,
                    request_id = %request_id,
                    "Request timed out"
                );
                Err(RouterError::Timeout(request_id.to_string()))
            }
        }
    }

    /// Forward a streaming request to a machine and return a receiver for multiple response frames.
    ///
    /// Unlike `forward_request` which waits for a single response, this registers a stream
    /// channel and returns immediately. The caller reads frames from the receiver until it
    /// closes (StreamEnd received).
    pub async fn forward_stream(
        &self,
        machine_id: &str,
        request_id: &str,
        method: &str,
        data: Vec<u8>,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<mpsc::Receiver<TunnelFrame>, RouterError> {
        let conn = self
            .registry
            .get(machine_id)
            .await
            .ok_or_else(|| RouterError::MachineOffline(machine_id.to_string()))?;

        let frame = TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Request as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                StreamPayload {
                    method: method.to_string(),
                    encrypted: Some(EncryptedPayload {
                        ciphertext: data,
                        nonce: Vec::new(),
                        ephemeral_pubkey: Vec::new(),
                    }),
                    sequence: 0,
                    metadata,
                },
            )),
        };

        // Register stream pending before sending
        let stream_rx = conn.register_stream_pending(request_id.to_string()).await;

        conn.send_frame(frame)
            .await
            .map_err(|_| RouterError::SendFailed(machine_id.to_string()))?;

        Ok(stream_rx)
    }

    /// Forward a bidirectional streaming request.
    ///
    /// Returns a sender for pushing frames to the daemon and a receiver for response frames.
    /// The caller sends client frames via the sender and reads server frames from the receiver.
    pub async fn forward_bidi_stream(
        &self,
        machine_id: &str,
        request_id: &str,
        method: &str,
        data: Vec<u8>,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<(mpsc::Sender<TunnelFrame>, mpsc::Receiver<TunnelFrame>), RouterError> {
        let conn = self
            .registry
            .get(machine_id)
            .await
            .ok_or_else(|| RouterError::MachineOffline(machine_id.to_string()))?;

        let frame = TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Request as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                StreamPayload {
                    method: method.to_string(),
                    encrypted: Some(EncryptedPayload {
                        ciphertext: data,
                        nonce: Vec::new(),
                        ephemeral_pubkey: Vec::new(),
                    }),
                    sequence: 0,
                    metadata,
                },
            )),
        };

        // Register stream pending before sending
        let stream_rx = conn.register_stream_pending(request_id.to_string()).await;

        conn.send_frame(frame)
            .await
            .map_err(|_| RouterError::SendFailed(machine_id.to_string()))?;

        // Return the connection's frame_tx clone for sending additional client frames
        let client_tx = conn.frame_tx.clone();
        Ok((client_tx, stream_rx))
    }

    /// Get a reference to the connection registry.
    pub fn registry(&self) -> &Arc<ConnectionRegistry> {
        &self.registry
    }

    /// Create an error TunnelFrame for returning to callers.
    pub fn error_frame(request_id: &str, code: TunnelErrorCode, message: &str) -> TunnelFrame {
        TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Error as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::Error(
                TunnelError {
                    code: code as i32,
                    message: message.to_string(),
                    details: Default::default(),
                },
            )),
        }
    }

    /// Check if a machine is currently connected.
    pub async fn is_machine_online(&self, machine_id: &str) -> bool {
        self.registry.is_connected(machine_id).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("Machine offline: {0}")]
    MachineOffline(String),

    #[error("Request buffered for offline machine: {0}")]
    Buffered(String),

    #[error("Failed to send through tunnel: {0}")]
    SendFailed(String),

    #[error("Request timed out: {0}")]
    Timeout(String),

    #[error("Response channel dropped: {0}")]
    ResponseDropped(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::RelayDatabase;
    use tokio::sync::mpsc;

    async fn test_buffer(
        registry: &Arc<ConnectionRegistry>,
        machines: &[&str],
    ) -> Arc<BufferManager> {
        let db = RelayDatabase::open_in_memory().await.unwrap();
        db.create_user("u1", "alice", "alice@test.com", "hash")
            .await
            .unwrap();
        for mid in machines {
            db.create_machine(mid, mid, "u1", "{}").await.unwrap();
        }
        Arc::new(BufferManager::new(db, Arc::clone(registry)))
    }

    #[tokio::test]
    async fn forward_request_to_online_machine() {
        let registry = Arc::new(ConnectionRegistry::new());
        let (tx, mut rx) = mpsc::channel(16);

        registry.register("m1".into(), "u1".into(), tx).await;

        let buffer = test_buffer(&registry, &["m1"]).await;
        let router = RequestRouter::new(Arc::clone(&registry), buffer, Duration::from_secs(5));

        // Spawn responder that echoes back
        let reg_clone = Arc::clone(&registry);
        tokio::spawn(async move {
            if let Some(frame) = rx.recv().await {
                let conn = reg_clone.get("m1").await.unwrap();
                let rid = frame.request_id;
                let response = TunnelFrame {
                    request_id: rid.clone(),
                    frame_type: FrameType::Response as i32,
                    ..Default::default()
                };
                conn.complete_pending(&rid, response).await;
            }
        });

        let result = router
            .forward_request("m1", "req-1", "TestMethod", vec![], Default::default())
            .await;

        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.request_id, "req-1");
        assert_eq!(resp.frame_type, FrameType::Response as i32);
    }

    #[tokio::test]
    async fn forward_to_offline_machine_buffers() {
        let registry = Arc::new(ConnectionRegistry::new());
        let buffer = test_buffer(&registry, &["m-missing"]).await;
        let router = RequestRouter::new(Arc::clone(&registry), buffer, Duration::from_secs(1));

        let result = router
            .forward_request("m-missing", "req-1", "Test", vec![], Default::default())
            .await;

        // Should be buffered, not a hard offline error
        assert!(matches!(result, Err(RouterError::Buffered(_))));
    }

    #[tokio::test]
    async fn forward_request_timeout() {
        let registry = Arc::new(ConnectionRegistry::new());
        let (tx, _rx) = mpsc::channel(16);

        registry.register("m1".into(), "u1".into(), tx).await;

        let buffer = test_buffer(&registry, &["m1"]).await;
        // Very short timeout, no responder
        let router = RequestRouter::new(Arc::clone(&registry), buffer, Duration::from_millis(50));

        let result = router
            .forward_request("m1", "req-1", "Test", vec![], Default::default())
            .await;

        assert!(matches!(result, Err(RouterError::Timeout(_))));
    }

    #[tokio::test]
    async fn forward_stream_to_online_machine() {
        let registry = Arc::new(ConnectionRegistry::new());
        let (tx, mut tunnel_rx) = mpsc::channel(16);
        registry.register("m1".into(), "u1".into(), tx).await;

        let buffer = test_buffer(&registry, &["m1"]).await;
        let router = RequestRouter::new(Arc::clone(&registry), buffer, Duration::from_secs(5));

        // Spawn mock responder: 3 StreamData + StreamEnd
        let reg = Arc::clone(&registry);
        tokio::spawn(async move {
            if let Some(frame) = tunnel_rx.recv().await {
                let rid = frame.request_id;
                let conn = reg.get("m1").await.unwrap();
                for i in 0..3 {
                    let f = TunnelFrame {
                        request_id: rid.clone(),
                        frame_type: FrameType::StreamData as i32,
                        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                            StreamPayload {
                                method: String::new(),
                                encrypted: Some(EncryptedPayload {
                                    ciphertext: vec![i],
                                    nonce: Vec::new(),
                                    ephemeral_pubkey: Vec::new(),
                                }),
                                sequence: i as u64,
                                metadata: Default::default(),
                            },
                        )),
                        ..Default::default()
                    };
                    conn.send_stream_frame(&rid, f).await;
                }
                conn.complete_stream(&rid).await;
            }
        });

        let mut rx = router
            .forward_stream("m1", "req-s1", "Test/Stream", vec![], Default::default())
            .await
            .unwrap();

        let mut received = vec![];
        while let Some(f) = rx.recv().await {
            received.push(f);
        }
        assert_eq!(received.len(), 3);
        for (i, f) in received.iter().enumerate() {
            assert_eq!(f.request_id, "req-s1");
            assert_eq!(f.frame_type, FrameType::StreamData as i32);
            if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &f.payload {
                assert_eq!(p.encrypted.as_ref().unwrap().ciphertext, vec![i as u8]);
            }
        }
    }

    #[tokio::test]
    async fn forward_stream_offline_returns_error() {
        let registry = Arc::new(ConnectionRegistry::new());
        let buffer = test_buffer(&registry, &["m1"]).await;
        let router = RequestRouter::new(Arc::clone(&registry), buffer, Duration::from_secs(1));

        let result = router
            .forward_stream("m1", "req-1", "Test", vec![], Default::default())
            .await;
        assert!(matches!(result, Err(RouterError::MachineOffline(_))));
    }

    #[tokio::test]
    async fn forward_bidi_stream_sends_and_receives() {
        let registry = Arc::new(ConnectionRegistry::new());
        let (tx, mut tunnel_rx) = mpsc::channel(16);
        registry.register("m1".into(), "u1".into(), tx).await;

        let buffer = test_buffer(&registry, &["m1"]).await;
        let router = RequestRouter::new(Arc::clone(&registry), buffer, Duration::from_secs(5));

        let reg = Arc::clone(&registry);
        tokio::spawn(async move {
            // Receive initial request
            if let Some(frame) = tunnel_rx.recv().await {
                let rid = frame.request_id;
                let conn = reg.get("m1").await.unwrap();
                // Send one response
                let f = TunnelFrame {
                    request_id: rid.clone(),
                    frame_type: FrameType::StreamData as i32,
                    ..Default::default()
                };
                conn.send_stream_frame(&rid, f).await;

                // Wait for client frame
                if let Some(client_frame) = tunnel_rx.recv().await {
                    assert_eq!(client_frame.request_id, rid);
                    // Send final + close
                    let f2 = TunnelFrame {
                        request_id: rid.clone(),
                        frame_type: FrameType::StreamData as i32,
                        ..Default::default()
                    };
                    conn.send_stream_frame(&rid, f2).await;
                    conn.complete_stream(&rid).await;
                }
            }
        });

        let (client_tx, mut rx) = router
            .forward_bidi_stream("m1", "req-b1", "Test/Bidi", vec![], Default::default())
            .await
            .unwrap();

        // Receive first server frame
        let f1 = rx.recv().await.unwrap();
        assert_eq!(f1.request_id, "req-b1");

        // Send client frame
        let client_frame = TunnelFrame {
            request_id: "req-b1".into(),
            frame_type: FrameType::StreamData as i32,
            ..Default::default()
        };
        client_tx.send(client_frame).await.unwrap();

        // Receive second server frame
        let f2 = rx.recv().await.unwrap();
        assert_eq!(f2.request_id, "req-b1");

        // Stream closes
        assert!(rx.recv().await.is_none());
    }
}
