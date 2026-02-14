//! Tests for `ConfigProxyService`.

use betcode_proto::v1::config_service_server::ConfigService;
use betcode_proto::v1::{
    GetPermissionsRequest, GetSettingsRequest, ListMcpServersRequest, ListMcpServersResponse,
    PermissionRules, Settings, UpdateSettingsRequest,
};

use super::ConfigProxyService;
use crate::server::test_helpers::{
    assert_daemon_error, assert_no_claims_error, assert_no_machine_error, assert_offline_error,
    make_request, proxy_test_setup, spawn_responder,
};

proxy_test_setup!(ConfigProxyService);

// --- Unary RPC routing ---

#[tokio::test]
async fn get_settings_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(&router, "m1", rx, Settings::default());
    let req = make_request(
        GetSettingsRequest {
            scope: "global".into(),
        },
        "m1",
    );
    let resp = svc.get_settings(req).await.unwrap().into_inner();
    // Settings::default() has all None/empty fields -- just assert it parses
    assert!(resp.daemon.is_none());
}

#[tokio::test]
async fn update_settings_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(&router, "m1", rx, Settings::default());
    let req = make_request(
        UpdateSettingsRequest {
            settings: Some(Settings::default()),
            scope: "global".into(),
        },
        "m1",
    );
    let _resp = svc.update_settings(req).await.unwrap().into_inner();
}

#[tokio::test]
async fn list_mcp_servers_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListMcpServersResponse { servers: vec![] },
    );
    let req = make_request(
        ListMcpServersRequest {
            status_filter: String::new(),
        },
        "m1",
    );
    let resp = svc.list_mcp_servers(req).await.unwrap().into_inner();
    assert!(resp.servers.is_empty());
}

#[tokio::test]
async fn get_permissions_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        PermissionRules {
            rules: vec![],
            denied_tools: vec![],
            require_approval: vec![],
        },
    );
    let req = make_request(
        GetPermissionsRequest {
            session_id: "s1".into(),
        },
        "m1",
    );
    let resp = svc.get_permissions(req).await.unwrap().into_inner();
    assert!(resp.rules.is_empty());
}

// --- Error handling ---

#[tokio::test]
async fn machine_offline_returns_unavailable() {
    let svc = setup_offline().await;
    assert_offline_error!(
        svc,
        get_settings,
        GetSettingsRequest {
            scope: String::new(),
        }
    );
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_machine_error!(
        svc,
        get_settings,
        GetSettingsRequest {
            scope: String::new(),
        }
    );
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_claims_error!(
        svc,
        get_settings,
        GetSettingsRequest {
            scope: String::new(),
        }
    );
}

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    assert_daemon_error!(
        svc,
        get_settings,
        GetSettingsRequest {
            scope: String::new(),
        },
        router,
        rx,
        "daemon error"
    );
}
