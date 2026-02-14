//! `CommandService` proxy that forwards calls through the tunnel to daemons.

use std::pin::Pin;
use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::command_service_server::CommandService;
use betcode_proto::v1::{
    AddPluginRequest, AddPluginResponse, DisablePluginRequest, DisablePluginResponse,
    EnablePluginRequest, EnablePluginResponse, ExecuteServiceCommandRequest,
    GetCommandRegistryRequest, GetCommandRegistryResponse, GetPluginStatusRequest,
    GetPluginStatusResponse, ListAgentsRequest, ListAgentsResponse, ListPathRequest,
    ListPathResponse, ListPluginsRequest, ListPluginsResponse, RemovePluginRequest,
    RemovePluginResponse, ServiceCommandOutput,
};

use betcode_proto::methods::{
    METHOD_ADD_PLUGIN, METHOD_DISABLE_PLUGIN, METHOD_ENABLE_PLUGIN, METHOD_EXECUTE_SERVICE_COMMAND,
    METHOD_GET_COMMAND_REGISTRY, METHOD_GET_PLUGIN_STATUS, METHOD_LIST_AGENTS, METHOD_LIST_PATH,
    METHOD_LIST_PLUGINS, METHOD_REMOVE_PLUGIN,
};

use crate::router::RequestRouter;

type ServiceCommandStream =
    Pin<Box<dyn tokio_stream::Stream<Item = Result<ServiceCommandOutput, Status>> + Send>>;

/// Proxies `CommandService` calls through the tunnel to a target daemon.
pub struct CommandProxyService {
    router: Arc<RequestRouter>,
}

impl CommandProxyService {
    pub const fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
    }
}

#[tonic::async_trait]
impl CommandService for CommandProxyService {
    type ExecuteServiceCommandStream = ServiceCommandStream;

    #[instrument(skip(self, request), fields(rpc = "GetCommandRegistry"))]
    async fn get_command_registry(
        &self,
        request: Request<GetCommandRegistryRequest>,
    ) -> Result<Response<GetCommandRegistryResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_GET_COMMAND_REGISTRY)
            .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListAgents"))]
    async fn list_agents(
        &self,
        request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_LIST_AGENTS).await
    }

    #[instrument(skip(self, request), fields(rpc = "ListPath"))]
    async fn list_path(
        &self,
        request: Request<ListPathRequest>,
    ) -> Result<Response<ListPathResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_LIST_PATH).await
    }

    #[instrument(skip(self, request), fields(rpc = "ExecuteServiceCommand"))]
    async fn execute_service_command(
        &self,
        request: Request<ExecuteServiceCommandRequest>,
    ) -> Result<Response<Self::ExecuteServiceCommandStream>, Status> {
        super::grpc_util::forward_stream_rpc(
            &self.router,
            request,
            METHOD_EXECUTE_SERVICE_COMMAND,
            64,
        )
        .await
    }

    #[instrument(skip(self, request), fields(rpc = "ListPlugins"))]
    async fn list_plugins(
        &self,
        request: Request<ListPluginsRequest>,
    ) -> Result<Response<ListPluginsResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_LIST_PLUGINS).await
    }

    #[instrument(skip(self, request), fields(rpc = "GetPluginStatus"))]
    async fn get_plugin_status(
        &self,
        request: Request<GetPluginStatusRequest>,
    ) -> Result<Response<GetPluginStatusResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_GET_PLUGIN_STATUS).await
    }

    #[instrument(skip(self, request), fields(rpc = "AddPlugin"))]
    async fn add_plugin(
        &self,
        request: Request<AddPluginRequest>,
    ) -> Result<Response<AddPluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_ADD_PLUGIN).await
    }

    #[instrument(skip(self, request), fields(rpc = "RemovePlugin"))]
    async fn remove_plugin(
        &self,
        request: Request<RemovePluginRequest>,
    ) -> Result<Response<RemovePluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_REMOVE_PLUGIN).await
    }

    #[instrument(skip(self, request), fields(rpc = "EnablePlugin"))]
    async fn enable_plugin(
        &self,
        request: Request<EnablePluginRequest>,
    ) -> Result<Response<EnablePluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_ENABLE_PLUGIN).await
    }

    #[instrument(skip(self, request), fields(rpc = "DisablePlugin"))]
    async fn disable_plugin(
        &self,
        request: Request<DisablePluginRequest>,
    ) -> Result<Response<DisablePluginResponse>, Status> {
        super::grpc_util::forward_unary_rpc(&self.router, request, METHOD_DISABLE_PLUGIN).await
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
