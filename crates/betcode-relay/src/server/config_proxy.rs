//! `ConfigService` proxy that forwards calls through the tunnel to daemons.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::config_service_server::ConfigService;
use betcode_proto::v1::{
    GetPermissionsRequest, GetSettingsRequest, ListMcpServersRequest, ListMcpServersResponse,
    PermissionRules, Settings, UpdateSettingsRequest,
};

use betcode_proto::methods::{
    METHOD_GET_PERMISSIONS, METHOD_GET_SETTINGS, METHOD_LIST_MCP_SERVERS, METHOD_UPDATE_SETTINGS,
};

use crate::router::RequestRouter;
use crate::server::agent_proxy::extract_machine_id;
use crate::server::interceptor::extract_claims;

/// Proxies `ConfigService` calls through the tunnel to a target daemon.
pub struct ConfigProxyService {
    router: Arc<RequestRouter>,
}

impl ConfigProxyService {
    pub const fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
    }
}

#[tonic::async_trait]
impl ConfigService for ConfigProxyService {
    #[instrument(skip(self, request), fields(rpc = "GetSettings"))]
    async fn get_settings(
        &self,
        request: Request<GetSettingsRequest>,
    ) -> Result<Response<Settings>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_GET_SETTINGS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "UpdateSettings"))]
    async fn update_settings(
        &self,
        request: Request<UpdateSettingsRequest>,
    ) -> Result<Response<Settings>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_UPDATE_SETTINGS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ListMcpServers"))]
    async fn list_mcp_servers(
        &self,
        request: Request<ListMcpServersRequest>,
    ) -> Result<Response<ListMcpServersResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_LIST_MCP_SERVERS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "GetPermissions"))]
    async fn get_permissions(
        &self,
        request: Request<GetPermissionsRequest>,
    ) -> Result<Response<PermissionRules>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_GET_PERMISSIONS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#[path = "config_proxy_tests.rs"]
mod tests;
