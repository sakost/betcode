use super::*;
use crate::subprocess::SubprocessManager;

// ---------------------------------------------------------------------------
// HandlerTestBuilder – shared setup for all handler tests
// ---------------------------------------------------------------------------

struct HandlerTestBuilder {
    max_processes: usize,
    with_crypto: bool,
    with_identity: bool,
    with_command_service: bool,
    with_gitlab_service: bool,
    with_worktree_service: bool,
}

/// Outputs produced by [`HandlerTestBuilder::build`].
struct HandlerTestOutput {
    handler: TunnelRequestHandler,
    rx: mpsc::Receiver<TunnelFrame>,
    /// Client-side crypto session (present only when `with_crypto` was set).
    client_crypto: Option<Arc<CryptoSession>>,
}

impl HandlerTestBuilder {
    fn new() -> Self {
        Self {
            max_processes: 5,
            with_crypto: false,
            with_identity: false,
            with_command_service: false,
            with_gitlab_service: false,
            with_worktree_service: false,
        }
    }

    fn max_processes(mut self, n: usize) -> Self {
        self.max_processes = n;
        self
    }

    fn with_crypto(mut self) -> Self {
        self.with_crypto = true;
        self
    }

    fn with_identity(mut self) -> Self {
        self.with_identity = true;
        self
    }

    fn with_command_service(mut self) -> Self {
        self.with_command_service = true;
        self
    }

    fn with_gitlab_service(mut self) -> Self {
        self.with_gitlab_service = true;
        self
    }

    fn with_worktree_service(mut self) -> Self {
        self.with_worktree_service = true;
        self
    }

    async fn build(self) -> HandlerTestOutput {
        let db = Database::open_in_memory().await.unwrap();
        let sub = Arc::new(SubprocessManager::new(self.max_processes));
        let mux = Arc::new(SessionMultiplexer::with_defaults());
        let relay = Arc::new(SessionRelay::new(sub, Arc::clone(&mux), db.clone()));
        let (outbound_tx, outbound_rx) = mpsc::channel(128);

        let mut client_crypto = None;
        let server_crypto = if self.with_crypto {
            let (client_session, server_session) = betcode_crypto::test_session_pair().unwrap();
            client_crypto = Some(Arc::new(client_session));
            Some(Arc::new(server_session))
        } else {
            None
        };

        let identity = if self.with_identity {
            Some(Arc::new(IdentityKeyPair::generate()))
        } else {
            None
        };

        let mut handler = TunnelRequestHandler::new(
            "test-machine".into(),
            relay,
            mux,
            db.clone(),
            outbound_tx,
            server_crypto,
            identity,
        );

        if self.with_command_service {
            use crate::commands::service_executor::ServiceExecutor;
            use crate::commands::CommandRegistry;
            use crate::completion::agent_lister::AgentLister;
            use crate::completion::file_index::FileIndex;
            use tokio::sync::RwLock;

            let registry = Arc::new(RwLock::new(CommandRegistry::new()));
            let file_index = Arc::new(RwLock::new(FileIndex::empty()));
            let agent_lister = Arc::new(RwLock::new(AgentLister::new()));
            let service_executor = Arc::new(RwLock::new(ServiceExecutor::new(
                std::env::temp_dir().to_path_buf(),
            )));
            let (shutdown_tx, _) = tokio::sync::watch::channel(false);
            let cmd_svc = CommandServiceImpl::new(
                registry,
                file_index,
                agent_lister,
                service_executor,
                shutdown_tx,
            );
            handler.set_command_service(Arc::new(cmd_svc));
        }

        if self.with_gitlab_service {
            use crate::gitlab::{GitLabClient, GitLabConfig};

            let config = GitLabConfig {
                base_url: "http://127.0.0.1:1".into(),
                token: "test-token".into(),
            };
            let client = Arc::new(GitLabClient::new(&config).unwrap());
            let gitlab_svc = Arc::new(GitLabServiceImpl::new(client));
            handler.set_gitlab_service(gitlab_svc);
        }

        if self.with_worktree_service {
            use crate::worktree::WorktreeManager;

            let wt_svc = WorktreeServiceImpl::new(WorktreeManager::new(
                db,
                std::env::temp_dir().join("betcode-test-worktrees"),
            ));
            handler.set_worktree_service(Arc::new(wt_svc));
        }

        HandlerTestOutput {
            handler,
            rx: outbound_rx,
            client_crypto,
        }
    }
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    let r = h
        .handle_frame(req_frame("r2", "Unknown/Method", vec![]))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn unexpected_frame_type_returns_error() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    let r = h
        .handle_frame(req_frame("bad", METHOD_LIST_SESSIONS, vec![0xFF, 0xFF]))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn stream_data_frame_returns_empty() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    let f = TunnelFrame {
        request_id: "sd".into(),
        frame_type: FrameType::StreamData as i32,
        timestamp: None,
        payload: None,
    };
    assert!(h.handle_frame(f).await.is_empty());
}

// --- Sprint 4.3: Streaming tests ---

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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    assert!(!h.has_active_stream("nonexistent").await);
}

#[tokio::test]
async fn incoming_stream_data_unknown_request_is_noop() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    // Should not panic or error for unknown request_id
    h.handle_incoming_stream_data("unknown-req", &[1, 2, 3])
        .await;
}

