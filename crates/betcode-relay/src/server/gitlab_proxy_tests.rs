//! Tests for `GitLabProxyService`.

use betcode_proto::v1::git_lab_service_server::GitLabService;
use betcode_proto::v1::{
    GetIssueRequest, GetIssueResponse, GetMergeRequestRequest, GetMergeRequestResponse,
    GetPipelineRequest, GetPipelineResponse, IssueInfo, ListIssuesRequest, ListIssuesResponse,
    ListMergeRequestsRequest, ListMergeRequestsResponse, ListPipelinesRequest,
    ListPipelinesResponse, MergeRequestInfo, PipelineInfo,
};

use super::GitLabProxyService;
use crate::server::test_helpers::{
    assert_daemon_error, assert_no_claims_error, assert_no_machine_error, assert_offline_error,
    assert_wrong_owner_error, make_request, proxy_test_setup, spawn_responder,
};

proxy_test_setup!(GitLabProxyService);

// --- Unary RPC routing ---

#[tokio::test]
async fn list_merge_requests_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListMergeRequestsResponse {
            merge_requests: vec![MergeRequestInfo {
                iid: 99,
                title: "fix flaky test".into(),
                source_branch: "fix/flaky".into(),
                ..Default::default()
            }],
            total: 1,
        },
    );
    let req = make_request(
        ListMergeRequestsRequest {
            project: "proj".into(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_merge_requests(req).await.unwrap().into_inner();
    assert_eq!(resp.merge_requests.len(), 1);
    assert_eq!(resp.merge_requests[0].iid, 99);
    assert_eq!(resp.merge_requests[0].title, "fix flaky test");
    assert_eq!(resp.total, 1);
}

#[tokio::test]
async fn get_merge_request_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        GetMergeRequestResponse {
            merge_request: Some(MergeRequestInfo {
                iid: 42,
                title: "test MR".into(),
                ..Default::default()
            }),
        },
    );
    let req = make_request(
        GetMergeRequestRequest {
            project: "proj".into(),
            iid: 42,
        },
        "m1",
    );
    let resp = svc.get_merge_request(req).await.unwrap().into_inner();
    let mr = resp.merge_request.unwrap();
    assert_eq!(mr.iid, 42);
    assert_eq!(mr.title, "test MR");
}

#[tokio::test]
async fn list_issues_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListIssuesResponse {
            issues: vec![IssueInfo {
                iid: 5,
                title: "login page broken".into(),
                author: "alice".into(),
                ..Default::default()
            }],
            total: 1,
        },
    );
    let req = make_request(
        ListIssuesRequest {
            project: "proj".into(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_issues(req).await.unwrap().into_inner();
    assert_eq!(resp.issues.len(), 1);
    assert_eq!(resp.issues[0].iid, 5);
    assert_eq!(resp.issues[0].title, "login page broken");
    assert_eq!(resp.total, 1);
}

#[tokio::test]
async fn get_issue_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(&router, "m1", rx, GetIssueResponse { issue: None });
    let req = make_request(
        GetIssueRequest {
            project: "proj".into(),
            iid: 7,
        },
        "m1",
    );
    let resp = svc.get_issue(req).await.unwrap().into_inner();
    assert!(resp.issue.is_none());
}

// --- Error handling ---

#[tokio::test]
async fn machine_offline_returns_unavailable() {
    let svc = setup_offline().await;
    assert_offline_error!(
        svc,
        list_merge_requests,
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_machine_error!(
        svc,
        list_merge_requests,
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_claims_error!(
        svc,
        list_merge_requests,
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        }
    );
}

#[tokio::test]
async fn wrong_owner_returns_permission_denied() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_wrong_owner_error!(
        svc,
        list_merge_requests,
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        }
    );
}

// --- M-4: Pipeline proxy tests ---

#[tokio::test]
async fn list_pipelines_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListPipelinesResponse {
            pipelines: vec![PipelineInfo {
                id: 101,
                status: 6, // PIPELINE_STATUS_SUCCESS
                ref_name: "main".into(),
                sha: "abc123".into(),
                source: "push".into(),
                web_url: "https://gitlab.com/pipeline/101".into(),
                ..Default::default()
            }],
            total: 1,
        },
    );
    let req = make_request(
        ListPipelinesRequest {
            project: "group/project".into(),
            status_filter: 0,
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_pipelines(req).await.unwrap().into_inner();
    assert_eq!(resp.pipelines.len(), 1);
    assert_eq!(resp.pipelines[0].id, 101);
    assert_eq!(resp.pipelines[0].ref_name, "main");
    assert_eq!(resp.total, 1);
}

#[tokio::test]
async fn get_pipeline_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        GetPipelineResponse {
            pipeline: Some(PipelineInfo {
                id: 202,
                status: 7, // PIPELINE_STATUS_FAILED
                ref_name: "feature".into(),
                sha: "def456".into(),
                source: "web".into(),
                web_url: "https://gitlab.com/pipeline/202".into(),
                ..Default::default()
            }),
        },
    );
    let req = make_request(
        GetPipelineRequest {
            project: "group/project".into(),
            pipeline_id: 202,
        },
        "m1",
    );
    let resp = svc.get_pipeline(req).await.unwrap().into_inner();
    let pipeline = resp.pipeline.unwrap();
    assert_eq!(pipeline.id, 202);
    assert_eq!(pipeline.ref_name, "feature");
    assert_eq!(pipeline.sha, "def456");
}

// --- M-8: daemon_error_propagated_to_client ---

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    assert_daemon_error!(
        svc,
        list_merge_requests,
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        },
        router,
        rx,
        "daemon error"
    );
}
