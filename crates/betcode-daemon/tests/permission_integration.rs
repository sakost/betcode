#![allow(clippy::unwrap_used)] // Integration tests use unwrap for brevity

//! Integration tests for the permission bridge.
//!
//! Tests the full flow: rule evaluation → pending requests → client response → grant caching.

use std::time::Duration;

use betcode_core::permissions::{PermissionAction, PermissionEngine, PermissionRule, RuleSource};
use betcode_daemon::permission::{
    DaemonPermissionEngine, PendingConfig, PermissionEvalRequest, PermissionEvaluation,
    PermissionResponse,
};
use betcode_daemon::storage::Database;

/// Helper to create an engine with in-memory DB.
async fn engine_with_db() -> DaemonPermissionEngine {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("session-1", "claude-sonnet-4", "/tmp")
        .await
        .unwrap();
    DaemonPermissionEngine::with_database(PermissionEngine::new(), PendingConfig::default(), db)
}

/// Build a `PermissionEvalRequest` with common defaults.
/// Only `request_id` and `tool_name` vary in most tests.
const fn eval_request<'a>(
    request_id: &'a str,
    tool_name: &'a str,
    description: &'a str,
) -> PermissionEvalRequest<'a> {
    PermissionEvalRequest {
        session_id: "session-1",
        request_id,
        tool_name,
        description,
        input_json: "{}",
        path: None,
        target_client: None,
        client_connected: true,
    }
}

#[tokio::test]
async fn full_permission_flow_allow() {
    let engine = engine_with_db().await;

    // 1. Bash requires Ask by default -> creates pending request
    let eval = engine
        .evaluate(&PermissionEvalRequest {
            session_id: "session-1",
            request_id: "req-1",
            tool_name: "Bash",
            description: "ls command",
            input_json: r#"{"command":"ls"}"#,
            path: None,
            target_client: Some("client-1"),
            client_connected: true,
        })
        .await;

    assert!(matches!(eval, PermissionEvaluation::Pending { .. }));

    // 2. Client grants with remember_session
    let response = PermissionResponse {
        request_id: "req-1".to_string(),
        granted: true,
        remember_session: true,
        remember_permanent: false,
    };

    let processed = engine.process_response(response).await.unwrap();
    assert!(processed.granted);
    assert_eq!(processed.request.tool_name, "Bash");

    // 3. Next Bash request should hit session cache
    let eval = engine
        .evaluate(&PermissionEvalRequest {
            session_id: "session-1",
            request_id: "req-2",
            tool_name: "Bash",
            description: "pwd command",
            input_json: r#"{"command":"pwd"}"#,
            path: None,
            target_client: Some("client-1"),
            client_connected: true,
        })
        .await;

    assert!(matches!(
        eval,
        PermissionEvaluation::Allowed { cached: true }
    ));
}

#[tokio::test]
async fn full_permission_flow_deny() {
    let engine = engine_with_db().await;

    // 1. Write requires Ask
    let eval = engine
        .evaluate(&PermissionEvalRequest {
            session_id: "session-1",
            request_id: "req-1",
            tool_name: "Write",
            description: "Write file",
            input_json: "{}",
            path: None,
            target_client: Some("client-1"),
            client_connected: true,
        })
        .await;

    assert!(matches!(eval, PermissionEvaluation::Pending { .. }));

    // 2. Client denies with remember_session
    let response = PermissionResponse {
        request_id: "req-1".to_string(),
        granted: false,
        remember_session: true,
        remember_permanent: false,
    };

    let processed = engine.process_response(response).await.unwrap();
    assert!(!processed.granted);

    // 3. Next Write request should hit session cache as denied
    let eval = engine
        .evaluate(&PermissionEvalRequest {
            session_id: "session-1",
            request_id: "req-2",
            tool_name: "Write",
            description: "Write another",
            input_json: "{}",
            path: None,
            target_client: Some("client-1"),
            client_connected: true,
        })
        .await;

    assert!(matches!(
        eval,
        PermissionEvaluation::Denied { cached: true }
    ));
}

#[tokio::test]
async fn persistent_grant_via_database() {
    let db = Database::open_in_memory().await.unwrap();
    db.create_session("session-1", "claude-sonnet-4", "/tmp")
        .await
        .unwrap();

    // Create first engine, grant permanently
    {
        let engine = DaemonPermissionEngine::with_database(
            PermissionEngine::new(),
            PendingConfig::default(),
            db.clone(),
        );

        let eval = engine
            .evaluate(&PermissionEvalRequest {
                session_id: "session-1",
                request_id: "req-1",
                tool_name: "Edit",
                description: "Edit file",
                input_json: "{}",
                path: None,
                target_client: Some("client-1"),
                client_connected: true,
            })
            .await;
        assert!(matches!(eval, PermissionEvaluation::Pending { .. }));

        let response = PermissionResponse {
            request_id: "req-1".to_string(),
            granted: true,
            remember_session: false,
            remember_permanent: true,
        };
        engine.process_response(response).await.unwrap();
    }

    // Create second engine (simulates daemon restart) - DB grant persists
    {
        let engine = DaemonPermissionEngine::with_database(
            PermissionEngine::new(),
            PendingConfig::default(),
            db.clone(),
        );

        let eval = engine
            .evaluate(&PermissionEvalRequest {
                session_id: "session-1",
                request_id: "req-2",
                tool_name: "Edit",
                description: "Edit another",
                input_json: "{}",
                path: None,
                target_client: Some("client-1"),
                client_connected: true,
            })
            .await;

        // Should be allowed from database cache
        assert!(matches!(
            eval,
            PermissionEvaluation::Allowed { cached: true }
        ));
    }
}

