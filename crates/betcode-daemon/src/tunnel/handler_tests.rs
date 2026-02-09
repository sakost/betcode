use super::*;
use crate::subprocess::SubprocessManager;

async fn make_handler() -> TunnelRequestHandler {
    let db = Database::open_in_memory().await.unwrap();
    let sub = Arc::new(SubprocessManager::new(5));
    let mux = Arc::new(SessionMultiplexer::with_defaults());
    let relay = Arc::new(SessionRelay::new(sub, Arc::clone(&mux), db.clone()));
    let (outbound_tx, _outbound_rx) = mpsc::channel(128);
    TunnelRequestHandler::new(
        "test-machine".into(),
        relay,
        mux,
        db,
        outbound_tx,
        None,
        None,
    )
}

fn req_frame(rid: &str, method: &str, data: Vec<u8>) -> TunnelFrame {
    TunnelFrame {
        request_id: rid.into(),
        frame_type: FrameType::Request as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: method.into(),
                encrypted: Some(betcode_proto::v1::EncryptedPayload {
                    ciphertext: data,
                    nonce: Vec::new(),
                    ephemeral_pubkey: Vec::new(),
                }),
                sequence: 0,
                metadata: HashMap::new(),
            },
        )),
    }
}

fn encode<M: Message>(msg: &M) -> Vec<u8> {
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    buf
}

#[tokio::test]
async fn control_frame_returns_empty() {
    let h = make_handler().await;
    let f = TunnelFrame {
        request_id: "c1".into(),
        frame_type: FrameType::Control as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::Control(
            betcode_proto::v1::TunnelControl {
                control_type: betcode_proto::v1::TunnelControlType::Ping as i32,
                params: HashMap::new(),
            },
        )),
    };
    assert!(h.handle_frame(f).await.is_empty());
}

#[tokio::test]
async fn error_frame_returns_empty() {
    let h = make_handler().await;
    let f = TunnelFrame {
        request_id: "e1".into(),
        frame_type: FrameType::Error as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::Error(
            TunnelError {
                code: TunnelErrorCode::Internal as i32,
                message: "test".into(),
                details: HashMap::new(),
            },
        )),
    };
    assert!(h.handle_frame(f).await.is_empty());
}