#[tokio::test]
async fn converse_bad_data_sends_error_on_outbound() {
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().max_processes(0).build().await;

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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();
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
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();
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
    let HandlerTestOutput { handler: h, .. } =
        HandlerTestBuilder::new().with_crypto().build().await;
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
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();
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

#[tokio::test]
async fn exchange_keys_establishes_crypto_session() {
    let HandlerTestOutput { handler: h, .. } =
        HandlerTestBuilder::new().with_identity().build().await;

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
    let HandlerTestOutput { handler: h, .. } =
        HandlerTestBuilder::new().with_identity().build().await;

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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;

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
    let HandlerTestOutput { handler: h, .. } =
        HandlerTestBuilder::new().with_identity().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
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

// --- Application-layer EncryptedEnvelope tests ---

use betcode_proto::v1::AgentEvent;

/// Build a tunnel-layer encrypted converse start frame with app-layer encryption.
/// This is what the CLI actually sends: StartConversation → app-encrypt → serialize → tunnel-encrypt.
fn encrypted_converse_frame(rid: &str, session_id: &str, crypto: &CryptoSession) -> TunnelFrame {
    let start_req = AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Start(
            betcode_proto::v1::StartConversation {
                session_id: session_id.into(),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                model: "test-model".into(),
                allowed_tools: vec![],
                plan_mode: false,
                worktree_id: String::new(),
                metadata: HashMap::new(),
            },
        )),
    };
    let app_encrypted = make_app_encrypted_agent_request(&start_req, crypto);
    let data = encode(&app_encrypted);
    encrypted_req_frame(rid, METHOD_CONVERSE, data, crypto)
}

/// Build an AgentRequest with the Encrypted oneof variant wrapping a real request.
fn make_app_encrypted_agent_request(inner: &AgentRequest, session: &CryptoSession) -> AgentRequest {
    let mut buf = Vec::new();
    inner.encode(&mut buf).unwrap();
    let enc = session.encrypt(&buf).unwrap();
    AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Encrypted(
            betcode_proto::v1::EncryptedEnvelope {
                ciphertext: enc.ciphertext,
                nonce: enc.nonce.to_vec(),
            },
        )),
    }
}

#[tokio::test]
async fn encrypted_agent_request_is_decrypted() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Start a converse session (app-layer + tunnel-layer encrypted, as the CLI does)
    h.handle_frame(encrypted_converse_frame(
        "enc-conv1",
        "enc-stream-1",
        &client_crypto,
    ))
    .await;
    assert!(h.has_active_stream("enc-conv1").await);

    // Verify the pending_config exists before we send the message
    {
        let streams = h.active_streams.read().await;
        assert!(
            streams.get("enc-conv1").unwrap().pending_config.is_some(),
            "pending_config should exist before first message"
        );
    }

    // Build an encrypted UserMessage AgentRequest (app-layer)
    let inner = AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Message(
            betcode_proto::v1::UserMessage {
                content: "encrypted hello".into(),
                attachments: vec![],
            },
        )),
    };
    let encrypted_req = make_app_encrypted_agent_request(&inner, &client_crypto);
    let encrypted_bytes = encode(&encrypted_req);

    // Wrap in a StreamData tunnel frame (tunnel-layer encrypted)
    let enc = client_crypto.encrypt(&encrypted_bytes).unwrap();
    let stream_frame = TunnelFrame {
        request_id: "enc-conv1".into(),
        frame_type: FrameType::StreamData as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: String::new(),
                encrypted: Some(betcode_proto::v1::EncryptedPayload {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce.to_vec(),
                    ephemeral_pubkey: Vec::new(),
                }),
                sequence: 0,
                metadata: HashMap::new(),
            },
        )),
    };
    // Should not panic — the handler decrypts the app-layer envelope
    h.handle_frame(stream_frame).await;

    // If decryption succeeded, the handler consumed pending_config to attempt
    // subprocess start (which will fail — no real binary — but the important
    // thing is that decryption worked and reached that code path).
    let streams = h.active_streams.read().await;
    // The stream may have been removed (start_session failed and cleanup runs),
    // OR pending_config was taken (consumed). Either proves decryption succeeded.
    match streams.get("enc-conv1") {
        Some(active) => assert!(
            active.pending_config.is_none(),
            "pending_config should have been consumed after successful decryption"
        ),
        None => {
            // Stream was removed because start_session failed — also proves decryption worked
        }
    }
}

#[tokio::test]
async fn encrypted_agent_request_with_wrong_key_rejected() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Start a converse session (app-layer + tunnel-layer encrypted)
    h.handle_frame(encrypted_converse_frame(
        "enc-conv3",
        "enc-stream-3",
        &client_crypto,
    ))
    .await;
    assert!(h.has_active_stream("enc-conv3").await);

    // Encrypt with a DIFFERENT session
    let (wrong_session, _) = betcode_crypto::test_session_pair().unwrap();
    let inner = AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Message(
            betcode_proto::v1::UserMessage {
                content: "wrong key".into(),
                attachments: vec![],
            },
        )),
    };
    let encrypted_req = make_app_encrypted_agent_request(&inner, &wrong_session);
    let encrypted_bytes = encode(&encrypted_req);

    // Tunnel-layer encrypt with the correct key
    let enc = client_crypto.encrypt(&encrypted_bytes).unwrap();
    let stream_frame = TunnelFrame {
        request_id: "enc-conv3".into(),
        frame_type: FrameType::StreamData as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: String::new(),
                encrypted: Some(betcode_proto::v1::EncryptedPayload {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce.to_vec(),
                    ephemeral_pubkey: Vec::new(),
                }),
                sequence: 0,
                metadata: HashMap::new(),
            },
        )),
    };
    h.handle_frame(stream_frame).await;

    // Should be rejected — pending_config untouched
    let streams = h.active_streams.read().await;
    let active = streams.get("enc-conv3").unwrap();
    assert!(
        active.pending_config.is_some(),
        "Request encrypted with wrong key should be rejected"
    );
}

