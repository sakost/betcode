#![allow(clippy::unwrap_used, clippy::panic, clippy::cast_possible_wrap)] // Integration tests

//! Integration tests for the relay pipeline and handler wiring.
//!
//! Tests the full flow: handler → relay → multiplexer → DB,
//! without spawning real Claude subprocesses.

use betcode_daemon::session::SessionMultiplexer;
use betcode_daemon::storage::{Database, SessionStatus};
use betcode_daemon::testutil;

/// Build a minimal `AgentEvent` with a `TextDelta` payload (for testing broadcasts).
fn text_delta_event(text: &str, is_complete: bool) -> betcode_proto::v1::AgentEvent {
    betcode_proto::v1::AgentEvent {
        sequence: 0,
        timestamp: None,
        parent_tool_use_id: String::new(),
        event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
            betcode_proto::v1::TextDelta {
                text: text.into(),
                is_complete,
            },
        )),
    }
}

// =========================================================================
// Database session lifecycle tests
// =========================================================================

#[tokio::test]
async fn session_created_in_db() {
    let db = Database::open_in_memory().await.unwrap();
    let session = db
        .create_session("test-1", "claude-sonnet-4", "/tmp/test")
        .await
        .unwrap();

    assert_eq!(session.id, "test-1");
    assert_eq!(session.model, "claude-sonnet-4");
    assert_eq!(session.working_directory, "/tmp/test");
    assert_eq!(session.status, "idle");
    assert!(session.claude_session_id.is_none());
}

#[tokio::test]
async fn session_status_transitions() {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("test-1", "claude-sonnet-4", "/tmp")
        .await
        .unwrap();

    db.update_session_status("test-1", SessionStatus::Active)
        .await
        .unwrap();
    assert_eq!(db.get_session("test-1").await.unwrap().status, "active");

    db.update_session_status("test-1", SessionStatus::Idle)
        .await
        .unwrap();
    assert_eq!(db.get_session("test-1").await.unwrap().status, "idle");
}

#[tokio::test]
async fn claude_session_id_persisted() {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("test-1", "claude-sonnet-4", "/tmp")
        .await
        .unwrap();

    db.update_claude_session_id("test-1", "claude-abc-123")
        .await
        .unwrap();
    let session = db.get_session("test-1").await.unwrap();
    assert_eq!(session.claude_session_id.as_deref(), Some("claude-abc-123"));
}

#[tokio::test]
async fn session_usage_accumulates() {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("test-1", "claude-sonnet-4", "/tmp")
        .await
        .unwrap();

    db.update_session_usage("test-1", 100, 50, 0.01)
        .await
        .unwrap();
    let s = db.get_session("test-1").await.unwrap();
    assert_eq!(s.total_input_tokens, 100);
    assert_eq!(s.total_output_tokens, 50);

    db.update_session_usage("test-1", 200, 100, 0.02)
        .await
        .unwrap();
    let s = db.get_session("test-1").await.unwrap();
    assert_eq!(s.total_input_tokens, 300);
    assert_eq!(s.total_output_tokens, 150);
    assert!((s.total_cost_usd - 0.03).abs() < f64::EPSILON);
}

// =========================================================================
// Message storage with FK and type constraints
// =========================================================================

#[tokio::test]
async fn message_stored_with_valid_types() {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("test-1", "claude-sonnet-4", "/tmp")
        .await
        .unwrap();

    let types = [
        "system",
        "assistant",
        "user",
        "result",
        "stream_event",
        "control_request",
        "control_response",
    ];
    for (i, t) in types.iter().enumerate() {
        db.insert_message("test-1", i as i64 + 1, t, "payload")
            .await
            .unwrap_or_else(|e| panic!("type '{t}' failed: {e}"));
    }

    let msgs = db.get_messages_from_sequence("test-1", 0).await.unwrap();
    assert_eq!(msgs.len(), types.len());
}

#[tokio::test]
async fn message_fk_requires_session() {
    let db = Database::open_in_memory().await.unwrap();
    let result = db
        .insert_message("nonexistent", 1, "system", "payload")
        .await;
    assert!(result.is_err());
}

// =========================================================================
// Relay wiring tests
// =========================================================================

#[tokio::test]
async fn relay_send_requires_active_session() {
    let tc = testutil::test_components().await;
    assert!(tc
        .relay
        .send_user_message("missing", "hello", None)
        .await
        .is_err());
    assert!(tc
        .relay
        .send_permission_response("missing", "r1", true, &serde_json::json!({}))
        .await
        .is_err());
    assert!(tc.relay.send_raw_stdin("missing", "{}").await.is_err());
}

#[tokio::test]
async fn relay_cancel_nonexistent_returns_false() {
    let tc = testutil::test_components().await;
    assert!(!tc.relay.cancel_session("missing").await.unwrap());
}

// =========================================================================
// Event forwarder + multiplexer integration
// =========================================================================

#[tokio::test]
async fn event_forwarder_assigns_sequences() {
    let mux = SessionMultiplexer::with_defaults();
    let handle = mux.subscribe("s1", "c1", "cli").await.unwrap();
    let fwd = mux.create_event_forwarder("s1".to_string());

    for _ in 0..3 {
        fwd.send(text_delta_event("x", false)).await.unwrap();
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let mut rx = handle.event_rx;
    assert_eq!(rx.try_recv().unwrap().sequence, 1);
    assert_eq!(rx.try_recv().unwrap().sequence, 2);
    assert_eq!(rx.try_recv().unwrap().sequence, 3);
}

#[tokio::test]
async fn multiple_clients_receive_broadcast() {
    let mux = SessionMultiplexer::with_defaults();
    let mut h1 = mux.subscribe("s1", "c1", "cli").await.unwrap();
    let mut h2 = mux.subscribe("s1", "c2", "flutter").await.unwrap();

    mux.broadcast("s1", text_delta_event("shared", true)).await;

    let e1 = h1.event_rx.try_recv().unwrap();
    let e2 = h2.event_rx.try_recv().unwrap();
    assert_eq!(e1.sequence, e2.sequence);
    assert_eq!(e1.sequence, 1);
}

// =========================================================================
// Session listing and resume query tests
// =========================================================================

#[tokio::test]
async fn session_list_filters_by_directory() {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("s1", "claude-sonnet-4", "/project/a")
        .await
        .unwrap();
    db.create_session("s2", "claude-sonnet-4", "/project/b")
        .await
        .unwrap();
    db.create_session("s3", "claude-sonnet-4", "/project/a")
        .await
        .unwrap();

    assert_eq!(
        db.list_sessions(Some("/project/a"), 50, 0)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(db.list_sessions(None, 50, 0).await.unwrap().len(), 3);
}

#[tokio::test]
async fn resume_from_sequence() {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("s1", "claude-sonnet-4", "/tmp")
        .await
        .unwrap();
    db.insert_message("s1", 1, "system", "p1").await.unwrap();
    db.insert_message("s1", 2, "stream_event", "p2")
        .await
        .unwrap();
    db.insert_message("s1", 3, "result", "p3").await.unwrap();

    let msgs = db.get_messages_from_sequence("s1", 1).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].sequence, 2);
    assert_eq!(msgs[1].sequence, 3);
}