#[tokio::test]
async fn request_without_payload_returns_error() {
    let h = make_handler().await;
    let f = TunnelFrame {
        request_id: "r1".into(),
        frame_type: FrameType::Request as i32,
        timestamp: None,
        payload: None,
    };
    let r = h.handle_frame(f).await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn unknown_method_returns_error() {
    let h = make_handler().await;
    let r = h
        .handle_frame(req_frame("r2", "Unknown/Method", vec![]))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn unexpected_frame_type_returns_error() {
    let h = make_handler().await;
    let f = TunnelFrame {
        request_id: "r3".into(),
        frame_type: FrameType::Response as i32,
        timestamp: None,
        payload: None,
    };
    let r = h.handle_frame(f).await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn list_sessions_empty() {
    let h = make_handler().await;
    let req = ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    };
    let r = h
        .handle_frame(req_frame("ls1", METHOD_LIST_SESSIONS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp =
            ListSessionsResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
                .unwrap();
        assert_eq!(resp.sessions.len(), 0);
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn cancel_turn_no_session() {
    let h = make_handler().await;
    let req = CancelTurnRequest {
        session_id: "none".into(),
    };
    let r = h
        .handle_frame(req_frame("ct1", METHOD_CANCEL_TURN, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp = CancelTurnResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
            .unwrap();
        assert!(!resp.was_active);
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn compact_session_no_messages() {
    let h = make_handler().await;
    h.db.create_session("s1", "model", "/tmp").await.unwrap();
    let req = CompactSessionRequest {
        session_id: "s1".into(),
    };
    let r = h
        .handle_frame(req_frame("cs1", METHOD_COMPACT_SESSION, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp =
            CompactSessionResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
                .unwrap();
        assert_eq!(resp.messages_before, 0);
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn request_input_lock_grants() {
    let h = make_handler().await;
    h.db.create_session("sl", "model", "/tmp").await.unwrap();
    let req = InputLockRequest {
        session_id: "sl".into(),
    };
    let r = h
        .handle_frame(req_frame("il1", METHOD_REQUEST_INPUT_LOCK, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp =
            InputLockResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice()).unwrap();
        assert!(resp.granted);
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn malformed_data_returns_error() {
    let h = make_handler().await;
    let r = h
        .handle_frame(req_frame("bad", METHOD_LIST_SESSIONS, vec![0xFF, 0xFF]))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn stream_data_frame_returns_empty() {
    let h = make_handler().await;
    let f = TunnelFrame {
        request_id: "sd".into(),
        frame_type: FrameType::StreamData as i32,
        timestamp: None,
        payload: None,
    };
    assert!(h.handle_frame(f).await.is_empty());
}

// --- Sprint 4.3: Streaming tests ---

/// Helper that returns handler + outbound receiver for streaming tests.
async fn make_handler_with_outbound() -> (TunnelRequestHandler, mpsc::Receiver<TunnelFrame>) {
    let db = Database::open_in_memory().await.unwrap();
    let sub = Arc::new(SubprocessManager::new(5));
    let mux = Arc::new(SessionMultiplexer::with_defaults());
    let relay = Arc::new(SessionRelay::new(sub, Arc::clone(&mux), db.clone()));
    let (outbound_tx, outbound_rx) = mpsc::channel(128);
    let handler = TunnelRequestHandler::new(
        "test-machine".into(),
        relay,
        mux,
        db,
        outbound_tx,
        None,
        None,
    );
    (handler, outbound_rx)
}

fn make_start_request(session_id: &str) -> Vec<u8> {
    use betcode_proto::v1::{agent_request::Request, StartConversation};
    let req = AgentRequest {
        request: Some(Request::Start(StartConversation {
            session_id: session_id.into(),
            working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
            model: "test-model".into(),
            allowed_tools: vec![],
            plan_mode: false,
            worktree_id: String::new(),
            metadata: HashMap::new(),
        })),
    };
    encode(&req)
}

#[tokio::test]
async fn has_active_stream_false_by_default() {
    let h = make_handler().await;
    assert!(!h.has_active_stream("nonexistent").await);
}

#[tokio::test]
async fn incoming_stream_data_unknown_request_is_noop() {
    let h = make_handler().await;
    // Should not panic or error for unknown request_id
    h.handle_incoming_stream_data("unknown-req", &[1, 2, 3])
        .await;
}

#[tokio::test]
async fn converse_bad_data_sends_error_on_outbound() {
    let (h, mut rx) = make_handler_with_outbound().await;
    let result = h
        .handle_frame(req_frame("conv1", METHOD_CONVERSE, vec![0xFF, 0xFF]))
        .await;
    assert!(result.is_empty()); // Converse always returns empty

    // Error should arrive on outbound channel
    let frame = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.frame_type, FrameType::Error as i32);
    assert_eq!(frame.request_id, "conv1");
}

#[tokio::test]
async fn converse_non_start_sends_error_on_outbound() {
    let (h, mut rx) = make_handler_with_outbound().await;
    // Send a UserMessage instead of StartConversation
    use betcode_proto::v1::{agent_request::Request, UserMessage};
    let req = AgentRequest {
        request: Some(Request::Message(UserMessage {
            content: "hello".into(),
            attachments: vec![],
        })),
    };
    let result = h
        .handle_frame(req_frame("conv2", METHOD_CONVERSE, encode(&req)))
        .await;
    assert!(result.is_empty());

    let frame = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.frame_type, FrameType::Error as i32);
    assert_eq!(frame.request_id, "conv2");
}

#[tokio::test]
async fn converse_start_session_failure_sends_error_cleans_active() {
    // Use max_processes=0 so spawn always fails with PoolExhausted
    let db = Database::open_in_memory().await.unwrap();
    let sub = Arc::new(SubprocessManager::new(0));
    let mux = Arc::new(SessionMultiplexer::with_defaults());
    let relay = Arc::new(SessionRelay::new(sub, Arc::clone(&mux), db.clone()));
    let (outbound_tx, mut rx) = mpsc::channel(128);
    let h = TunnelRequestHandler::new(
        "test-machine".into(),
        relay,
        mux,
        db,
        outbound_tx,
        None,
        None,
    );

    // Send StartConversation — this defers the subprocess spawn
    let data = make_start_request("sess-fail");
    let result = h
        .handle_frame(req_frame("conv3", METHOD_CONVERSE, data))
        .await;
    assert!(result.is_empty());

    // Send a UserMessage via StreamData to trigger the deferred spawn failure
    use betcode_proto::v1::{agent_request::Request, UserMessage};
    let user_msg = AgentRequest {
        request: Some(Request::Message(UserMessage {
            content: "hello".into(),
            attachments: vec![],
        })),
    };
    let stream_frame = TunnelFrame {
        request_id: "conv3".into(),
        frame_type: FrameType::StreamData as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: String::new(),
                encrypted: Some(betcode_proto::v1::EncryptedPayload {
                    ciphertext: encode(&user_msg),
                    nonce: Vec::new(),
                    ephemeral_pubkey: Vec::new(),
                }),
                sequence: 0,
                metadata: HashMap::new(),
            },
        )),
    };
    let result = h.handle_frame(stream_frame).await;
    assert!(result.is_empty());

    // Error should arrive on outbound (spawn fails with PoolExhausted)
    let frame = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.frame_type, FrameType::Error as i32);
    assert_eq!(frame.request_id, "conv3");

    // Active stream should have been cleaned up
    assert!(!h.has_active_stream("conv3").await);
}

#[tokio::test]
async fn resume_session_returns_empty_vec_async() {
    let (h, _rx) = make_handler_with_outbound().await;
    h.db().create_session("rs1", "model", "/tmp").await.unwrap();
    let req = ResumeSessionRequest {
        session_id: "rs1".into(),
        from_sequence: 0,
    };
    let result = h
        .handle_frame(req_frame("resume1", METHOD_RESUME_SESSION, encode(&req)))
        .await;
    // ResumeSession dispatches async, returns empty vec
    assert!(result.is_empty());
}

#[tokio::test]
async fn resume_session_replays_stored_messages() {
    let (h, mut rx) = make_handler_with_outbound().await;
    h.db().create_session("rs2", "model", "/tmp").await.unwrap();

    // Store some base64-encoded messages (raw bytes encoded as base64)
    let event_data = vec![10, 20, 30]; // Arbitrary bytes
    let b64 = base64_encode(&event_data);
    h.db()
        .insert_message("rs2", 1, "stream_event", &b64)
        .await
        .unwrap();
    h.db()
        .insert_message("rs2", 2, "stream_event", &b64)
        .await
        .unwrap();

    let req = ResumeSessionRequest {
        session_id: "rs2".into(),
        from_sequence: 0,
    };
    let result = h
        .handle_frame(req_frame("resume2", METHOD_RESUME_SESSION, encode(&req)))
        .await;
    assert!(result.is_empty());

    // Should receive 2 StreamData frames + 1 StreamEnd
    let mut stream_data_count = 0;
    let mut got_stream_end = false;
    for _ in 0..3 {
        let frame = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match FrameType::try_from(frame.frame_type) {
            Ok(FrameType::StreamData) => stream_data_count += 1,
            Ok(FrameType::StreamEnd) => got_stream_end = true,
            _ => panic!("Unexpected frame type: {}", frame.frame_type),
        }
        assert_eq!(frame.request_id, "resume2");
    }
    assert_eq!(stream_data_count, 2);
    assert!(got_stream_end);
}

#[tokio::test]
async fn resume_session_empty_sends_stream_end_only() {
    let (h, mut rx) = make_handler_with_outbound().await;
    h.db().create_session("rs3", "model", "/tmp").await.unwrap();

    let req = ResumeSessionRequest {
        session_id: "rs3".into(),
        from_sequence: 0,
    };
    h.handle_frame(req_frame("resume3", METHOD_RESUME_SESSION, encode(&req)))
        .await;

    // Should just get StreamEnd (no messages stored)
    let frame = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.frame_type, FrameType::StreamEnd as i32);
    assert_eq!(frame.request_id, "resume3");
}

use betcode_core::db::base64_encode;
use betcode_crypto::CryptoSession;

// --- E2E encryption tests ---

/// Helper that returns handler with crypto + client session + outbound receiver.
async fn make_handler_with_crypto() -> (
    TunnelRequestHandler,
    Arc<CryptoSession>,
    mpsc::Receiver<TunnelFrame>,
) {
    let db = Database::open_in_memory().await.unwrap();
    let sub = Arc::new(SubprocessManager::new(5));
    let mux = Arc::new(SessionMultiplexer::with_defaults());
    let relay = Arc::new(SessionRelay::new(sub, Arc::clone(&mux), db.clone()));
    let (outbound_tx, outbound_rx) = mpsc::channel(128);

    let (client_session, server_session) = betcode_crypto::test_session_pair().unwrap();
    let server_crypto = Arc::new(server_session);
    let client_crypto = Arc::new(client_session);

    let handler = TunnelRequestHandler::new(
        "test-machine".into(),
        relay,
        mux,
        db,
        outbound_tx,
        Some(server_crypto),
        None,
    );
    (handler, client_crypto, outbound_rx)
}

fn encrypted_req_frame(
    rid: &str,
    method: &str,
    data: Vec<u8>,
    crypto: &CryptoSession,
) -> TunnelFrame {
    let encrypted = crypto.encrypt(&data).unwrap();
    TunnelFrame {
        request_id: rid.into(),
        frame_type: FrameType::Request as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: method.into(),
                encrypted: Some(betcode_proto::v1::EncryptedPayload {
                    ciphertext: encrypted.ciphertext,
                    nonce: encrypted.nonce.to_vec(),
                    ephemeral_pubkey: Vec::new(),
                }),
                sequence: 0,
                metadata: HashMap::new(),
            },
        )),
    }
}

#[tokio::test]
async fn handler_decrypts_incoming_encrypted_request() {
    let (h, client_crypto, _rx) = make_handler_with_crypto().await;
    let req = ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    };
    let r = h
        .handle_frame(encrypted_req_frame(
            "els1",
            METHOD_LIST_SESSIONS,
            encode(&req),
            &client_crypto,
        ))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    // Response should be encrypted — verify we can decrypt it
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let enc = p.encrypted.as_ref().unwrap();
        assert!(
            !enc.nonce.is_empty(),
            "nonce must be present for encrypted response"
        );
        let decrypted = client_crypto.decrypt(&enc.ciphertext, &enc.nonce).unwrap();
        let resp = ListSessionsResponse::decode(decrypted.as_slice()).unwrap();
        assert_eq!(resp.sessions.len(), 0);
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn handler_encrypts_outgoing_response() {
    let (h, client_crypto, _rx) = make_handler_with_crypto().await;
    let req = CancelTurnRequest {
        session_id: "none".into(),
    };
    let r = h
        .handle_frame(encrypted_req_frame(
            "ect1",
            METHOD_CANCEL_TURN,
            encode(&req),
            &client_crypto,
        ))
        .await;
    assert_eq!(r.len(), 1);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let enc = p.encrypted.as_ref().unwrap();
        // Nonce should be non-empty (encrypted)
        assert!(!enc.nonce.is_empty());
        // Raw protobuf decode on ciphertext should fail
        assert!(CancelTurnResponse::decode(enc.ciphertext.as_slice()).is_err());
        // But decrypting first should work
        let decrypted = client_crypto.decrypt(&enc.ciphertext, &enc.nonce).unwrap();
        let resp = CancelTurnResponse::decode(decrypted.as_slice()).unwrap();
        assert!(!resp.was_active);
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn handler_rejects_encrypted_with_bad_key() {
    let (h, _client_crypto, _rx) = make_handler_with_crypto().await;
    // Encrypt with a completely different session — handler cannot decrypt
    let (wrong_session, _) = betcode_crypto::test_session_pair().unwrap();
    let req = ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    };
    let r = h
        .handle_frame(encrypted_req_frame(
            "ebk1",
            METHOD_LIST_SESSIONS,
            encode(&req),
            &wrong_session,
        ))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn handler_rejects_tampered_ciphertext() {
    let (h, client_crypto, _rx) = make_handler_with_crypto().await;
    let req = ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    };
    let mut frame = encrypted_req_frame("etc1", METHOD_LIST_SESSIONS, encode(&req), &client_crypto);
    // Tamper with the ciphertext
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(ref mut p)) = frame.payload {
        if let Some(ref mut enc) = p.encrypted {
            if let Some(byte) = enc.ciphertext.first_mut() {
                *byte ^= 0xFF;
            }
        }
    }
    let r = h.handle_frame(frame).await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

// --- Key exchange tests ---

use betcode_crypto::{IdentityKeyPair, KeyExchangeState};
use betcode_proto::v1::{KeyExchangeRequest, KeyExchangeResponse};

/// Helper that creates a handler with identity keypair (for key exchange).
async fn make_handler_with_identity() -> (TunnelRequestHandler, mpsc::Receiver<TunnelFrame>) {
    let db = Database::open_in_memory().await.unwrap();
    let sub = Arc::new(SubprocessManager::new(5));
    let mux = Arc::new(SessionMultiplexer::with_defaults());
    let relay = Arc::new(SessionRelay::new(sub, Arc::clone(&mux), db.clone()));
    let (outbound_tx, outbound_rx) = mpsc::channel(128);
    let identity = Arc::new(IdentityKeyPair::generate());
    let handler = TunnelRequestHandler::new(
        "test-machine".into(),
        relay,
        mux,
        db,
        outbound_tx,
        None, // No crypto yet — will be set by key exchange
        Some(identity),
    );
    (handler, outbound_rx)
}

#[tokio::test]
async fn exchange_keys_establishes_crypto_session() {
    let (h, _rx) = make_handler_with_identity().await;

    // Client generates ephemeral keypair
    let client_state = KeyExchangeState::new();
    let client_pub = client_state.public_bytes();

    let req = KeyExchangeRequest {
        machine_id: "test-machine".into(),
        identity_pubkey: Vec::new(),
        fingerprint: String::new(),
        ephemeral_pubkey: client_pub.to_vec(),
    };

    let r = h
        .handle_frame(req_frame("kex1", METHOD_EXCHANGE_KEYS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);

    // Parse response
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp = KeyExchangeResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
            .unwrap();

        assert_eq!(resp.daemon_ephemeral_pubkey.len(), 32);
        assert!(!resp.daemon_fingerprint.is_empty());

        // Client completes key exchange
        let client_session = client_state
            .complete(&resp.daemon_ephemeral_pubkey)
            .unwrap();

        // Now send an encrypted request — it should be decryptable
        let list_req = ListSessionsRequest {
            working_directory: String::new(),
            worktree_id: String::new(),
            limit: 10,
            offset: 0,
        };
        let r2 = h
            .handle_frame(encrypted_req_frame(
                "els-after-kex",
                METHOD_LIST_SESSIONS,
                encode(&list_req),
                &client_session,
            ))
            .await;
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].frame_type, FrameType::Response as i32);

        // Response should be encrypted — verify we can decrypt
        if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p2)) = &r2[0].payload {
            let enc = p2.encrypted.as_ref().unwrap();
            assert!(
                !enc.nonce.is_empty(),
                "response should be encrypted after key exchange"
            );
            let decrypted = client_session.decrypt(&enc.ciphertext, &enc.nonce).unwrap();
            let resp2 = ListSessionsResponse::decode(decrypted.as_slice()).unwrap();
            assert_eq!(resp2.sessions.len(), 0);
        } else {
            panic!("wrong payload");
        }
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn exchange_keys_rejects_invalid_pubkey_length() {
    let (h, _rx) = make_handler_with_identity().await;

    let req = KeyExchangeRequest {
        machine_id: "test-machine".into(),
        identity_pubkey: Vec::new(),
        fingerprint: String::new(),
        ephemeral_pubkey: vec![0u8; 16], // Wrong length
    };

    let r = h
        .handle_frame(req_frame("kex-bad", METHOD_EXCHANGE_KEYS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn exchange_keys_without_identity_still_works() {
    // Handler without identity keypair — key exchange still produces valid session
    let (h, _rx) = make_handler_with_outbound().await;

    let client_state = KeyExchangeState::new();
    let client_pub = client_state.public_bytes();

    let req = KeyExchangeRequest {
        machine_id: "test-machine".into(),
        identity_pubkey: Vec::new(),
        fingerprint: String::new(),
        ephemeral_pubkey: client_pub.to_vec(),
    };

    let r = h
        .handle_frame(req_frame("kex-noid", METHOD_EXCHANGE_KEYS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);

    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp = KeyExchangeResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
            .unwrap();
        assert_eq!(resp.daemon_ephemeral_pubkey.len(), 32);
        // No identity = empty fingerprint
        assert!(resp.daemon_fingerprint.is_empty());
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn concurrent_key_exchanges_do_not_corrupt_session() {
    let (h, _rx) = make_handler_with_identity().await;
    let h = Arc::new(h);

    // Launch 5 concurrent key exchanges
    let mut handles = Vec::new();
    for i in 0..5 {
        let handler = Arc::clone(&h);
        handles.push(tokio::spawn(async move {
            let client_state = KeyExchangeState::new();
            let client_pub = client_state.public_bytes();

            let req = KeyExchangeRequest {
                machine_id: "test-machine".into(),
                identity_pubkey: Vec::new(),
                fingerprint: String::new(),
                ephemeral_pubkey: client_pub.to_vec(),
            };

            let r = handler
                .handle_frame(req_frame(
                    &format!("kex-concurrent-{}", i),
                    METHOD_EXCHANGE_KEYS,
                    encode(&req),
                ))
                .await;
            assert_eq!(r.len(), 1);
            assert_eq!(r[0].frame_type, FrameType::Response as i32);

            // Parse response and complete exchange
            if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
                let resp = KeyExchangeResponse::decode(
                    p.encrypted.as_ref().unwrap().ciphertext.as_slice(),
                )
                .unwrap();
                let session = client_state
                    .complete(&resp.daemon_ephemeral_pubkey)
                    .unwrap();
                (i, session)
            } else {
                panic!("wrong payload for exchange {}", i);
            }
        }));
    }

    // Collect all results — all should succeed (no panics)
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.unwrap());
    }

    // The last exchange to complete is the one installed. Verify the handler
    // has a valid crypto session by sending an encrypted request with it.
    // We can't know which exchange "won", but the handler should not be in
    // a corrupt state. Try each session until one works.
    let list_req = ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    };
    let mut any_succeeded = false;
    for (_i, session) in &results {
        let r = h
            .handle_frame(encrypted_req_frame(
                "verify-after-concurrent",
                METHOD_LIST_SESSIONS,
                encode(&list_req),
                session,
            ))
            .await;
        if r.len() == 1 && r[0].frame_type == FrameType::Response as i32 {
            any_succeeded = true;
            break;
        }
    }
    assert!(
        any_succeeded,
        "None of the concurrent exchange sessions could decrypt — handler state corrupted"
    );
}

#[tokio::test]
async fn request_with_empty_encrypted_payload_is_handled() {
    // No crypto session — empty ciphertext should passthrough as empty data
    let h = make_handler().await;
    let req = ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    };
    // Send valid protobuf data as "ciphertext" (passthrough mode)
    let r = h
        .handle_frame(req_frame("ep1", METHOD_LIST_SESSIONS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
}

#[tokio::test]
async fn request_with_none_encrypted_field_uses_empty_data() {
    let h = make_handler().await;
    // Frame with encrypted=None — should result in empty data, which decodes
    // to default protobuf values (empty ListSessionsRequest)
    let frame = TunnelFrame {
        request_id: "ne1".into(),
        frame_type: FrameType::Request as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: METHOD_LIST_SESSIONS.into(),
                encrypted: None,
                sequence: 0,
                metadata: HashMap::new(),
            },
        )),
    };
    let r = h.handle_frame(frame).await;
    assert_eq!(r.len(), 1);
    // Empty data decodes to default ListSessionsRequest → successful response
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
}
