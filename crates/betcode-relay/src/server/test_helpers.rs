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

/// Generate `setup_with_machine` and `setup_offline` helper functions for a
/// proxy service test module.
///
/// Usage (inside a test module):
/// ```ignore
/// proxy_test_setup!(WorktreeProxyService);
/// ```
///
/// This expands to two `async fn`s that wrap `setup_router_with_machine` /
/// `setup_offline_router` and construct the given service type from the router.
macro_rules! proxy_test_setup {
    ($svc_ty:ty) => {
        async fn setup_with_machine(
            mid: &str,
        ) -> (
            $svc_ty,
            std::sync::Arc<$crate::router::RequestRouter>,
            tokio::sync::mpsc::Receiver<betcode_proto::v1::TunnelFrame>,
        ) {
            let (router, rx) = $crate::server::test_helpers::setup_router_with_machine(mid).await;
            (<$svc_ty>::new(std::sync::Arc::clone(&router)), router, rx)
        }

        async fn setup_offline() -> $svc_ty {
            let router = $crate::server::test_helpers::setup_offline_router().await;
            <$svc_ty>::new(router)
        }
    };
}

pub(crate) use proxy_test_setup;

/// Create a `Request<T>` with test claims but *without* the `x-machine-id`
/// header.  Used to test the "missing machine-id" error path.
pub fn make_request_no_machine<T>(inner: T) -> Request<T> {
    let mut req = Request::new(inner);
    req.extensions_mut().insert(test_claims());
    req
}

/// Create a `Request<T>` with the `x-machine-id` header but *without* claims
/// in the extensions.  Used to test the "missing claims" error path.
pub fn make_request_no_claims<T>(inner: T, machine_id: &str) -> Request<T> {
    let mut req = Request::new(inner);
    req.metadata_mut()
        .insert("x-machine-id", machine_id.parse().unwrap());
    req
}

