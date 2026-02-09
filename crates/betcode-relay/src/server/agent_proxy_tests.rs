//! Tests for AgentProxyService.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use prost::Message;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tonic::Request;

use betcode_proto::v1::agent_service_server::AgentService;
use betcode_proto::v1::{
    AgentEvent, CancelTurnRequest, CancelTurnResponse, CompactSessionRequest,
    CompactSessionResponse, EncryptedPayload, FrameType, InputLockRequest, InputLockResponse,
    KeyExchangeRequest, KeyExchangeResponse, ListSessionsRequest, ListSessionsResponse,
    ResumeSessionRequest, StreamPayload, TunnelFrame,
};

use super::{extract_machine_id, AgentProxyService};
use crate::auth::claims::Claims;
use crate::buffer::BufferManager;
use crate::registry::ConnectionRegistry;
use crate::router::RequestRouter;
use crate::storage::RelayDatabase;

fn test_claims() -> Claims {
    Claims {
        jti: "test-jti".into(),
        sub: "u1".into(),
        username: "alice".into(),
        iat: 0,
        exp: i64::MAX,
        token_type: "access".into(),
    }
}

fn make_request<T>(inner: T, machine_id: &str) -> Request<T> {
    let mut req = Request::new(inner);
    req.metadata_mut()
        .insert("x-machine-id", machine_id.parse().unwrap());
    req.extensions_mut().insert(test_claims());
    req
}

fn encode_msg<M: Message>(msg: &M) -> Vec<u8> {
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    buf
}

async fn setup_with_machine(
    mid: &str,
) -> (
    AgentProxyService,
    Arc<RequestRouter>,
    mpsc::Receiver<TunnelFrame>,
) {
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
    (AgentProxyService::new(Arc::clone(&router)), router, rx)
}

async fn setup_offline() -> AgentProxyService {
    let registry = Arc::new(ConnectionRegistry::new());
    let db = RelayDatabase::open_in_memory().await.unwrap();
    db.create_user("u1", "alice", "a@t.com", "hash")
        .await
        .unwrap();
    db.create_machine("m-off", "m-off", "u1", "{}")
        .await
        .unwrap();
    let buffer = Arc::new(BufferManager::new(db, Arc::clone(&registry)));
    let router = Arc::new(RequestRouter::new(registry, buffer, Duration::from_secs(5)));
    AgentProxyService::new(router)
}

/// Spawn a mock daemon that replies with an encoded response to the first request.
fn spawn_responder<M: Message + 'static>(
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

// --- extract_machine_id ---

#[test]
fn extract_machine_id_present() {
    let mut req = Request::new(());
    req.metadata_mut()
        .insert("x-machine-id", "m1".parse().unwrap());
    assert_eq!(extract_machine_id(&req).unwrap(), "m1");
}

#[test]
fn extract_machine_id_missing() {
    let req = Request::new(());
    let err = extract_machine_id(&req).unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// --- Unary RPC routing ---

#[tokio::test]
async fn list_sessions_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListSessionsResponse {
            sessions: vec![],
            total: 0,
        },
    );
    let req = make_request(
        ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_sessions(req).await.unwrap().into_inner();
    assert_eq!(resp.sessions.len(), 0);
}

#[tokio::test]
async fn cancel_turn_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(&router, "m1", rx, CancelTurnResponse { was_active: true });
    let req = make_request(
        CancelTurnRequest {
            session_id: "s1".into(),
        },
        "m1",
    );
    let resp = svc.cancel_turn(req).await.unwrap().into_inner();
    assert!(resp.was_active);
}

#[tokio::test]
async fn compact_session_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let response = CompactSessionResponse {
        messages_before: 10,
        messages_after: 3,
        tokens_saved: 500,
    };
    spawn_responder(&router, "m1", rx, response);
    let req = make_request(
        CompactSessionRequest {
            session_id: "s1".into(),
        },
        "m1",
    );
    let resp = svc.compact_session(req).await.unwrap().into_inner();
    assert_eq!(resp.messages_before, 10);
    assert_eq!(resp.messages_after, 3);
}

#[tokio::test]
async fn request_input_lock_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        InputLockResponse {
            granted: true,
            previous_holder: String::new(),
        },
    );
    let req = make_request(
        InputLockRequest {
            session_id: "s1".into(),
        },
        "m1",
    );
    let resp = svc.request_input_lock(req).await.unwrap().into_inner();
    assert!(resp.granted);
}

// --- Error handling ---

