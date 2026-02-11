//! Tests for WorktreeProxyService.

use std::collections::HashMap;
use std::sync::Arc;

use tonic::Request;

use betcode_proto::v1::worktree_service_server::WorktreeService;
use betcode_proto::v1::{
    CreateWorktreeRequest, FrameType, GetWorktreeRequest, ListWorktreesRequest,
    ListWorktreesResponse, RemoveWorktreeRequest, RemoveWorktreeResponse, TunnelFrame,
    WorktreeDetail,
};

use super::WorktreeProxyService;
use crate::server::test_helpers::{
    make_request, setup_offline_router, setup_router_with_machine, spawn_responder, test_claims,
};

async fn setup_with_machine(
    mid: &str,
) -> (
    WorktreeProxyService,
    Arc<crate::router::RequestRouter>,
    tokio::sync::mpsc::Receiver<TunnelFrame>,
) {
    let (router, rx) = setup_router_with_machine(mid).await;
    (WorktreeProxyService::new(Arc::clone(&router)), router, rx)
}

async fn setup_offline() -> WorktreeProxyService {
    let router = setup_offline_router().await;
    WorktreeProxyService::new(router)
}

// --- Unary RPC routing ---

#[tokio::test]
async fn list_worktrees_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListWorktreesResponse { worktrees: vec![] },
    );
    let req = make_request(
        ListWorktreesRequest {
            repo_path: String::new(),
        },
        "m1",
    );
    let resp = svc.list_worktrees(req).await.unwrap().into_inner();
    assert!(resp.worktrees.is_empty());
}

#[tokio::test]
async fn create_worktree_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let detail = WorktreeDetail {
        id: "wt-1".into(),
        name: "feature".into(),
        path: "/tmp/wt".into(),
        branch: "feature-branch".into(),
        ..Default::default()
    };
    spawn_responder(&router, "m1", rx, detail);
    let req = make_request(
        CreateWorktreeRequest {
            name: "feature".into(),
            repo_path: "/repo".into(),
            branch: "feature-branch".into(),
            setup_script: String::new(),
        },
        "m1",
    );
    let resp = svc.create_worktree(req).await.unwrap().into_inner();
    assert_eq!(resp.id, "wt-1");
    assert_eq!(resp.name, "feature");
}

#[tokio::test]
async fn get_worktree_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let detail = WorktreeDetail {
        id: "wt-2".into(),
        name: "bugfix".into(),
        exists_on_disk: true,
        ..Default::default()
    };
    spawn_responder(&router, "m1", rx, detail);
    let req = make_request(GetWorktreeRequest { id: "wt-2".into() }, "m1");
    let resp = svc.get_worktree(req).await.unwrap().into_inner();
    assert_eq!(resp.id, "wt-2");
    assert!(resp.exists_on_disk);
}

#[tokio::test]
async fn remove_worktree_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(&router, "m1", rx, RemoveWorktreeResponse { removed: true });
    let req = make_request(RemoveWorktreeRequest { id: "wt-1".into() }, "m1");
    let resp = svc.remove_worktree(req).await.unwrap().into_inner();
    assert!(resp.removed);
}

// --- Error handling ---

#[tokio::test]
async fn machine_offline_returns_unavailable() {
    let svc = setup_offline().await;
    let req = make_request(
        ListWorktreesRequest {
            repo_path: String::new(),
        },
        "m-off",
    );
    let err = svc.list_worktrees(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(ListWorktreesRequest {
        repo_path: String::new(),
    });
    req.extensions_mut().insert(test_claims());
    let err = svc.list_worktrees(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(ListWorktreesRequest {
        repo_path: String::new(),
    });
    req.metadata_mut()
        .insert("x-machine-id", "m1".parse().unwrap());
    let err = svc.list_worktrees(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
}

// --- M-8: daemon_error_propagated_to_client ---

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
                        message: "daemon error".into(),
                        details: HashMap::new(),
                    },
                )),
            };
            let conn = rc.registry().get("m1").await.unwrap();
            conn.complete_pending(&rid, err_frame).await;
        }
    });
    let req = make_request(
        ListWorktreesRequest {
            repo_path: String::new(),
        },
        "m1",
    );
    let err = svc.list_worktrees(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
    assert!(err.message().contains("daemon error"));
}