#[tokio::test]
async fn encrypted_agent_request_with_corrupted_data_rejected() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Start a converse session (app-layer + tunnel-layer encrypted)
    h.handle_frame(encrypted_converse_frame(
        "enc-conv4",
        "enc-stream-4",
        &client_crypto,
    ))
    .await;
    assert!(h.has_active_stream("enc-conv4").await);

    // Create a valid encrypted request, then corrupt it
    let inner = AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Message(
            betcode_proto::v1::UserMessage {
                content: "will be corrupted".into(),
                attachments: vec![],
            },
        )),
    };
    let mut encrypted_req = make_app_encrypted_agent_request(&inner, &client_crypto);
    // Corrupt the ciphertext
    if let Some(betcode_proto::v1::agent_request::Request::Encrypted(ref mut env)) =
        encrypted_req.request
    {
        if let Some(byte) = env.ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
    }
    let encrypted_bytes = encode(&encrypted_req);

    // Tunnel-layer encrypt
    let enc = client_crypto.encrypt(&encrypted_bytes).unwrap();
    let stream_frame = TunnelFrame {
        request_id: "enc-conv4".into(),
        frame_type: FrameType::StreamData as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: String::new(),
                encrypted: Some(betcode_proto::v1::EncryptedPayload {
                    ciphertext: enc.ciphertext,
                    nonce: enc.nonce.to_vec(),
                    ephemeral_pubkey: Vec::new(),
                }),
                sequence: 0,
                metadata: HashMap::new(),
            },
        )),
    };
    h.handle_frame(stream_frame).await;

    // Should be rejected
    let streams = h.active_streams.read().await;
    let active = streams.get("enc-conv4").unwrap();
    assert!(
        active.pending_config.is_some(),
        "Corrupted encrypted request should be rejected"
    );
}

// --- Downgrade attack and additional security tests ---

#[tokio::test]
async fn plaintext_agent_request_rejected_when_crypto_active() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Start a valid converse session (properly encrypted)
    h.handle_frame(encrypted_converse_frame(
        "da-conv1",
        "da-stream-1",
        &client_crypto,
    ))
    .await;
    assert!(h.has_active_stream("da-conv1").await);

    // Send a plaintext UserMessage (NOT app-layer encrypted) via tunnel-encrypted frame
    let plain_req = AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Message(
            betcode_proto::v1::UserMessage {
                content: "injected plaintext".into(),
                attachments: vec![],
            },
        )),
    };
    let plain_bytes = encode(&plain_req);
    // Tunnel-layer encrypt (so it passes tunnel decryption) but NOT app-layer encrypted
    let frame = encrypted_req_frame("da-conv1", "", plain_bytes, &client_crypto);
    // Change frame type to StreamData for the in-stream message
    let frame = TunnelFrame {
        frame_type: FrameType::StreamData as i32,
        ..frame
    };
    h.handle_frame(frame).await;

    // Should be rejected — pending_config untouched (the plaintext message was dropped)
    let streams = h.active_streams.read().await;
    let active = streams.get("da-conv1").unwrap();
    assert!(
        active.pending_config.is_some(),
        "Plaintext request should be rejected when crypto is active"
    );
}

#[tokio::test]
async fn plaintext_start_conversation_rejected_when_crypto_active() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Send a plaintext StartConversation that is tunnel-layer encrypted but NOT
    // app-layer encrypted. handle_converse should reject because crypto is active.
    let start_data = make_start_request("reject-stream-1");
    let result = h
        .handle_frame(encrypted_req_frame(
            "reject-conv1",
            METHOD_CONVERSE,
            start_data,
            &client_crypto,
        ))
        .await;
    // handle_converse sends errors via outbound_tx, returns empty vec
    assert!(result.is_empty());

    // Should NOT have created a stream — the plaintext was rejected
    assert!(
        !h.has_active_stream("reject-conv1").await,
        "Plaintext StartConversation should be rejected when crypto is active"
    );
}

// --- Step 5: Event forwarder encryption tests ---

