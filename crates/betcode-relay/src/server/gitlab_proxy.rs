//! `GitLabService` proxy that forwards calls through the tunnel to daemons.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::git_lab_service_server::GitLabService;
use betcode_proto::v1::{
    GetIssueRequest, GetIssueResponse, GetMergeRequestRequest, GetMergeRequestResponse,
    GetPipelineRequest, GetPipelineResponse, ListIssuesRequest, ListIssuesResponse,
    ListMergeRequestsRequest, ListMergeRequestsResponse, ListPipelinesRequest,
    ListPipelinesResponse,
};

use betcode_proto::methods::{
    METHOD_GET_ISSUE, METHOD_GET_MERGE_REQUEST, METHOD_GET_PIPELINE, METHOD_LIST_ISSUES,
    METHOD_LIST_MERGE_REQUESTS, METHOD_LIST_PIPELINES,
};

use crate::router::RequestRouter;

/// Proxies `GitLabService` calls through the tunnel to a target daemon.
pub struct GitLabProxyService {
    router: Arc<RequestRouter>,
}

impl GitLabProxyService {
    pub const fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
    }
}

#[tonic::async_trait]
impl GitLabService for GitLabProxyService {
    #[instrument(skip(self, request), fields(rpc = "ListMergeRequests"))]
    async fn list_merge_requests(
        &self,
        request: Request<ListMergeRequestsRequest>,
    ) -> Result<Response<ListMergeRequestsResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_LIST_MERGE_REQUESTS).await
    }

    #[instrument(skip(self, request), fields(rpc = "GetMergeRequest"))]
    async fn get_merge_request(
        &self,
        request: Request<GetMergeRequestRequest>,
    ) -> Result<Response<GetMergeRequestResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_GET_MERGE_REQUEST).await
    }

    #[instrument(skip(self, request), fields(rpc = "ListPipelines"))]
    async fn list_pipelines(
        &self,
        request: Request<ListPipelinesRequest>,
    ) -> Result<Response<ListPipelinesResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_LIST_PIPELINES).await
    }

    #[instrument(skip(self, request), fields(rpc = "GetPipeline"))]
    async fn get_pipeline(
        &self,
        request: Request<GetPipelineRequest>,
    ) -> Result<Response<GetPipelineResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_GET_PIPELINE).await
    }

    #[instrument(skip(self, request), fields(rpc = "ListIssues"))]
    async fn list_issues(
        &self,
        request: Request<ListIssuesRequest>,
    ) -> Result<Response<ListIssuesResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_LIST_ISSUES).await
    }

    #[instrument(skip(self, request), fields(rpc = "GetIssue"))]
    async fn get_issue(
        &self,
        request: Request<GetIssueRequest>,
    ) -> Result<Response<GetIssueResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_GET_ISSUE).await
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#[path = "gitlab_proxy_tests.rs"]
mod tests;
