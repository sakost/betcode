use super::*;
use crate::subprocess::SubprocessManager;

async fn make_handler() -> TunnelRequestHandler {
    let db = Database::open_in_memory().await.unwrap();
    let sub = Arc::new(SubprocessManager::new(5));
    let mux = Arc::new(SessionMultiplexer::with_defaults());
    let relay = Arc::new(SessionRelay::new(sub, Arc::clone(&mux), db.clone()));
    let (outbound_tx, _outbound_rx) = mpsc::channel(128);
    TunnelRequestHandler::new("test-machine".into(), relay, mux, db, outbound_tx)
}

fn req_frame(rid: &str, method: &str, data: Vec<u8>) -> TunnelFrame {
    TunnelFrame {
        request_id: rid.into(),
        frame_type: FrameType::Request as i32,
        timestamp: None,
        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
            StreamPayload {
                method: method.into(),
                data,
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
        let resp = ListSessionsResponse::decode(p.data.as_slice()).unwrap();
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
        let resp = CancelTurnResponse::decode(p.data.as_slice()).unwrap();
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
        let resp = CompactSessionResponse::decode(p.data.as_slice()).unwrap();
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
        let resp = InputLockResponse::decode(p.data.as_slice()).unwrap();
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
    let handler = TunnelRequestHandler::new("test-machine".into(), relay, mux, db, outbound_tx);
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
    let h = TunnelRequestHandler::new("test-machine".into(), relay, mux, db, outbound_tx);

    let data = make_start_request("sess-fail");
    let result = h
        .handle_frame(req_frame("conv3", METHOD_CONVERSE, data))
        .await;
    assert!(result.is_empty());

    // Error should arrive on outbound (spawn fails immediately with PoolExhausted)
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

/// Simple base64 encoder for tests.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[(n >> 18 & 0x3F) as usize] as char);
        result.push(CHARS[(n >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
