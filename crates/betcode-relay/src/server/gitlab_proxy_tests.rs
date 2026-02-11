//! Tests for GitLabProxyService.

use std::collections::HashMap;
use std::sync::Arc;

use tonic::Request;

use betcode_proto::v1::git_lab_service_server::GitLabService;
use betcode_proto::v1::{
    FrameType, GetIssueRequest, GetIssueResponse, GetMergeRequestRequest, GetMergeRequestResponse,
    GetPipelineRequest, GetPipelineResponse, ListIssuesRequest, ListIssuesResponse,
    ListMergeRequestsRequest, ListMergeRequestsResponse, ListPipelinesRequest,
    ListPipelinesResponse, MergeRequestInfo, PipelineInfo, TunnelFrame,
};

use super::GitLabProxyService;
use crate::server::test_helpers::{
    make_request, setup_offline_router, setup_router_with_machine, spawn_responder, test_claims,
};

async fn setup_with_machine(
    mid: &str,
) -> (
    GitLabProxyService,
    Arc<crate::router::RequestRouter>,
    tokio::sync::mpsc::Receiver<TunnelFrame>,
) {
    let (router, rx) = setup_router_with_machine(mid).await;
    (GitLabProxyService::new(Arc::clone(&router)), router, rx)
}

async fn setup_offline() -> GitLabProxyService {
    let router = setup_offline_router().await;
    GitLabProxyService::new(router)
}

// --- Unary RPC routing ---

#[tokio::test]
async fn list_merge_requests_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListMergeRequestsResponse {
            merge_requests: vec![],
            total: 0,
        },
    );
    let req = make_request(
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_merge_requests(req).await.unwrap().into_inner();
    assert!(resp.merge_requests.is_empty());
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
            issues: vec![],
            total: 0,
        },
    );
    let req = make_request(
        ListIssuesRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let resp = svc.list_issues(req).await.unwrap().into_inner();
    assert!(resp.issues.is_empty());
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
    let req = make_request(
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        },
        "m-off",
    );
    let err = svc.list_merge_requests(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(ListMergeRequestsRequest {
        project: String::new(),
        state_filter: 0,
        limit: 10,
        offset: 0,
    });
    req.extensions_mut().insert(test_claims());
    let err = svc.list_merge_requests(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(ListMergeRequestsRequest {
        project: String::new(),
        state_filter: 0,
        limit: 10,
        offset: 0,
    });
    req.metadata_mut()
        .insert("x-machine-id", "m1".parse().unwrap());
    let err = svc.list_merge_requests(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
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
        ListMergeRequestsRequest {
            project: String::new(),
            state_filter: 0,
            limit: 10,
            offset: 0,
        },
        "m1",
    );
    let err = svc.list_merge_requests(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
    assert!(err.message().contains("daemon error"));
}
