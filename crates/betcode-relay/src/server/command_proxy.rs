//! `CommandService` proxy that forwards calls through the tunnel to daemons.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use prost::Message;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{instrument, warn};

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
use crate::server::agent_proxy::{extract_machine_id, router_error_to_status};
use crate::server::interceptor::extract_claims;

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
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_GET_COMMAND_REGISTRY,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ListAgents"))]
    async fn list_agents(
        &self,
        request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_LIST_AGENTS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ListPath"))]
    async fn list_path(
        &self,
        request: Request<ListPathRequest>,
    ) -> Result<Response<ListPathResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_LIST_PATH,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ExecuteServiceCommand"))]
    async fn execute_service_command(
        &self,
        request: Request<ExecuteServiceCommandRequest>,
    ) -> Result<Response<Self::ExecuteServiceCommandStream>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let req = request.into_inner();
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut buf = Vec::with_capacity(req.encoded_len());
        req.encode(&mut buf)
            .map_err(|e| Status::internal(format!("Encode error: {e}")))?;

        let mut stream_rx = self
            .router
            .forward_stream(
                &machine_id,
                &request_id,
                METHOD_EXECUTE_SERVICE_COMMAND,
                buf,
                HashMap::new(),
            )
            .await
            .map_err(router_error_to_status)?;

        let (tx, rx) = mpsc::channel::<Result<ServiceCommandOutput, Status>>(64);
        let mid = machine_id;
        tokio::spawn(async move {
            use betcode_proto::v1::{tunnel_frame, FrameType};
            while let Some(frame) = stream_rx.recv().await {
                match FrameType::try_from(frame.frame_type) {
                    Ok(FrameType::StreamData) => {
                        if let Some(tunnel_frame::Payload::StreamData(p)) = frame.payload {
                            let data = p
                                .encrypted
                                .as_ref()
                                .map_or(&[][..], |e| &e.ciphertext[..]);
                            match ServiceCommandOutput::decode(data) {
                                Ok(output) => {
                                    if tx.send(Ok(output)).await.is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, machine_id = %mid, "Failed to decode ServiceCommandOutput");
                                }
                            }
                        }
                    }
                    Ok(FrameType::Error) => {
                        if let Some(tunnel_frame::Payload::Error(e)) = frame.payload {
                            let _ = tx
                                .send(Err(Status::internal(format!(
                                    "Daemon error: {}",
                                    e.message
                                ))))
                                .await;
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    #[instrument(skip(self, request), fields(rpc = "ListPlugins"))]
    async fn list_plugins(
        &self,
        request: Request<ListPluginsRequest>,
    ) -> Result<Response<ListPluginsResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_LIST_PLUGINS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "GetPluginStatus"))]
    async fn get_plugin_status(
        &self,
        request: Request<GetPluginStatusRequest>,
    ) -> Result<Response<GetPluginStatusResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_GET_PLUGIN_STATUS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "AddPlugin"))]
    async fn add_plugin(
        &self,
        request: Request<AddPluginRequest>,
    ) -> Result<Response<AddPluginResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_ADD_PLUGIN,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "RemovePlugin"))]
    async fn remove_plugin(
        &self,
        request: Request<RemovePluginRequest>,
    ) -> Result<Response<RemovePluginResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_REMOVE_PLUGIN,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "EnablePlugin"))]
    async fn enable_plugin(
        &self,
        request: Request<EnablePluginRequest>,
    ) -> Result<Response<EnablePluginResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_ENABLE_PLUGIN,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "DisablePlugin"))]
    async fn disable_plugin(
        &self,
        request: Request<DisablePluginRequest>,
    ) -> Result<Response<DisablePluginResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_DISABLE_PLUGIN,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
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