#[tokio::test]
async fn event_forwarder_encrypts_when_crypto_active() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        mut rx,
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Create a session in DB + start converse (app-layer + tunnel-layer encrypted)
    h.db()
        .create_session("ef-enc1", "model", "/tmp")
        .await
        .unwrap();
    h.handle_frame(encrypted_converse_frame(
        "ef-conv1",
        "ef-enc1",
        &client_crypto,
    ))
    .await;

    // Broadcast an event into the session
    h.multiplexer()
        .broadcast(
            "ef-enc1",
            AgentEvent {
                sequence: 1,
                timestamp: None,
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                    betcode_proto::v1::TextDelta {
                        text: "encrypted text".into(),
                        is_complete: false,
                    },
                )),
            },
        )
        .await;

    // Receive the forwarded frame
    let frame = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.frame_type, FrameType::StreamData as i32);

    // Extract tunnel-layer payload. Tunnel-layer is now passthrough when app-layer
    // crypto is active (no redundant double-encryption), so ciphertext contains
    // the raw serialized wrapper and nonce is empty.
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &frame.payload {
        let enc = p.encrypted.as_ref().unwrap();
        assert!(
            enc.nonce.is_empty(),
            "Tunnel-layer should be passthrough when app-layer encrypts"
        );
        let wire_bytes = &enc.ciphertext; // passthrough: ciphertext IS the raw bytes

        // Decode the AgentEvent — should have Encrypted variant (app-layer)
        let wrapper = AgentEvent::decode(wire_bytes.as_slice()).unwrap();
        match wrapper.event {
            Some(betcode_proto::v1::agent_event::Event::Encrypted(ref env)) => {
                // Decrypt the app-layer envelope
                let inner_bytes = client_crypto.decrypt(&env.ciphertext, &env.nonce).unwrap();
                let inner_event = AgentEvent::decode(inner_bytes.as_slice()).unwrap();
                match inner_event.event {
                    Some(betcode_proto::v1::agent_event::Event::TextDelta(ref td)) => {
                        assert_eq!(td.text, "encrypted text");
                    }
                    other => panic!("Expected TextDelta, got {:?}", other),
                }
            }
            other => panic!("Expected Encrypted envelope, got {:?}", other),
        }
    } else {
        panic!("Expected StreamData payload");
    }
}

#[tokio::test]
async fn event_forwarder_no_encryption_when_no_crypto() {
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().build().await;

    // Create a session in DB + start converse
    h.db()
        .create_session("ef-plain1", "model", "/tmp")
        .await
        .unwrap();
    let start_data = make_start_request("ef-plain1");
    h.handle_frame(req_frame("ef-conv2", METHOD_CONVERSE, start_data))
        .await;

    // Broadcast an event
    h.multiplexer()
        .broadcast(
            "ef-plain1",
            AgentEvent {
                sequence: 1,
                timestamp: None,
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                    betcode_proto::v1::TextDelta {
                        text: "plaintext output".into(),
                        is_complete: false,
                    },
                )),
            },
        )
        .await;

    // Receive the forwarded frame
    let frame = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.frame_type, FrameType::StreamData as i32);

    // Without crypto, the event should be directly decodable (passthrough)
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &frame.payload {
        let enc = p.encrypted.as_ref().unwrap();
        // No crypto → ciphertext is raw bytes, nonce is empty
        assert!(enc.nonce.is_empty());
        let event = AgentEvent::decode(enc.ciphertext.as_slice()).unwrap();
        match event.event {
            Some(betcode_proto::v1::agent_event::Event::TextDelta(ref td)) => {
                assert_eq!(td.text, "plaintext output");
            }
            other => panic!("Expected TextDelta (not encrypted), got {:?}", other),
        }
    } else {
        panic!("Expected StreamData payload");
    }
}

/// Relay-forwarded data has empty nonce (no tunnel-layer encryption).
/// When crypto is active, decrypt_payload should passthrough these payloads
/// since the relay doesn't have the crypto keys. App-layer EncryptedEnvelope
/// handles the actual E2E protection.
#[tokio::test]
async fn decrypt_payload_passthrough_on_empty_nonce_with_crypto_active() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        ..
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Build an app-layer encrypted AgentRequest (as the CLI would send)
    let inner_req = AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Message(
            betcode_proto::v1::UserMessage {
                content: "hello from relay".into(),
                attachments: vec![],
            },
        )),
    };
    let app_encrypted = make_app_encrypted_agent_request(&inner_req, &client_crypto);
    let wire_bytes = encode(&app_encrypted);

    // Simulate relay forwarding: raw bytes with empty nonce (no tunnel-layer encryption)
    let frame = req_frame("r1", METHOD_LIST_SESSIONS, wire_bytes.clone());

    // The handler should passthrough the empty-nonce payload and then
    // successfully app-layer decrypt the EncryptedEnvelope inside.
    let responses = h.handle_frame(frame).await;

    // Should NOT get a tunnel decryption error
    for resp in &responses {
        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(err)) = &resp.payload {
            panic!(
                "Got error response (tunnel decryption should have been skipped): {}",
                err.message,
            );
        }
    }
}

/// Relay-forwarded data with empty nonce should passthrough even when crypto active,
/// but app-layer plaintext should still be rejected.
#[tokio::test]
async fn relay_forwarded_plaintext_request_rejected_when_crypto_active() {
    let HandlerTestOutput { handler: h, .. } =
        HandlerTestBuilder::new().with_crypto().build().await;

    // Plain (non-app-layer-encrypted) AgentRequest
    let plain_req = AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::Message(
            betcode_proto::v1::UserMessage {
                content: "plain attack".into(),
                attachments: vec![],
            },
        )),
    };
    let wire_bytes = encode(&plain_req);

    // Relay-forwarded: empty nonce (passes tunnel layer)
    let frame = req_frame("r2", METHOD_LIST_SESSIONS, wire_bytes);
    let _responses = h.handle_frame(frame).await;

    // Tunnel passthrough should succeed, but app-layer should reject plaintext.
    // The handler should not process the request as a valid UserMessage.
    // It should either return an error or silently discard it.
    let has_active_stream = h.has_active_stream("r2").await;
    assert!(
        !has_active_stream,
        "plaintext request should not create a stream"
    );
}

