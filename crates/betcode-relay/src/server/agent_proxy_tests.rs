//! Tests for `AgentProxyService`.

use std::sync::Arc;

use tokio_stream::StreamExt;
use tonic::Request;

use betcode_proto::v1::agent_service_server::AgentService;
use betcode_proto::v1::{
    AgentEvent, CancelTurnRequest, CancelTurnResponse, CompactSessionRequest,
    CompactSessionResponse, InputLockRequest, InputLockResponse, KeyExchangeRequest,
    KeyExchangeResponse, ListSessionsRequest, ListSessionsResponse, ResumeSessionRequest,
    SessionSummary,
};

use super::{AgentProxyService, extract_machine_id};
use crate::server::test_helpers::{
    assert_daemon_error, assert_no_claims_error, assert_no_machine_error, assert_offline_error,
    make_request, proxy_test_setup, spawn_responder, spawn_stream_responder, stream_data_frame,
};

proxy_test_setup!(AgentProxyService);

/// Send a `ResumeSessionRequest` and collect all streamed `AgentEvent`s.
async fn collect_resume_events(
    svc: &AgentProxyService,
    session_id: &str,
    machine_id: &str,
) -> Vec<AgentEvent> {
    let req = make_request(
        ResumeSessionRequest {
            session_id: session_id.into(),
            from_sequence: 0,
        },
        machine_id,
    );
    let resp = svc.resume_session(req).await.unwrap();
    let mut stream = resp.into_inner();
    let mut events = vec![];
    while let Some(result) = stream.next().await {
        events.push(result.unwrap());
    }
    events
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
            sessions: vec![SessionSummary {
                id: "sess-42".into(),
                model: "gpt-4".into(),
                working_directory: "/home/dev".into(),
                status: "active".into(),
                message_count: 5,
                ..Default::default()
            }],
            total: 1,
        },
    );
    let req = make_request(
        ListSessionsRequest {
            working_directory: "/home/dev".into(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_sessions(req).await.unwrap().into_inner();
    assert_eq!(resp.sessions.len(), 1);
    assert_eq!(resp.sessions[0].id, "sess-42");
    assert_eq!(resp.sessions[0].model, "gpt-4");
    assert_eq!(resp.sessions[0].message_count, 5);
    assert_eq!(resp.total, 1);
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
    assert_offline_error!(
        svc,
        list_sessions,
        ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_machine_error!(
        svc,
        list_sessions,
        ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_claims_error!(
        svc,
        list_sessions,
        ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    assert_daemon_error!(
        svc,
        list_sessions,
        ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        },
        router,
        rx,
        "daemon crashed"
    );
}

// --- Streaming ---

#[tokio::test]
async fn resume_session_streams_events() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let events: Vec<AgentEvent> = (0u64..2)
        .map(|seq| AgentEvent {
            sequence: seq,
            ..Default::default()
        })
        .collect();
    spawn_stream_responder(&router, "m1", rx, events);
    let events = collect_resume_events(&svc, "s1", "m1").await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, 0);
    assert_eq!(events[1].sequence, 1);
}

#[tokio::test]
async fn resume_session_offline_returns_unavailable() {
    let svc = setup_offline().await;
    assert_offline_error!(
        svc,
        resume_session,
        ResumeSessionRequest {
            session_id: "s1".into(),
            from_sequence: 0,
        }
    );
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
    assert_offline_error!(
        svc,
        exchange_keys,
        KeyExchangeRequest {
            machine_id: "m-off".into(),
            identity_pubkey: Vec::new(),
            fingerprint: String::new(),
            ephemeral_pubkey: vec![0u8; 32],
        }
    );
}

// --- Encrypted event passthrough ---

#[tokio::test]
async fn encrypted_agent_event_forwards_through_proxy() {
    let (svc, router, mut tunnel_rx) = setup_with_machine("m1").await;
    let rc = Arc::clone(&router);

    // Prepare an AgentEvent with the Encrypted variant (opaque ciphertext bytes)
    let opaque_ciphertext = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
    let opaque_nonce = vec![
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC,
    ];
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
            let f = stream_data_frame(&rid, &encrypted_event, 0);
            conn.send_stream_frame(&rid, f).await;
            conn.complete_stream(&rid).await;
        }
    });

    let events = collect_resume_events(&svc, "s1", "m1").await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence, 42);

    // Verify the Encrypted variant passes through with ciphertext intact
    match &events[0].event {
        Some(betcode_proto::v1::agent_event::Event::Encrypted(env)) => {
            assert_eq!(
                env.ciphertext, opaque_ciphertext,
                "ciphertext should pass through relay unchanged"
            );
            assert_eq!(
                env.nonce, opaque_nonce,
                "nonce should pass through relay unchanged"
            );
        }
        other => panic!("Expected Encrypted event, got {other:?}"),
    }
}