#[tokio::test]
async fn pending_request_timeout() {
    let config = PendingConfig {
        connected_timeout: Duration::from_millis(5),
        disconnected_timeout: Duration::from_millis(50),
        cleanup_interval: Duration::from_millis(10),
    };

    let engine = DaemonPermissionEngine::new(PermissionEngine::new(), config);

    // Create pending request
    let eval = engine
        .evaluate(&PermissionEvalRequest {
            session_id: "session-1",
            request_id: "req-1",
            tool_name: "Bash",
            description: "command",
            input_json: "{}",
            path: None,
            target_client: Some("client-1"),
            client_connected: true,
        })
        .await;
    assert!(matches!(eval, PermissionEvaluation::Pending { .. }));

    // Wait for connected timeout
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Cleanup should remove it
    let expired = engine.cleanup_expired().await;
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0], "req-1");

    // Responding to expired request should fail
    let response = PermissionResponse {
        request_id: "req-1".to_string(),
        granted: true,
        remember_session: false,
        remember_permanent: false,
    };
    let result = engine.process_response(response).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn disconnected_client_preserves_pending() {
    let config = PendingConfig {
        connected_timeout: Duration::from_millis(5),
        disconnected_timeout: Duration::from_secs(60),
        cleanup_interval: Duration::from_millis(10),
    };

    let engine = DaemonPermissionEngine::new(PermissionEngine::new(), config);

    // Create pending request for disconnected client
    let eval = engine
        .evaluate(&PermissionEvalRequest {
            session_id: "session-1",
            request_id: "req-1",
            tool_name: "Bash",
            description: "command",
            input_json: "{}",
            path: None,
            target_client: Some("client-1"),
            client_connected: false,
        })
        .await;
    assert!(matches!(eval, PermissionEvaluation::Pending { .. }));

    // Wait past connected timeout
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Cleanup should NOT remove it (disconnected has longer TTL)
    let expired = engine.cleanup_expired().await;
    assert_eq!(expired.len(), 0);

    // Client reconnects and responds
    engine.update_client_status("client-1", true).await;

    let response = PermissionResponse {
        request_id: "req-1".to_string(),
        granted: true,
        remember_session: true,
        remember_permanent: false,
    };
    let result = engine.process_response(response).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn session_grant_cleared_on_reset() {
    let engine = engine_with_db().await;

    // Grant Bash for session
    engine
        .add_session_grant("session-1", "Bash", None, true)
        .await;

    // Should be cached
    let eval = engine
        .evaluate(&eval_request("req-1", "Bash", "command"))
        .await;
    assert!(matches!(
        eval,
        PermissionEvaluation::Allowed { cached: true }
    ));

    // Clear session grants
    engine.clear_session_grants("session-1").await;

    // Should go back to Ask (pending)
    let eval = engine
        .evaluate(&eval_request("req-2", "Bash", "command"))
        .await;
    assert!(matches!(eval, PermissionEvaluation::Pending { .. }));
}

#[tokio::test]
async fn custom_rules_override_defaults() {
    let rules = vec![PermissionRule {
        id: "allow-all-bash".to_string(),
        tool_pattern: "Bash".to_string(),
        path_pattern: None,
        action: PermissionAction::Allow,
        priority: 10, // Higher priority than default Ask (200)
        description: Some("Allow all bash".to_string()),
        source: RuleSource::Project,
    }];

    let mut rule_engine = PermissionEngine::new();
    rule_engine.add_rules(rules);

    let engine = DaemonPermissionEngine::new(rule_engine, PendingConfig::default());

    // Bash should be allowed by custom rule
    let eval = engine
        .evaluate(&eval_request("req-1", "Bash", "command"))
        .await;

    assert!(matches!(
        eval,
        PermissionEvaluation::Allowed { cached: false }
    ));
}

#[tokio::test]
async fn multiple_sessions_independent_grants() {
    let engine = engine_with_db().await;

    // Grant Bash for session-1
    engine
        .add_session_grant("session-1", "Bash", None, true)
        .await;

    // session-1: should be cached
    let eval = engine
        .evaluate(&eval_request("req-1", "Bash", "command"))
        .await;
    assert!(matches!(
        eval,
        PermissionEvaluation::Allowed { cached: true }
    ));

    // session-2: should still be Ask (pending) - no grant for this session
    let eval = engine
        .evaluate(&PermissionEvalRequest {
            session_id: "session-2",
            ..eval_request("req-2", "Bash", "command")
        })
        .await;
    assert!(matches!(eval, PermissionEvaluation::Pending { .. }));
}
