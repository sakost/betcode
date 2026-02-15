//! Tests for `GitRepoProxyService`.

use betcode_proto::v1::git_repo_service_server::GitRepoService;
use betcode_proto::v1::{
    GetRepoRequest, GitRepoDetail, ListReposRequest, ListReposResponse, RegisterRepoRequest,
    ScanReposRequest, UnregisterRepoRequest, UnregisterRepoResponse, UpdateRepoRequest,
    WorktreeMode,
};

use super::GitRepoProxyService;
use crate::server::test_helpers::{
    assert_daemon_error, assert_no_claims_error, assert_no_machine_error, assert_offline_error,
    assert_wrong_owner_error, make_request, proxy_test_setup, spawn_responder,
};

proxy_test_setup!(GitRepoProxyService);

// --- Unary RPC routing ---

#[tokio::test]
async fn register_repo_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let detail = GitRepoDetail {
        id: "repo-1".into(),
        name: "my-repo".into(),
        repo_path: "/home/user/projects/my-repo".into(),
        worktree_mode: WorktreeMode::Global as i32,
        ..Default::default()
    };
    spawn_responder(&router, "m1", rx, detail);
    let req = make_request(
        RegisterRepoRequest {
            repo_path: "/home/user/projects/my-repo".into(),
            name: "my-repo".into(),
            worktree_mode: WorktreeMode::Global as i32,
            ..Default::default()
        },
        "m1",
    );
    let resp = svc.register_repo(req).await.unwrap().into_inner();
    assert_eq!(resp.id, "repo-1");
    assert_eq!(resp.name, "my-repo");
}

#[tokio::test]
async fn unregister_repo_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        UnregisterRepoResponse {
            removed: true,
            worktrees_removed: 3,
        },
    );
    let req = make_request(
        UnregisterRepoRequest {
            id: "repo-1".into(),
            remove_worktrees: true,
        },
        "m1",
    );
    let resp = svc.unregister_repo(req).await.unwrap().into_inner();
    assert!(resp.removed);
    assert_eq!(resp.worktrees_removed, 3);
}

#[tokio::test]
async fn list_repos_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListReposResponse {
            repos: vec![GitRepoDetail {
                id: "repo-1".into(),
                name: "my-repo".into(),
                ..Default::default()
            }],
            total_count: 1,
        },
    );
    let req = make_request(
        ListReposRequest {
            limit: 0,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_repos(req).await.unwrap().into_inner();
    assert_eq!(resp.repos.len(), 1);
    assert_eq!(resp.repos[0].id, "repo-1");
}

#[tokio::test]
async fn get_repo_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let detail = GitRepoDetail {
        id: "repo-2".into(),
        name: "other-repo".into(),
        worktree_mode: WorktreeMode::Local as i32,
        ..Default::default()
    };
    spawn_responder(&router, "m1", rx, detail);
    let req = make_request(
        GetRepoRequest {
            id: "repo-2".into(),
        },
        "m1",
    );
    let resp = svc.get_repo(req).await.unwrap().into_inner();
    assert_eq!(resp.id, "repo-2");
    assert_eq!(resp.worktree_mode, WorktreeMode::Local as i32);
}

#[tokio::test]
async fn update_repo_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let detail = GitRepoDetail {
        id: "repo-1".into(),
        name: "renamed".into(),
        worktree_mode: WorktreeMode::Custom as i32,
        ..Default::default()
    };
    spawn_responder(&router, "m1", rx, detail);
    let req = make_request(
        UpdateRepoRequest {
            id: "repo-1".into(),
            name: Some("renamed".into()),
            worktree_mode: Some(WorktreeMode::Custom as i32),
            ..Default::default()
        },
        "m1",
    );
    let resp = svc.update_repo(req).await.unwrap().into_inner();
    assert_eq!(resp.id, "repo-1");
    assert_eq!(resp.name, "renamed");
}

#[tokio::test]
async fn scan_repos_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListReposResponse {
            repos: vec![
                GitRepoDetail {
                    id: "scan-1".into(),
                    name: "found-repo".into(),
                    ..Default::default()
                },
                GitRepoDetail {
                    id: "scan-2".into(),
                    name: "another-repo".into(),
                    ..Default::default()
                },
            ],
            total_count: 2,
        },
    );
    let req = make_request(
        ScanReposRequest {
            scan_path: "/home/user/projects".into(),
            max_depth: 3,
        },
        "m1",
    );
    let resp = svc.scan_repos(req).await.unwrap().into_inner();
    assert_eq!(resp.repos.len(), 2);
    assert_eq!(resp.repos[0].name, "found-repo");
}

// --- Error handling ---

#[tokio::test]
async fn machine_offline_returns_unavailable() {
    let svc = setup_offline().await;
    assert_offline_error!(
        svc,
        list_repos,
        ListReposRequest {
            limit: 0,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_machine_error!(
        svc,
        list_repos,
        ListReposRequest {
            limit: 0,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_claims_error!(
        svc,
        list_repos,
        ListReposRequest {
            limit: 0,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn wrong_owner_returns_permission_denied() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_wrong_owner_error!(
        svc,
        list_repos,
        ListReposRequest {
            limit: 0,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    assert_daemon_error!(
        svc,
        list_repos,
        ListReposRequest {
            limit: 0,
            offset: 0,
        },
        router,
        rx,
        "repo registration failed"
    );
}
