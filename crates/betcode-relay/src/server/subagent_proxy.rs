//! `SubagentService` proxy that forwards calls through the tunnel to daemons.

use std::pin::Pin;
use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::subagent_service_server::SubagentService;
use betcode_proto::v1::{
    CancelSubagentRequest, CancelSubagentResponse, CreateOrchestrationRequest,
    CreateOrchestrationResponse, ListSubagentsRequest, ListSubagentsResponse, OrchestrationEvent,
    RevokeAutoApproveRequest, RevokeAutoApproveResponse, SendToSubagentRequest,
    SendToSubagentResponse, SpawnSubagentRequest, SpawnSubagentResponse, SubagentEvent,
    WatchOrchestrationRequest, WatchSubagentRequest,
};

use betcode_proto::methods::{
    METHOD_CANCEL_SUBAGENT, METHOD_CREATE_ORCHESTRATION, METHOD_LIST_SUBAGENTS,
    METHOD_REVOKE_AUTO_APPROVE, METHOD_SEND_TO_SUBAGENT, METHOD_SPAWN_SUBAGENT,
    METHOD_WATCH_ORCHESTRATION, METHOD_WATCH_SUBAGENT,
};

use crate::router::RequestRouter;
use crate::storage::RelayDatabase;

/// Proxies `SubagentService` calls through the tunnel to a target daemon.
pub struct SubagentProxyService {
    router: Arc<RequestRouter>,
    db: RelayDatabase,
}

impl SubagentProxyService {
    pub const fn new(router: Arc<RequestRouter>, db: RelayDatabase) -> Self {
        Self { router, db }
    }
}

#[tonic::async_trait]
impl SubagentService for SubagentProxyService {
    #[instrument(skip(self, request), fields(rpc = "SpawnSubagent"))]
    async fn spawn_subagent(
        &self,
        request: Request<SpawnSubagentRequest>,
    ) -> Result<Response<SpawnSubagentResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_SPAWN_SUBAGENT)
            .await
    }

    type WatchSubagentStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<SubagentEvent, Status>> + Send>>;

    #[instrument(skip(self, request), fields(rpc = "WatchSubagent"))]
    async fn watch_subagent(
        &self,
        request: Request<WatchSubagentRequest>,
    ) -> Result<Response<Self::WatchSubagentStream>, Status> {
        super::grpc_util::forward_stream_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_WATCH_SUBAGENT,
            64,
        )
        .await
    }

    #[instrument(skip(self, request), fields(rpc = "SendToSubagent"))]
    async fn send_to_subagent(
        &self,
        request: Request<SendToSubagentRequest>,
    ) -> Result<Response<SendToSubagentResponse>, Status> {
        super::grpc_util::forward_unary_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_SEND_TO_SUBAGENT,
        )
        .await
    }

    #[instrument(skip(self, request), fields(rpc = "CancelSubagent"))]
    async fn cancel_subagent(
        &self,
        request: Request<CancelSubagentRequest>,
    ) -> Result<Response<CancelSubagentResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_CANCEL_SUBAGENT)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListSubagents"))]
    async fn list_subagents(
        &self,
        request: Request<ListSubagentsRequest>,
    ) -> Result<Response<ListSubagentsResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_LIST_SUBAGENTS)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "CreateOrchestration"))]
    async fn create_orchestration(
        &self,
        request: Request<CreateOrchestrationRequest>,
    ) -> Result<Response<CreateOrchestrationResponse>, Status> {
        super::grpc_util::forward_unary_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_CREATE_ORCHESTRATION,
        )
        .await
    }

    type WatchOrchestrationStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<OrchestrationEvent, Status>> + Send>>;

    #[instrument(skip(self, request), fields(rpc = "WatchOrchestration"))]
    async fn watch_orchestration(
        &self,
        request: Request<WatchOrchestrationRequest>,
    ) -> Result<Response<Self::WatchOrchestrationStream>, Status> {
        super::grpc_util::forward_stream_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_WATCH_ORCHESTRATION,
            64,
        )
        .await
    }

    #[instrument(skip(self, request), fields(rpc = "RevokeAutoApprove"))]
    async fn revoke_auto_approve(
        &self,
        request: Request<RevokeAutoApproveRequest>,
    ) -> Result<Response<RevokeAutoApproveResponse>, Status> {
        super::grpc_util::forward_unary_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_REVOKE_AUTO_APPROVE,
        )
        .await
    }
}
