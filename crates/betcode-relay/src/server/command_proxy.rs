//! `CommandService` proxy that forwards calls through the tunnel to daemons.

use std::pin::Pin;
use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::{self, command_service_server::CommandService};

use betcode_proto::methods::{
    METHOD_ADD_PLUGIN, METHOD_DISABLE_PLUGIN, METHOD_ENABLE_PLUGIN, METHOD_EXECUTE_SERVICE_COMMAND,
    METHOD_GET_COMMAND_REGISTRY, METHOD_GET_PLUGIN_STATUS, METHOD_LIST_AGENTS, METHOD_LIST_PATH,
    METHOD_LIST_PLUGINS, METHOD_REMOVE_PLUGIN,
};

use crate::router::RequestRouter;
use crate::storage::RelayDatabase;

type ServiceCommandStream =
    Pin<Box<dyn tokio_stream::Stream<Item = Result<v1::ServiceCommandOutput, Status>> + Send>>;

/// Proxies `CommandService` calls through the tunnel to a target daemon.
pub struct CommandProxyService {
    router: Arc<RequestRouter>,
    db: RelayDatabase,
}

impl CommandProxyService {
    pub const fn new(router: Arc<RequestRouter>, db: RelayDatabase) -> Self {
        Self { router, db }
    }
}

#[tonic::async_trait]
impl CommandService for CommandProxyService {
    type ExecuteServiceCommandStream = ServiceCommandStream;

    #[instrument(skip(self, request), fields(rpc = "GetCommandRegistry"))]
    async fn get_command_registry(
        &self,
        request: Request<v1::GetCommandRegistryRequest>,
    ) -> Result<Response<v1::GetCommandRegistryResponse>, Status> {
        super::grpc_util::forward_unary_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_GET_COMMAND_REGISTRY,
        )
        .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListAgents"))]
    async fn list_agents(
        &self,
        request: Request<v1::ListAgentsRequest>,
    ) -> Result<Response<v1::ListAgentsResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_LIST_AGENTS)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListPath"))]
    async fn list_path(
        &self,
        request: Request<v1::ListPathRequest>,
    ) -> Result<Response<v1::ListPathResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_LIST_PATH).await
    }

    #[instrument(skip(self, request), fields(rpc = "ExecuteServiceCommand"))]
    async fn execute_service_command(
        &self,
        request: Request<v1::ExecuteServiceCommandRequest>,
    ) -> Result<Response<Self::ExecuteServiceCommandStream>, Status> {
        super::grpc_util::forward_stream_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_EXECUTE_SERVICE_COMMAND,
            64,
        )
        .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListPlugins"))]
    async fn list_plugins(
        &self,
        request: Request<v1::ListPluginsRequest>,
    ) -> Result<Response<v1::ListPluginsResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_LIST_PLUGINS)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "GetPluginStatus"))]
    async fn get_plugin_status(
        &self,
        request: Request<v1::GetPluginStatusRequest>,
    ) -> Result<Response<v1::GetPluginStatusResponse>, Status> {
        super::grpc_util::forward_unary_rpc(
            &self.router,
            &self.db,
            request,
            METHOD_GET_PLUGIN_STATUS,
        )
        .await
    }

    #[instrument(skip(self, request), fields(rpc = "AddPlugin"))]
    async fn add_plugin(
        &self,
        request: Request<v1::AddPluginRequest>,
    ) -> Result<Response<v1::AddPluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_ADD_PLUGIN)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "RemovePlugin"))]
    async fn remove_plugin(
        &self,
        request: Request<v1::RemovePluginRequest>,
    ) -> Result<Response<v1::RemovePluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_REMOVE_PLUGIN)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "EnablePlugin"))]
    async fn enable_plugin(
        &self,
        request: Request<v1::EnablePluginRequest>,
    ) -> Result<Response<v1::EnablePluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_ENABLE_PLUGIN)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "DisablePlugin"))]
    async fn disable_plugin(
        &self,
        request: Request<v1::DisablePluginRequest>,
    ) -> Result<Response<v1::DisablePluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, &self.db, request, METHOD_DISABLE_PLUGIN)
            .await
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation
)]
#[path = "command_proxy_tests.rs"]
mod tests;
