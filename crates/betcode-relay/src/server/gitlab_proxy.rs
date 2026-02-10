//! GitLabService proxy that forwards calls through the tunnel to daemons.

use std::collections::HashMap;
use std::sync::Arc;

use prost::Message;
use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::git_lab_service_server::GitLabService;
use betcode_proto::v1::{
    GetIssueRequest, GetIssueResponse, GetMergeRequestRequest, GetMergeRequestResponse,
    GetPipelineRequest, GetPipelineResponse, ListIssuesRequest, ListIssuesResponse,
    ListMergeRequestsRequest, ListMergeRequestsResponse, ListPipelinesRequest,
    ListPipelinesResponse,
};

use crate::router::RequestRouter;
use crate::server::agent_proxy::{decode_response, extract_machine_id, router_error_to_status};
use crate::server::interceptor::extract_claims;

/// Proxies GitLabService calls through the tunnel to a target daemon.
pub struct GitLabProxyService {
    router: Arc<RequestRouter>,
}

impl GitLabProxyService {
    pub fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
    }

    async fn forward_unary<Req: Message, Resp: Message + Default>(
        &self,
        machine_id: &str,
        method: &str,
        req: &Req,
    ) -> Result<Resp, Status> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut buf = Vec::with_capacity(req.encoded_len());
        req.encode(&mut buf)
            .map_err(|e| Status::internal(format!("Encode error: {}", e)))?;
        let frame = self
            .router
            .forward_request(machine_id, &request_id, method, buf, HashMap::new())
            .await
            .map_err(router_error_to_status)?;
        decode_response(&frame)
    }
}

#[tonic::async_trait]
impl GitLabService for GitLabProxyService {
    #[instrument(skip(self, request), fields(rpc = "ListMergeRequests"))]
    async fn list_merge_requests(
        &self,
        request: Request<ListMergeRequestsRequest>,
    ) -> Result<Response<ListMergeRequestsResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "GitLabService/ListMergeRequests",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "GetMergeRequest"))]
    async fn get_merge_request(
        &self,
        request: Request<GetMergeRequestRequest>,
    ) -> Result<Response<GetMergeRequestResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "GitLabService/GetMergeRequest",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ListPipelines"))]
    async fn list_pipelines(
        &self,
        request: Request<ListPipelinesRequest>,
    ) -> Result<Response<ListPipelinesResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "GitLabService/ListPipelines",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "GetPipeline"))]
    async fn get_pipeline(
        &self,
        request: Request<GetPipelineRequest>,
    ) -> Result<Response<GetPipelineResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "GitLabService/GetPipeline",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ListIssues"))]
    async fn list_issues(
        &self,
        request: Request<ListIssuesRequest>,
    ) -> Result<Response<ListIssuesResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "GitLabService/ListIssues",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "GetIssue"))]
    async fn get_issue(
        &self,
        request: Request<GetIssueRequest>,
    ) -> Result<Response<GetIssueResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(&machine_id, "GitLabService/GetIssue", &request.into_inner())
            .await?;
        Ok(Response::new(resp))
    }
}