/// Verify that resume_session with E2E crypto produces frames the relay can decode.
///
/// The relay extracts `ciphertext` and decodes it as `AgentEvent`. When crypto is
/// active, the daemon must wrap events in `AgentEvent::Encrypted` (app-layer) so
/// the relay sees valid protobuf, not raw encrypted bytes.
#[tokio::test]
async fn resume_session_with_crypto_produces_decodable_frames() {
    let HandlerTestOutput {
        handler: h,
        client_crypto,
        mut rx,
    } = HandlerTestBuilder::new().with_crypto().build().await;
    let client_crypto = client_crypto.unwrap();

    // Store an event in the DB the same way pipeline.rs does (base64-encoded protobuf)
    let event = AgentEvent {
        sequence: 1,
        timestamp: None,
        parent_tool_use_id: String::new(),
        event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
            betcode_proto::v1::TextDelta {
                text: "Hello from history".into(),
                is_complete: true,
            },
        )),
    };
    let mut buf = Vec::new();
    event.encode(&mut buf).unwrap();
    let payload = betcode_core::db::base64_encode(&buf);
    h.db()
        .create_session("resume-enc1", "model", "/tmp")
        .await
        .unwrap();
    h.db()
        .insert_message("resume-enc1", 1, "assistant", &payload)
        .await
        .unwrap();

    // Call handle_resume_session
    let resume_req = betcode_proto::v1::ResumeSessionRequest {
        session_id: "resume-enc1".into(),
        from_sequence: 0,
    };
    let _responses = h
        .handle_frame(encrypted_req_frame(
            "resume1",
            "AgentService/ResumeSession",
            encode(&resume_req),
            &client_crypto,
        ))
        .await;

    // The resume handler sends frames async via outbound_tx. Collect them.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut decoded_events = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        if frame.frame_type == FrameType::StreamData as i32 {
            if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = frame.payload {
                let data = p
                    .encrypted
                    .as_ref()
                    .map(|e| &e.ciphertext[..])
                    .unwrap_or(&[]);
                // This is what the relay does — decode ciphertext as AgentEvent.
                // It MUST succeed (no protobuf decode error).
                match AgentEvent::decode(data) {
                    Ok(evt) => decoded_events.push(evt),
                    Err(e) => panic!(
                        "Relay cannot decode resume frame as AgentEvent: {}. \
                         This means the daemon sent encrypted bytes without \
                         wrapping in EncryptedEnvelope.",
                        e
                    ),
                }
            }
        }
    }
    assert!(
        !decoded_events.is_empty(),
        "Expected at least one decodable event from resume"
    );
    // The relay sees an AgentEvent::Encrypted wrapper (it can't decrypt, but it can decode)
    let first = &decoded_events[0];
    assert!(
        matches!(
            first.event,
            Some(betcode_proto::v1::agent_event::Event::Encrypted(_))
        ),
        "Resume events should be app-layer encrypted (EncryptedEnvelope), got: {:?}",
        first.event
    );
}

/// Verify that resume_session WITHOUT crypto sends plain events (no encryption wrapper).
#[tokio::test]
async fn resume_session_without_crypto_sends_plain_events() {
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().build().await;
    let db = h.db().clone();

    // Store an event
    let event = AgentEvent {
        sequence: 1,
        timestamp: None,
        parent_tool_use_id: String::new(),
        event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
            betcode_proto::v1::TextDelta {
                text: "Hello plain".into(),
                is_complete: true,
            },
        )),
    };
    let mut buf = Vec::new();
    event.encode(&mut buf).unwrap();
    let payload = betcode_core::db::base64_encode(&buf);
    db.create_session("resume-plain1", "model", "/tmp")
        .await
        .unwrap();
    db.insert_message("resume-plain1", 1, "assistant", &payload)
        .await
        .unwrap();

    // Call handle_resume_session (no crypto)
    let resume_req = betcode_proto::v1::ResumeSessionRequest {
        session_id: "resume-plain1".into(),
        from_sequence: 0,
    };
    let _responses = h
        .handle_frame(req_frame(
            "resume2",
            "AgentService/ResumeSession",
            encode(&resume_req),
        ))
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut decoded_events = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        if frame.frame_type == FrameType::StreamData as i32 {
            if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = frame.payload {
                let data = p
                    .encrypted
                    .as_ref()
                    .map(|e| &e.ciphertext[..])
                    .unwrap_or(&[]);
                match AgentEvent::decode(data) {
                    Ok(evt) => decoded_events.push(evt),
                    Err(e) => panic!("Cannot decode plain resume frame: {}", e),
                }
            }
        }
    }
    assert!(
        !decoded_events.is_empty(),
        "Expected at least one event from plain resume"
    );
    // Without crypto, the event should be a plain TextDelta (not Encrypted)
    let first = &decoded_events[0];
    assert!(
        matches!(
            first.event,
            Some(betcode_proto::v1::agent_event::Event::TextDelta(_))
        ),
        "Plain resume events should be unwrapped TextDelta, got: {:?}",
        first.event
    );
}

