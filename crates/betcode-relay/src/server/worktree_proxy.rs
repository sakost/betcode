//! `WorktreeService` proxy that forwards calls through the tunnel to daemons.

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
use crate::storage::RelayDatabase;

/// Proxies `WorktreeService` calls through the tunnel to a target daemon.
pub struct WorktreeProxyService {
    router: Arc<RequestRouter>,
    db: RelayDatabase,
}

impl WorktreeProxyService {
    pub const fn new(router: Arc<RequestRouter>, db: RelayDatabase) -> Self {
        Self { router, db }
    }
}

#[tonic::async_trait]
impl WorktreeService for WorktreeProxyService {
    #[instrument(skip(self, request), fields(rpc = "CreateWorktree"))]
    async fn create_worktree(
        &self,
        request: Request<CreateWorktreeRequest>,
    ) -> Result<Response<WorktreeDetail>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_CREATE_WORKTREE)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "RemoveWorktree"))]
    async fn remove_worktree(
        &self,
        request: Request<RemoveWorktreeRequest>,
    ) -> Result<Response<RemoveWorktreeResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_REMOVE_WORKTREE)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListWorktrees"))]
    async fn list_worktrees(
        &self,
        request: Request<ListWorktreesRequest>,
    ) -> Result<Response<ListWorktreesResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_LIST_WORKTREES)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "GetWorktree"))]
    async fn get_worktree(
        &self,
        request: Request<GetWorktreeRequest>,
    ) -> Result<Response<WorktreeDetail>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_GET_WORKTREE)
            .await
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#[path = "worktree_proxy_tests.rs"]
mod tests;