#[tokio::test]
async fn machine_offline_returns_unavailable() {
    let svc = setup_offline().await;
    let req = make_request(
        ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        },
        "m-off",
    );
    let err = svc.list_sessions(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    });
    req.extensions_mut().insert(test_claims());
    let err = svc.list_sessions(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    });
    req.metadata_mut()
        .insert("x-machine-id", "m1".parse().unwrap());
    let err = svc.list_sessions(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, mut rx) = setup_with_machine("m1").await;
    let rc = Arc::clone(&router);
    tokio::spawn(async move {
        if let Some(frame) = rx.recv().await {
            let rid = frame.request_id.clone();
            let err_frame = TunnelFrame {
                request_id: rid.clone(),
                frame_type: FrameType::Error as i32,
                timestamp: None,
                payload: Some(betcode_proto::v1::tunnel_frame::Payload::Error(
                    betcode_proto::v1::TunnelError {
                        code: betcode_proto::v1::TunnelErrorCode::Internal as i32,
                        message: "daemon crashed".into(),
                        details: HashMap::new(),
                    },
                )),
            };
            let conn = rc.registry().get("m1").await.unwrap();
            conn.complete_pending(&rid, err_frame).await;
        }
    });
    let req = make_request(
        ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let err = svc.list_sessions(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
    assert!(err.message().contains("daemon crashed"));
}

// --- Streaming ---

#[tokio::test]
async fn resume_session_streams_events() {
    let (svc, router, mut tunnel_rx) = setup_with_machine("m1").await;
    let rc = Arc::clone(&router);
    tokio::spawn(async move {
        if let Some(frame) = tunnel_rx.recv().await {
            let rid = frame.request_id.clone();
            let conn = rc.registry().get("m1").await.unwrap();
            for seq in 0u64..2 {
                let event = AgentEvent {
                    sequence: seq,
                    ..Default::default()
                };
                let f = TunnelFrame {
                    request_id: rid.clone(),
                    frame_type: FrameType::StreamData as i32,
                    timestamp: None,
                    payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                        StreamPayload {
                            method: String::new(),
                            encrypted: Some(EncryptedPayload {
                                ciphertext: encode_msg(&event),
                                nonce: Vec::new(),
                                ephemeral_pubkey: Vec::new(),
                            }),
                            sequence: seq,
                            metadata: HashMap::new(),
                        },
                    )),
                };
                conn.send_stream_frame(&rid, f).await;
            }
            conn.complete_stream(&rid).await;
        }
    });
    let req = make_request(
        ResumeSessionRequest {
            session_id: "s1".into(),
            from_sequence: 0,
        },
        "m1",
    );
    let resp = svc.resume_session(req).await.unwrap();
    let mut stream = resp.into_inner();
    let mut events = vec![];
    while let Some(result) = stream.next().await {
        events.push(result.unwrap());
    }
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, 0);
    assert_eq!(events[1].sequence, 1);
}

#[tokio::test]
async fn resume_session_offline_returns_unavailable() {
    let svc = setup_offline().await;
    let req = make_request(
        ResumeSessionRequest {
            session_id: "s1".into(),
            from_sequence: 0,
        },
        "m-off",
    );
    match svc.resume_session(req).await {
        Err(err) => assert_eq!(err.code(), tonic::Code::Unavailable),
        Ok(_) => panic!("Expected unavailable error"),
    }
}

// --- Key exchange ---

#[tokio::test]
async fn exchange_keys_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        KeyExchangeResponse {
            daemon_identity_pubkey: vec![1u8; 32],
            daemon_fingerprint: "aa:bb:cc".into(),
            daemon_ephemeral_pubkey: vec![2u8; 32],
        },
    );
    let req = make_request(
        KeyExchangeRequest {
            machine_id: "m1".into(),
            identity_pubkey: vec![3u8; 32],
            fingerprint: "dd:ee:ff".into(),
            ephemeral_pubkey: vec![4u8; 32],
        },
        "m1",
    );
    let resp = svc.exchange_keys(req).await.unwrap().into_inner();
    assert_eq!(resp.daemon_ephemeral_pubkey, vec![2u8; 32]);
    assert_eq!(resp.daemon_fingerprint, "aa:bb:cc");
}

#[tokio::test]
async fn exchange_keys_offline_returns_unavailable() {
    let svc = setup_offline().await;
    let req = make_request(
        KeyExchangeRequest {
            machine_id: "m-off".into(),
            identity_pubkey: Vec::new(),
            fingerprint: String::new(),
            ephemeral_pubkey: vec![0u8; 32],
        },
        "m-off",
    );
    let err = svc.exchange_keys(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
}

// --- Encrypted event passthrough ---

#[tokio::test]
async fn encrypted_agent_event_forwards_through_proxy() {
    let (svc, router, mut tunnel_rx) = setup_with_machine("m1").await;
    let rc = Arc::clone(&router);

    // Prepare an AgentEvent with the Encrypted variant (opaque ciphertext bytes)
    let opaque_ciphertext = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
    let opaque_nonce = vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC];
    let encrypted_event = AgentEvent {
        sequence: 42,
        timestamp: None,
        parent_tool_use_id: String::new(),
        event: Some(betcode_proto::v1::agent_event::Event::Encrypted(
            betcode_proto::v1::EncryptedEnvelope {
                ciphertext: opaque_ciphertext.clone(),
                nonce: opaque_nonce.clone(),
            },
        )),
    };

    tokio::spawn(async move {
        if let Some(frame) = tunnel_rx.recv().await {
            let rid = frame.request_id.clone();
            let conn = rc.registry().get("m1").await.unwrap();
            let f = TunnelFrame {
                request_id: rid.clone(),
                frame_type: FrameType::StreamData as i32,
                timestamp: None,
                payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                    StreamPayload {
                        method: String::new(),
                        encrypted: Some(EncryptedPayload {
                            ciphertext: encode_msg(&encrypted_event),
                            nonce: Vec::new(),
                            ephemeral_pubkey: Vec::new(),
                        }),
                        sequence: 0,
                        metadata: HashMap::new(),
                    },
                )),
            };
            conn.send_stream_frame(&rid, f).await;
            conn.complete_stream(&rid).await;
        }
    });

    let req = make_request(
        ResumeSessionRequest {
            session_id: "s1".into(),
            from_sequence: 0,
        },
        "m1",
    );
    let resp = svc.resume_session(req).await.unwrap();
    let mut stream = resp.into_inner();
    let mut events = vec![];
    while let Some(result) = stream.next().await {
        events.push(result.unwrap());
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence, 42);

    // Verify the Encrypted variant passes through with ciphertext intact
    match &events[0].event {
        Some(betcode_proto::v1::agent_event::Event::Encrypted(env)) => {
            assert_eq!(env.ciphertext, opaque_ciphertext, "ciphertext should pass through relay unchanged");
            assert_eq!(env.nonce, opaque_nonce, "nonce should pass through relay unchanged");
        }
        other => panic!("Expected Encrypted event, got {:?}", other),
    }
}
