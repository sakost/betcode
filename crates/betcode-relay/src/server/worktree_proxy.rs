//! WorktreeService proxy that forwards calls through the tunnel to daemons.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::worktree_service_server::WorktreeService;
use betcode_proto::v1::{
    CreateWorktreeRequest, GetWorktreeRequest, ListWorktreesRequest, ListWorktreesResponse,
    RemoveWorktreeRequest, RemoveWorktreeResponse, WorktreeDetail,
};

use betcode_proto::methods::{
    METHOD_CREATE_WORKTREE, METHOD_GET_WORKTREE, METHOD_LIST_WORKTREES, METHOD_REMOVE_WORKTREE,
};

use crate::router::RequestRouter;
use crate::server::agent_proxy::extract_machine_id;
use crate::server::interceptor::extract_claims;

/// Proxies WorktreeService calls through the tunnel to a target daemon.
pub struct WorktreeProxyService {
    router: Arc<RequestRouter>,
}

impl WorktreeProxyService {
    pub fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
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
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_CREATE_WORKTREE,
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
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_REMOVE_WORKTREE,
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
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_LIST_WORKTREES,
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
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_GET_WORKTREE,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }
}

#[cfg(test)]
#[path = "worktree_proxy_tests.rs"]
mod tests;
