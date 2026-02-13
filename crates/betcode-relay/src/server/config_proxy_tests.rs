//! Tests for `ConfigProxyService`.

use std::sync::Arc;

use tonic::Request;

use betcode_proto::v1::config_service_server::ConfigService;
use betcode_proto::v1::{
    GetPermissionsRequest, GetSettingsRequest, ListMcpServersRequest, ListMcpServersResponse,
    PermissionRules, Settings, UpdateSettingsRequest,
};

use super::ConfigProxyService;
use crate::server::test_helpers::{
    make_request, setup_offline_router, setup_router_with_machine, spawn_error_responder,
    spawn_responder, test_claims,
};

async fn setup_with_machine(
    mid: &str,
) -> (
    ConfigProxyService,
    Arc<crate::router::RequestRouter>,
    tokio::sync::mpsc::Receiver<betcode_proto::v1::TunnelFrame>,
) {
    let (router, rx) = setup_router_with_machine(mid).await;
    (ConfigProxyService::new(Arc::clone(&router)), router, rx)
}

async fn setup_offline() -> ConfigProxyService {
    let router = setup_offline_router().await;
    ConfigProxyService::new(router)
}

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
    let req = make_request(
        GetSettingsRequest {
            scope: String::new(),
        },
        "m-off",
    );
    let err = svc.get_settings(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(GetSettingsRequest {
        scope: String::new(),
    });
    req.extensions_mut().insert(test_claims());
    let err = svc.get_settings(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(GetSettingsRequest {
        scope: String::new(),
    });
    req.metadata_mut()
        .insert("x-machine-id", "m1".parse().unwrap());
    let err = svc.get_settings(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_error_responder(&router, "m1", rx, "daemon error");
    let req = make_request(
        GetSettingsRequest {
            scope: String::new(),
        },
        "m1",
    );
    let err = svc.get_settings(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
    assert!(err.message().contains("daemon error"));
}