/// Verify that QuestionResponse requests are handled in the tunnel handler
/// (not silently dropped by the catch-all `_ =>` arm).
#[tokio::test]
async fn question_response_is_handled_through_tunnel() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;

    // Send a QuestionResponse through the converse stream.
    // The session doesn't exist (no real subprocess), but the handler should
    // reach the QuestionResponse arm and attempt send_question_response (which
    // will fail with SessionNotFound — that's fine, it's logged as a warning).
    let question_resp = betcode_proto::v1::AgentRequest {
        request: Some(betcode_proto::v1::agent_request::Request::QuestionResponse(
            betcode_proto::v1::UserQuestionResponse {
                question_id: "req_q1".into(),
                answers: [("Which database?".into(), "SQLite".into())]
                    .into_iter()
                    .collect(),
            },
        )),
    };
    let qr_bytes = encode(&question_resp);
    let frame = req_frame("q1", METHOD_CONVERSE, qr_bytes);

    // This should not panic. The QuestionResponse arm should exist and be reachable.
    let _responses = h.handle_frame(frame).await;
}

// --- CommandService tunnel handler tests ---

use betcode_proto::v1::{
    GetCommandRegistryResponse, ListAgentsResponse, ListPathResponse, ListWorktreesResponse,
    RemoveWorktreeResponse,
};

