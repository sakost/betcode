//! GitLabService gRPC implementation.
//!
//! Wraps the reqwest-based GitLabClient to serve gRPC requests.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::info;

use betcode_proto::v1::{
    git_lab_service_server::GitLabService, GetIssueRequest, GetIssueResponse,
    GetMergeRequestRequest, GetMergeRequestResponse, GetPipelineRequest, GetPipelineResponse,
    IssueState, ListIssuesRequest, ListIssuesResponse, ListMergeRequestsRequest,
    ListMergeRequestsResponse, ListPipelinesRequest, ListPipelinesResponse, MergeRequestState,
    PipelineStatus,
};

use super::gitlab_convert::*;
use crate::gitlab::GitLabClient;

/// GitLabService implementation backed by GitLabClient.
pub struct GitLabServiceImpl {
    client: Arc<GitLabClient>,
}

impl GitLabServiceImpl {
    /// Create a new GitLabService.
    pub fn new(client: Arc<GitLabClient>) -> Self {
        Self { client }
    }
}

#[tonic::async_trait]
impl GitLabService for GitLabServiceImpl {
    async fn list_merge_requests(
        &self,
        request: Request<ListMergeRequestsRequest>,
    ) -> Result<Response<ListMergeRequestsResponse>, Status> {
        let req = request.into_inner();
        let state_filter =
            MergeRequestState::try_from(req.state_filter).unwrap_or(MergeRequestState::Unspecified);
        let state_str = mr_state_to_str(state_filter);
        let per_page = if req.limit == 0 { 20 } else { req.limit };
        let page = if req.offset == 0 {
            1
        } else {
            req.offset / per_page + 1
        };

        info!(project = %req.project, state = ?state_str, "Listing merge requests");

        let mrs = self
            .client
            .list_merge_requests(&req.project, state_str, per_page, page)
            .await
            .map_err(to_status)?;

        let merge_requests: Vec<_> = mrs.into_iter().map(to_mr_info).collect();
        let total = merge_requests.len() as u32;
        Ok(Response::new(ListMergeRequestsResponse {
            merge_requests,
            total,
        }))
    }

    async fn get_merge_request(
        &self,
        request: Request<GetMergeRequestRequest>,
    ) -> Result<Response<GetMergeRequestResponse>, Status> {
        let req = request.into_inner();
        info!(project = %req.project, iid = req.iid, "Getting merge request");

        let mr = self
            .client
            .get_merge_request(&req.project, req.iid)
            .await
            .map_err(to_status)?;

        Ok(Response::new(GetMergeRequestResponse {
            merge_request: Some(to_mr_info(mr)),
        }))
    }

    async fn list_pipelines(
        &self,
        request: Request<ListPipelinesRequest>,
    ) -> Result<Response<ListPipelinesResponse>, Status> {
        let req = request.into_inner();
        let status_filter =
            PipelineStatus::try_from(req.status_filter).unwrap_or(PipelineStatus::Unspecified);
        let status_str = pipeline_status_to_str(status_filter);
        let per_page = if req.limit == 0 { 20 } else { req.limit };
        let page = if req.offset == 0 {
            1
        } else {
            req.offset / per_page + 1
        };

        info!(project = %req.project, status = ?status_str, "Listing pipelines");

        let pipelines = self
            .client
            .list_pipelines(&req.project, status_str, per_page, page)
            .await
            .map_err(to_status)?;

        let pipelines: Vec<_> = pipelines.into_iter().map(to_pipeline_info).collect();
        let total = pipelines.len() as u32;
        Ok(Response::new(ListPipelinesResponse { pipelines, total }))
    }

    async fn get_pipeline(
        &self,
        request: Request<GetPipelineRequest>,
    ) -> Result<Response<GetPipelineResponse>, Status> {
        let req = request.into_inner();
        info!(project = %req.project, pipeline_id = req.pipeline_id, "Getting pipeline");

        let pipeline = self
            .client
            .get_pipeline(&req.project, req.pipeline_id)
            .await
            .map_err(to_status)?;

        Ok(Response::new(GetPipelineResponse {
            pipeline: Some(to_pipeline_info(pipeline)),
        }))
    }

    async fn list_issues(
        &self,
        request: Request<ListIssuesRequest>,
    ) -> Result<Response<ListIssuesResponse>, Status> {
        let req = request.into_inner();
        let state_filter =
            IssueState::try_from(req.state_filter).unwrap_or(IssueState::Unspecified);
        let state_str = issue_state_to_str(state_filter);
        let per_page = if req.limit == 0 { 20 } else { req.limit };
        let page = if req.offset == 0 {
            1
        } else {
            req.offset / per_page + 1
        };

        info!(project = %req.project, state = ?state_str, "Listing issues");

        let issues = self
            .client
            .list_issues(&req.project, state_str, per_page, page)
            .await
            .map_err(to_status)?;

        let issues: Vec<_> = issues.into_iter().map(to_issue_info).collect();
        let total = issues.len() as u32;
        Ok(Response::new(ListIssuesResponse { issues, total }))
    }

    async fn get_issue(
        &self,
        request: Request<GetIssueRequest>,
    ) -> Result<Response<GetIssueResponse>, Status> {
        let req = request.into_inner();
        info!(project = %req.project, iid = req.iid, "Getting issue");

        let issue = self
            .client
            .get_issue(&req.project, req.iid)
            .await
            .map_err(to_status)?;

        Ok(Response::new(GetIssueResponse {
            issue: Some(to_issue_info(issue)),
        }))
    }
}
