//! `GitRepoService` proxy that forwards calls through the tunnel to daemons.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::git_repo_service_server::GitRepoService;
use betcode_proto::v1::{
    GetRepoRequest, GitRepoDetail, ListReposRequest, ListReposResponse, RegisterRepoRequest,
    ScanReposRequest, UnregisterRepoRequest, UnregisterRepoResponse, UpdateRepoRequest,
};

use betcode_proto::methods::{
    METHOD_GET_REPO, METHOD_LIST_REPOS, METHOD_REGISTER_REPO, METHOD_SCAN_REPOS,
    METHOD_UNREGISTER_REPO, METHOD_UPDATE_REPO,
};

use crate::router::RequestRouter;
use crate::storage::RelayDatabase;

/// Proxies `GitRepoService` calls through the tunnel to a target daemon.
pub struct GitRepoProxyService {
    router: Arc<RequestRouter>,
    db: RelayDatabase,
}

impl GitRepoProxyService {
    pub const fn new(router: Arc<RequestRouter>, db: RelayDatabase) -> Self {
        Self { router, db }
    }
}

#[tonic::async_trait]
impl GitRepoService for GitRepoProxyService {
    #[instrument(skip(self, request), fields(rpc = "RegisterRepo"))]
    async fn register_repo(
        &self,
        request: Request<RegisterRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_REGISTER_REPO)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "UnregisterRepo"))]
    async fn unregister_repo(
        &self,
        request: Request<UnregisterRepoRequest>,
    ) -> Result<Response<UnregisterRepoResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_UNREGISTER_REPO)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListRepos"))]
    async fn list_repos(
        &self,
        request: Request<ListReposRequest>,
    ) -> Result<Response<ListReposResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_LIST_REPOS)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "GetRepo"))]
    async fn get_repo(
        &self,
        request: Request<GetRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_GET_REPO).await
    }

    #[instrument(skip(self, request), fields(rpc = "UpdateRepo"))]
    async fn update_repo(
        &self,
        request: Request<UpdateRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_UPDATE_REPO)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "ScanRepos"))]
    async fn scan_repos(
        &self,
        request: Request<ScanReposRequest>,
    ) -> Result<Response<ListReposResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_SCAN_REPOS)
            .await
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#[path = "git_repo_proxy_tests.rs"]
mod tests;
