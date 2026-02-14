//! Tests for `WorktreeProxyService`.

use betcode_proto::v1::worktree_service_server::WorktreeService;
use betcode_proto::v1::{
    CreateWorktreeRequest, GetWorktreeRequest, ListWorktreesRequest, ListWorktreesResponse,
    RemoveWorktreeRequest, RemoveWorktreeResponse, WorktreeDetail,
};

use super::WorktreeProxyService;
use crate::server::test_helpers::{
    assert_daemon_error, assert_no_claims_error, assert_no_machine_error, assert_offline_error,
    make_request, proxy_test_setup, spawn_responder,
};

proxy_test_setup!(WorktreeProxyService);

// --- Unary RPC routing ---

#[tokio::test]
async fn list_worktrees_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListWorktreesResponse {
            worktrees: vec![WorktreeDetail {
                id: "wt-10".into(),
                name: "my-feature".into(),
                path: "/tmp/wt-10".into(),
                branch: "feat/my-feature".into(),
                ..Default::default()
            }],
        },
    );
    let req = make_request(
        ListWorktreesRequest {
            repo_id: "/repo".into(),
        },
        "m1",
    );
    let resp = svc.list_worktrees(req).await.unwrap().into_inner();
    assert_eq!(resp.worktrees.len(), 1);
    assert_eq!(resp.worktrees[0].id, "wt-10");
    assert_eq!(resp.worktrees[0].name, "my-feature");
    assert_eq!(resp.worktrees[0].branch, "feat/my-feature");
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
            repo_id: "/repo".into(),
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
    assert_offline_error!(
        svc,
        list_worktrees,
        ListWorktreesRequest {
            repo_id: String::new(),
        }
    );
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_machine_error!(
        svc,
        list_worktrees,
        ListWorktreesRequest {
            repo_id: String::new(),
        }
    );
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_claims_error!(
        svc,
        list_worktrees,
        ListWorktreesRequest {
            repo_id: String::new(),
        }
    );
}

// --- M-8: daemon_error_propagated_to_client ---

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    assert_daemon_error!(
        svc,
        list_worktrees,
        ListWorktreesRequest {
            repo_id: String::new(),
        },
        router,
        rx,
        "daemon error"
    );
}