/// Assert that calling `$method` without claims produces a `tonic::Code::Internal` error
/// mentioning "claims" (case-insensitive).
macro_rules! assert_no_claims_error {
    ($svc:expr, $method:ident, $req:expr) => {{
        let req = $crate::server::test_helpers::make_request_no_claims($req, "m1");
        let err = $svc.$method(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
        assert!(
            err.message().to_lowercase().contains("claims"),
            "expected claims error, got: {}",
            err.message()
        );
    }};
}

pub(crate) use assert_no_claims_error;

/// Assert that calling `$method` without a machine-id header produces a
/// `tonic::Code::InvalidArgument` error mentioning "machine".
macro_rules! assert_no_machine_error {
    ($svc:expr, $method:ident, $req:expr) => {{
        let req = $crate::server::test_helpers::make_request_no_machine($req);
        let err = $svc.$method(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().to_lowercase().contains("machine"),
            "expected machine error, got: {}",
            err.message()
        );
    }};
}

pub(crate) use assert_no_machine_error;

/// Assert that calling `$method` on an offline machine produces a
/// `tonic::Code::Unavailable` error.
///
/// Uses `match` instead of `unwrap_err()` to support response types that don't
/// implement `Debug` (e.g. streaming responses).
macro_rules! assert_offline_error {
    ($svc:expr, $method:ident, $req:expr) => {{
        let req = $crate::server::test_helpers::make_request($req, "m-off");
        match $svc.$method(req).await {
            Err(err) => assert_eq!(err.code(), tonic::Code::Unavailable),
            Ok(_) => panic!("expected Unavailable error, got Ok"),
        }
    }};
}

pub(crate) use assert_offline_error;

/// Assert that a daemon error is propagated to the client as a `tonic::Code::Internal` error.
macro_rules! assert_daemon_error {
    ($svc:expr, $method:ident, $req:expr, $router:expr, $rx:expr, $msg:expr) => {{
        $crate::server::test_helpers::spawn_error_responder(&$router, "m1", $rx, $msg);
        let req = $crate::server::test_helpers::make_request($req, "m1");
        let err = $svc.$method(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
        assert!(
            err.message().contains($msg),
            "expected daemon error '{}', got: {}",
            $msg,
            err.message()
        );
    }};
}

pub(crate) use assert_daemon_error;

/// Receive the first `TunnelFrame` from a tunnel channel and resolve the
/// associated `TunnelConnection`.
///
/// Returns `(request_id, connection)`.  This is the shared preamble used by
/// all `spawn_*_responder` helpers.
async fn recv_and_connect(
    tunnel_rx: &mut mpsc::Receiver<TunnelFrame>,
    router: &RequestRouter,
    mid: &str,
) -> Option<(String, Arc<crate::registry::TunnelConnection>)> {
    let frame = tunnel_rx.recv().await?;
    let rid = frame.request_id.clone();
    let conn = router.registry().get(mid).await.unwrap();
    Some((rid, conn))
}

/// Spawn a mock daemon task that receives the first frame on the tunnel
/// channel, resolves the connection, and passes control to `handler`.
///
/// This is the common preamble for `spawn_responder`, `spawn_stream_responder`,
/// and `spawn_error_responder`.
fn spawn_tunnel_handler<F, Fut>(
    router: &Arc<RequestRouter>,
    mid: &str,
    mut tunnel_rx: mpsc::Receiver<TunnelFrame>,
    handler: F,
) where
    F: FnOnce(String, Arc<crate::registry::TunnelConnection>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let router = Arc::clone(router);
    let mid = mid.to_string();
    tokio::spawn(async move {
        if let Some((rid, conn)) = recv_and_connect(&mut tunnel_rx, &router, &mid).await {
            handler(rid, conn).await;
        }
    });
}

/// Spawn a mock daemon that replies with multiple `StreamData` frames and then
/// closes the stream.
///
/// `messages` is a list of protobuf messages, each sent as a separate
/// `StreamData` frame with an incrementing sequence number.
pub fn spawn_stream_responder<M: Message + 'static>(
    router: &Arc<RequestRouter>,
    mid: &str,
    tunnel_rx: mpsc::Receiver<TunnelFrame>,
    messages: Vec<M>,
) {
    spawn_tunnel_handler(router, mid, tunnel_rx, |rid, conn| async move {
        for (seq, msg) in messages.iter().enumerate() {
            let f = stream_data_frame(&rid, msg, seq as u64);
            conn.send_stream_frame(&rid, f).await;
        }
        conn.complete_stream(&rid).await;
    });
}

/// Spawn a mock daemon that replies with a `TunnelError` frame to the first request.
///
/// This simulates a daemon-side error being propagated back through the relay.
pub fn spawn_error_responder(
    router: &Arc<RequestRouter>,
    machine_id: &str,
    rx: mpsc::Receiver<TunnelFrame>,
    error_message: &str,
) {
    let msg = error_message.to_string();
    spawn_tunnel_handler(router, machine_id, rx, |rid, conn| async move {
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
        conn.complete_pending(&rid, err_frame).await;
    });
}

/// Build a `TunnelFrame` with the given frame type, encoding `msg` as the payload.
fn build_payload_frame<M: Message>(
    request_id: &str,
    msg: &M,
    frame_type: FrameType,
    sequence: u64,
) -> TunnelFrame {
    TunnelFrame {
        request_id: request_id.to_string(),
        frame_type: frame_type as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: String::new(),
                encrypted: Some(EncryptedPayload {
                    ciphertext: encode_msg(msg),
                    nonce: Vec::new(),
                    ephemeral_pubkey: Vec::new(),
                }),
                sequence,
                metadata: HashMap::new(),
            },
        )),
    }
}

/// Build a `TunnelFrame` containing a protobuf-encoded `StreamData` payload.
///
/// This is the frame shape used by both unary response responders and stream
/// data frames in tests.
pub fn stream_data_frame<M: Message>(request_id: &str, msg: &M, sequence: u64) -> TunnelFrame {
    build_payload_frame(request_id, msg, FrameType::StreamData, sequence)
}

/// Spawn a mock daemon that replies with an encoded response to the first request.
pub fn spawn_responder<M: Message + 'static>(
    router: &Arc<RequestRouter>,
    mid: &str,
    tunnel_rx: mpsc::Receiver<TunnelFrame>,
    response: M,
) {
    spawn_tunnel_handler(router, mid, tunnel_rx, |rid, conn| async move {
        let resp_frame = build_payload_frame(&rid, &response, FrameType::Response, 0);
        conn.complete_pending(&rid, resp_frame).await;
    });
}