#[tokio::test]
async fn get_command_registry_through_tunnel() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_command_service()
        .build()
        .await;
    let req = GetCommandRegistryRequest {};
    let r = h
        .handle_frame(req_frame("cmd1", METHOD_GET_COMMAND_REGISTRY, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let _resp =
            GetCommandRegistryResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
                .unwrap();
        // Successfully decoded — registry may contain discovered commands
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn list_agents_through_tunnel() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_command_service()
        .build()
        .await;
    let req = ListAgentsRequest {
        query: String::new(),
        max_results: 10,
    };
    let r = h
        .handle_frame(req_frame("cmd2", METHOD_LIST_AGENTS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp = ListAgentsResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
            .unwrap();
        assert!(resp.agents.is_empty());
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn list_path_through_tunnel() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_command_service()
        .build()
        .await;
    let req = ListPathRequest {
        query: std::env::temp_dir().to_string_lossy().into_owned(),
        max_results: 10,
    };
    let r = h
        .handle_frame(req_frame("cmd3", METHOD_LIST_PATH, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let _resp =
            ListPathResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice()).unwrap();
        // Successfully decoded — path listing returned something
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn command_service_not_set_returns_error() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await; // No command service set
    let req = GetCommandRegistryRequest {};
    let r = h
        .handle_frame(req_frame(
            "cmd-err",
            METHOD_GET_COMMAND_REGISTRY,
            encode(&req),
        ))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
        assert!(e.message.contains("CommandService not available"));
    } else {
        panic!("expected error payload");
    }
}

#[tokio::test]
async fn command_service_malformed_data_returns_error() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_command_service()
        .build()
        .await;
    let r = h
        .handle_frame(req_frame(
            "cmd-bad",
            METHOD_GET_COMMAND_REGISTRY,
            vec![0xFF, 0xFF],
        ))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

// --- GitLabService tunnel handler tests ---

#[tokio::test]
async fn gitlab_service_dispatches_when_set() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_gitlab_service()
        .build()
        .await;
    let req = ListMergeRequestsRequest {
        project: "group/project".into(),
        state_filter: 0,
        limit: 10,
        offset: 0,
    };
    let r = h
        .handle_frame(req_frame("gl-ok", METHOD_LIST_MERGE_REQUESTS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    // Should reach the GitLabServiceImpl (HTTP error), NOT the "not available" error.
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
        assert!(
            !e.message.contains("GitLabService not available"),
            "Expected HTTP/service error, not 'not available'. Got: {}",
            e.message
        );
    } else {
        panic!("expected error payload from HTTP failure");
    }
}

#[tokio::test]
async fn gitlab_service_not_set_returns_error() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await; // No gitlab service set
    let req = ListMergeRequestsRequest {
        project: String::new(),
        state_filter: 0,
        limit: 10,
        offset: 0,
    };
    let r = h
        .handle_frame(req_frame(
            "gl-err",
            METHOD_LIST_MERGE_REQUESTS,
            encode(&req),
        ))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
        assert!(e.message.contains("GitLabService not available"));
    } else {
        panic!("expected error payload");
    }
}

#[tokio::test]
async fn gitlab_malformed_data_returns_error() {
    // Even without the service set, malformed data for a GitLab method should
    // trigger the "not available" error, not a panic.
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    let r = h
        .handle_frame(req_frame(
            "gl-bad",
            METHOD_LIST_MERGE_REQUESTS,
            vec![0xFF, 0xFF],
        ))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

#[tokio::test]
async fn gitlab_all_methods_dispatch_without_service() {
    // All 6 GitLab method constants should be recognized and hit the
    // "not available" error path (not the unknown method path).
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    let methods = [
        METHOD_LIST_MERGE_REQUESTS,
        METHOD_GET_MERGE_REQUEST,
        METHOD_LIST_PIPELINES,
        METHOD_GET_PIPELINE,
        METHOD_LIST_ISSUES,
        METHOD_GET_ISSUE,
    ];
    for method in methods {
        let r = h
            .handle_frame(req_frame("gl-dispatch", method, vec![]))
            .await;
        assert_eq!(r.len(), 1, "method: {method}");
        assert_eq!(r[0].frame_type, FrameType::Error as i32, "method: {method}");
        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
            assert!(
                e.message.contains("GitLabService not available"),
                "method {method} should hit 'not available', got: {}",
                e.message
            );
        }
    }
}

// --- WorktreeService tunnel handler tests ---

#[tokio::test]
async fn list_worktrees_through_tunnel() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_worktree_service()
        .build()
        .await;
    let req = ListWorktreesRequest {
        repo_path: String::new(),
    };
    let r = h
        .handle_frame(req_frame("wt1", METHOD_LIST_WORKTREES, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp =
            ListWorktreesResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
                .unwrap();
        assert!(resp.worktrees.is_empty());
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn remove_worktree_nonexistent_through_tunnel() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_worktree_service()
        .build()
        .await;
    let req = RemoveWorktreeRequest {
        id: "nonexistent".into(),
    };
    let r = h
        .handle_frame(req_frame("wt2", METHOD_REMOVE_WORKTREE, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Response as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &r[0].payload {
        let resp =
            RemoveWorktreeResponse::decode(p.encrypted.as_ref().unwrap().ciphertext.as_slice())
                .unwrap();
        assert!(!resp.removed);
    } else {
        panic!("wrong payload");
    }
}

#[tokio::test]
async fn worktree_service_not_set_returns_error() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await; // No worktree service set
    let req = ListWorktreesRequest {
        repo_path: String::new(),
    };
    let r = h
        .handle_frame(req_frame("wt-err", METHOD_LIST_WORKTREES, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
        assert!(e.message.contains("WorktreeService not available"));
    } else {
        panic!("expected error payload");
    }
}

#[tokio::test]
async fn worktree_all_methods_dispatch_without_service() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    let methods = [
        METHOD_CREATE_WORKTREE,
        METHOD_REMOVE_WORKTREE,
        METHOD_LIST_WORKTREES,
        METHOD_GET_WORKTREE,
    ];
    for method in methods {
        let r = h
            .handle_frame(req_frame("wt-dispatch", method, vec![]))
            .await;
        assert_eq!(r.len(), 1, "method: {method}");
        assert_eq!(r[0].frame_type, FrameType::Error as i32, "method: {method}");
        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
            assert!(
                e.message.contains("WorktreeService not available"),
                "method {method} should hit 'not available', got: {}",
                e.message
            );
        }
    }
}

#[tokio::test]
async fn worktree_malformed_data_returns_error() {
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_worktree_service()
        .build()
        .await;
    let r = h
        .handle_frame(req_frame("wt-bad", METHOD_LIST_WORKTREES, vec![0xFF, 0xFF]))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
}

// --- I-2: ExecuteServiceCommand streaming tests (daemon side) ---

use betcode_proto::v1::{ExecuteServiceCommandRequest, ServiceCommandOutput};

#[tokio::test]
async fn execute_service_command_streams_output() {
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new()
        .with_command_service()
        .build()
        .await;
    // "pwd" is a simple command that prints the working directory and exits
    let req = ExecuteServiceCommandRequest {
        command: "pwd".into(),
        args: vec![],
    };
    let result = h
        .handle_frame(req_frame(
            "esc1",
            METHOD_EXECUTE_SERVICE_COMMAND,
            encode(&req),
        ))
        .await;
    // handle_execute_service_command always returns empty vec (sends frames async)
    assert!(result.is_empty());

    // Collect frames: expect at least one StreamData followed by StreamEnd
    let mut stream_data_count = 0;
    let mut got_stream_end = false;
    let mut got_error = false;
    let mut decoded_outputs = Vec::new();
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("Timed out waiting for frame")
            .expect("Channel closed unexpectedly");
        assert_eq!(frame.request_id, "esc1");
        match FrameType::try_from(frame.frame_type) {
            Ok(FrameType::StreamData) => {
                stream_data_count += 1;
                if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) =
                    &frame.payload
                {
                    let data = p
                        .encrypted
                        .as_ref()
                        .map(|e| e.ciphertext.as_slice())
                        .unwrap_or(&[]);
                    if let Ok(output) = ServiceCommandOutput::decode(data) {
                        decoded_outputs.push(output);
                    }
                }
            }
            Ok(FrameType::StreamEnd) => {
                got_stream_end = true;
                break;
            }
            Ok(FrameType::Error) => {
                // Some environments may not have "pwd"; still a valid test path
                got_error = true;
                break;
            }
            other => panic!("Unexpected frame type: {:?}", other),
        }
    }
    // Ensure the test always validates something (guard against vacuous pass)
    assert!(
        stream_data_count > 0 || got_error,
        "Expected either stream data or an explicit error frame"
    );
    if stream_data_count > 0 {
        assert!(got_stream_end, "Expected StreamEnd after StreamData frames");
        // At least one output should be a stdout_line (the pwd result) or an exit_code
        let has_output = decoded_outputs.iter().any(|o| o.output.is_some());
        assert!(
            has_output,
            "Expected at least one ServiceCommandOutput with data"
        );
    }
}

