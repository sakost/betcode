//! Shared test helpers for relay proxy test modules.
//!
//! Provides common setup and utility functions used across
//! `agent_proxy_tests`, `command_proxy_tests`, `gitlab_proxy_tests`,
//! and `worktree_proxy_tests`.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use prost::Message;
use tokio::sync::mpsc;
use tonic::Request;

use betcode_proto::v1::{EncryptedPayload, FrameType, StreamPayload, TunnelErrorCode, TunnelFrame};

use crate::auth::claims::Claims;
use crate::buffer::BufferManager;
use crate::registry::ConnectionRegistry;
use crate::router::RequestRouter;
use crate::storage::RelayDatabase;

/// Build a `Claims` value suitable for tests.
pub fn test_claims() -> Claims {
    Claims {
        jti: "test-jti".into(),
        sub: "u1".into(),
        username: "alice".into(),
        iat: 0,
        exp: i64::MAX,
        token_type: "access".into(),
    }
}

/// Create a `Request<T>` with the `x-machine-id` header and test claims
/// already attached.
pub fn make_request<T>(inner: T, machine_id: &str) -> Request<T> {
    let mut req = Request::new(inner);
    req.metadata_mut()
        .insert("x-machine-id", machine_id.parse().unwrap());
    req.extensions_mut().insert(test_claims());
    req
}

/// Encode a protobuf message into a `Vec<u8>`.
pub fn encode_msg<M: Message>(msg: &M) -> Vec<u8> {
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    buf
}

/// Set up a [`RequestRouter`] with a machine registered and connected,
/// returning the router and the receiving end of the tunnel channel.
///
/// Each proxy test file wraps this to construct its own concrete proxy type
/// from the returned router.
pub async fn setup_router_with_machine(
    mid: &str,
) -> (Arc<RequestRouter>, mpsc::Receiver<TunnelFrame>) {
    let registry = Arc::new(ConnectionRegistry::new());
    let (tx, rx) = mpsc::channel(128);
    registry.register(mid.into(), "u1".into(), tx).await;
    let db = RelayDatabase::open_in_memory().await.unwrap();
    db.create_user("u1", "alice", "a@t.com", "hash")
        .await
        .unwrap();
    db.create_machine(mid, mid, "u1", "{}").await.unwrap();
    let buffer = Arc::new(BufferManager::new(db, Arc::clone(&registry)));
    let router = Arc::new(RequestRouter::new(registry, buffer, Duration::from_secs(5)));
    (router, rx)
}

/// Set up a [`RequestRouter`] *without* any connected machine (the machine
/// record exists in the DB but no tunnel sender is registered).
///
/// Each proxy test file wraps this to construct its own concrete proxy type
/// from the returned router.
pub async fn setup_offline_router() -> Arc<RequestRouter> {
    let registry = Arc::new(ConnectionRegistry::new());
    let db = RelayDatabase::open_in_memory().await.unwrap();
    db.create_user("u1", "alice", "a@t.com", "hash")
        .await
        .unwrap();
    db.create_machine("m-off", "m-off", "u1", "{}")
        .await
        .unwrap();
    let buffer = Arc::new(BufferManager::new(db, Arc::clone(&registry)));
    Arc::new(RequestRouter::new(registry, buffer, Duration::from_secs(5)))
}

/// Spawn a mock daemon that replies with a `TunnelError` frame to the first request.
///
/// This simulates a daemon-side error being propagated back through the relay.
pub fn spawn_error_responder(
    router: &Arc<RequestRouter>,
    machine_id: &str,
    mut rx: mpsc::Receiver<TunnelFrame>,
    error_message: &str,
) {
    let router = Arc::clone(router);
    let mid = machine_id.to_string();
    let msg = error_message.to_string();
    tokio::spawn(async move {
        if let Some(frame) = rx.recv().await {
            let rid = frame.request_id.clone();
            let err_frame = TunnelFrame {
                request_id: rid.clone(),
                frame_type: FrameType::Error as i32,
                timestamp: None,
                payload: Some(betcode_proto::v1::tunnel_frame::Payload::Error(
                    betcode_proto::v1::TunnelError {
                        code: TunnelErrorCode::Internal as i32,
                        message: msg,
                        details: HashMap::new(),
                    },
                )),
            };
            let conn = router.registry().get(&mid).await.unwrap();
            conn.complete_pending(&rid, err_frame).await;
        }
    });
}

/// Spawn a mock daemon that replies with an encoded response to the first request.
pub fn spawn_responder<M: Message + 'static>(
    router: &Arc<RequestRouter>,
    mid: &str,
    mut tunnel_rx: mpsc::Receiver<TunnelFrame>,
    response: M,
) {
    let router = Arc::clone(router);
    let mid = mid.to_string();
    tokio::spawn(async move {
        if let Some(frame) = tunnel_rx.recv().await {
            let rid = frame.request_id.clone();
            let resp_frame = TunnelFrame {
                request_id: rid.clone(),
                frame_type: FrameType::Response as i32,
                timestamp: None,
                payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                    StreamPayload {
                        method: String::new(),
                        encrypted: Some(EncryptedPayload {
                            ciphertext: encode_msg(&response),
                            nonce: Vec::new(),
                            ephemeral_pubkey: Vec::new(),
                        }),
                        sequence: 0,
                        metadata: HashMap::new(),
                    },
                )),
            };
            let conn = router.registry().get(&mid).await.unwrap();
            conn.complete_pending(&rid, resp_frame).await;
        }
    });
}
