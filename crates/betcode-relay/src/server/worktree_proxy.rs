//! WorktreeService proxy that forwards calls through the tunnel to daemons.

use std::collections::HashMap;
use std::sync::Arc;

use prost::Message;
use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::worktree_service_server::WorktreeService;
use betcode_proto::v1::{
    CreateWorktreeRequest, GetWorktreeRequest, ListWorktreesRequest, ListWorktreesResponse,
    RemoveWorktreeRequest, RemoveWorktreeResponse, WorktreeDetail,
};

use crate::router::RequestRouter;
use crate::server::agent_proxy::{decode_response, extract_machine_id, router_error_to_status};
use crate::server::interceptor::extract_claims;

/// Proxies WorktreeService calls through the tunnel to a target daemon.
pub struct WorktreeProxyService {
    router: Arc<RequestRouter>,
}

impl WorktreeProxyService {
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
impl WorktreeService for WorktreeProxyService {
    #[instrument(skip(self, request), fields(rpc = "CreateWorktree"))]
    async fn create_worktree(
        &self,
        request: Request<CreateWorktreeRequest>,
    ) -> Result<Response<WorktreeDetail>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "WorktreeService/CreateWorktree",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "RemoveWorktree"))]
    async fn remove_worktree(
        &self,
        request: Request<RemoveWorktreeRequest>,
    ) -> Result<Response<RemoveWorktreeResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "WorktreeService/RemoveWorktree",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ListWorktrees"))]
    async fn list_worktrees(
        &self,
        request: Request<ListWorktreesRequest>,
    ) -> Result<Response<ListWorktreesResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "WorktreeService/ListWorktrees",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "GetWorktree"))]
    async fn get_worktree(
        &self,
        request: Request<GetWorktreeRequest>,
    ) -> Result<Response<WorktreeDetail>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = self
            .forward_unary(
                &machine_id,
                "WorktreeService/GetWorktree",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }
}