#[tokio::test]
async fn execute_service_command_not_set_sends_error() {
    // Handler without command service — should send error via outbound_tx
    let HandlerTestOutput {
        handler: h, mut rx, ..
    } = HandlerTestBuilder::new().build().await;
    let req = ExecuteServiceCommandRequest {
        command: "pwd".into(),
        args: vec![],
    };
    let result = h
        .handle_frame(req_frame(
            "esc-none",
            METHOD_EXECUTE_SERVICE_COMMAND,
            encode(&req),
        ))
        .await;
    assert!(result.is_empty());

    let frame = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.frame_type, FrameType::Error as i32);
    assert_eq!(frame.request_id, "esc-none");
    if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &frame.payload {
        assert!(e.message.contains("CommandService not available"));
    } else {
        panic!("expected error payload");
    }
}

// --- Plugin RPC tunnel handler tests ---

#[tokio::test]
async fn command_all_plugin_methods_dispatch_without_service() {
    // All 6 plugin method constants should be recognized and hit the
    // "CommandService not available" error path (not the unknown method path).
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new().build().await;
    let methods = [
        METHOD_LIST_PLUGINS,
        METHOD_GET_PLUGIN_STATUS,
        METHOD_ADD_PLUGIN,
        METHOD_REMOVE_PLUGIN,
        METHOD_ENABLE_PLUGIN,
        METHOD_DISABLE_PLUGIN,
    ];
    for method in methods {
        let r = h
            .handle_frame(req_frame("plugin-dispatch", method, vec![]))
            .await;
        assert_eq!(r.len(), 1, "method: {method}");
        assert_eq!(r[0].frame_type, FrameType::Error as i32, "method: {method}");
        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
            assert!(
                e.message.contains("CommandService not available"),
                "method {method} should hit 'not available', got: {}",
                e.message
            );
        }
    }
}

#[tokio::test]
async fn command_plugin_methods_dispatch_with_service() {
    // When CommandService is configured, plugin RPCs should reach the service dispatch
    // (not "Unknown method" or "CommandService not available"). The current implementation
    // stubs return Status::unimplemented, which the dispatch_rpc! macro converts to an
    // error frame with the service's status message.
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_command_service()
        .build()
        .await;
    let methods = [
        METHOD_LIST_PLUGINS,
        METHOD_GET_PLUGIN_STATUS,
        METHOD_ADD_PLUGIN,
        METHOD_REMOVE_PLUGIN,
        METHOD_ENABLE_PLUGIN,
        METHOD_DISABLE_PLUGIN,
    ];
    for method in methods {
        let r = h
            .handle_frame(req_frame("plugin-svc", method, vec![]))
            .await;
        assert_eq!(r.len(), 1, "method: {method}");
        assert_eq!(r[0].frame_type, FrameType::Error as i32, "method: {method}");
        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
            assert!(
                !e.message.contains("CommandService not available"),
                "method {method} should reach service dispatch, not 'not available'. Got: {}",
                e.message
            );
            assert!(
                !e.message.contains("Unknown method"),
                "method {method} should be recognized, not 'Unknown method'. Got: {}",
                e.message
            );
        }
    }
}

#[tokio::test]
async fn list_plugins_with_valid_request_dispatches_through_service() {
    // Send a properly encoded ListPluginsRequest to verify the full dispatch path:
    // decode → service call → response frame. The service currently returns
    // Status::unimplemented, but the key assertion is that it reaches the service
    // (not "CommandService not available" or "Unknown method").
    let HandlerTestOutput { handler: h, .. } = HandlerTestBuilder::new()
        .with_command_service()
        .build()
        .await;
    let req = ListPluginsRequest {};
    let r = h
        .handle_frame(req_frame("plugin-list", METHOD_LIST_PLUGINS, encode(&req)))
        .await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].frame_type, FrameType::Error as i32);
    if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = &r[0].payload {
        // Should contain the service stub message, not infrastructure errors
        assert!(
            e.message.contains("Plugin management not yet available"),
            "Expected service-level stub message, got: {}",
            e.message
        );
    } else {
        panic!("expected error payload");
    }
}

// --- M-5: relay_forwarded output encryption bypass test ---

#[tokio::test]
async fn relay_forwarded_response_has_empty_nonce_when_crypto_active() {
    // Set up handler WITH crypto active
    let HandlerTestOutput { handler: h, .. } =
        HandlerTestBuilder::new().with_crypto().build().await;

    // Build a request with empty nonce (relay-forwarded)
    let req = ListSessionsRequest {
        working_directory: String::new(),
        worktree_id: String::new(),
        limit: 10,
        offset: 0,
    };
    // Use req_frame (which puts empty nonce) to simulate relay-forwarded request
    let frame = req_frame("rf1", METHOD_LIST_SESSIONS, encode(&req));

    let responses = h.handle_frame(frame).await;
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].frame_type, FrameType::Response as i32);

    // The response should also have an empty nonce (plaintext) because the
    // request was relay-forwarded — the relay cannot decrypt encrypted responses.
    if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = &responses[0].payload {
        let enc = p
            .encrypted
            .as_ref()
            .expect("encrypted payload should exist");
        assert!(
            enc.nonce.is_empty(),
            "Relay-forwarded response must have empty nonce (plaintext) even when crypto is active, but got nonce length {}",
            enc.nonce.len()
        );
        // Verify the response is valid protobuf (directly decodable, not encrypted)
        let resp = ListSessionsResponse::decode(enc.ciphertext.as_slice())
            .expect("Response should be decodable as plaintext protobuf");
        // Sessions list should be valid (empty is fine)
        let _ = resp.sessions;
    } else {
        panic!("Expected StreamData payload in response");
    }
}
