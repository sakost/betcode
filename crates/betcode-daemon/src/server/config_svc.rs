//! `ConfigService` gRPC implementation.
//!
//! Returns sensible runtime defaults from `ServerConfig`.
//! Persistent config storage is not yet implemented.

use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::{
    config_service_server::ConfigService, DaemonSettings, GetPermissionsRequest,
    GetSettingsRequest, ListMcpServersRequest, ListMcpServersResponse, PermissionRules,
    PermissionSettings, SessionSettings, Settings, UpdateSettingsRequest,
};

use super::config::ServerConfig;

/// `ConfigService` implementation backed by `ServerConfig` runtime state.
#[derive(Clone)]
pub struct ConfigServiceImpl {
    config: ServerConfig,
}

impl ConfigServiceImpl {
    /// Create a new `ConfigServiceImpl`.
    pub const fn new(config: ServerConfig) -> Self {
        Self { config }
    }

    /// Build the current `Settings` response from runtime configuration.
    fn build_settings(&self) -> Settings {
        Settings {
            daemon: Some(DaemonSettings {
                max_subprocesses: 5,
                socket_path: String::new(),
                port: self.config.tcp_addr.map_or(0, |a| u32::from(a.port())),
                database_path: String::new(),
                log_level: "info".into(),
                max_payload_bytes: 0,
            }),
            sessions: Some(SessionSettings {
                default_model: "claude-sonnet-4-5-20250929".into(),
                auto_compact: true,
                auto_compact_threshold: 80,
                max_messages_per_session: 0,
            }),
            permissions: Some(PermissionSettings {
                connected_timeout_secs: 300,
                disconnected_timeout_secs: 60,
                enable_auto_approve: false,
                auto_approve_directories: vec![],
                activity_refresh_enabled: true,
            }),
            feature_flags: std::collections::HashMap::default(),
        }
    }
}

#[tonic::async_trait]
impl ConfigService for ConfigServiceImpl {
    #[instrument(skip(self, request), fields(rpc = "GetSettings"))]
    async fn get_settings(
        &self,
        request: Request<GetSettingsRequest>,
    ) -> Result<Response<Settings>, Status> {
        let _req = request.into_inner();
        // `scope` is ignored for now; always return full settings.
        Ok(Response::new(self.build_settings()))
    }

    #[instrument(skip(self, request), fields(rpc = "UpdateSettings"))]
    async fn update_settings(
        &self,
        request: Request<UpdateSettingsRequest>,
    ) -> Result<Response<Settings>, Status> {
        let _req = request.into_inner();
        Err(Status::unimplemented("UpdateSettings not yet implemented"))
    }

    #[instrument(skip(self, request), fields(rpc = "ListMcpServers"))]
    async fn list_mcp_servers(
        &self,
        request: Request<ListMcpServersRequest>,
    ) -> Result<Response<ListMcpServersResponse>, Status> {
        let _req = request.into_inner();
        // No MCP servers managed yet; return empty list.
        Ok(Response::new(ListMcpServersResponse { servers: vec![] }))
    }

    #[instrument(skip(self, request), fields(rpc = "GetPermissions"))]
    async fn get_permissions(
        &self,
        request: Request<GetPermissionsRequest>,
    ) -> Result<Response<PermissionRules>, Status> {
        let _req = request.into_inner();
        // No permission rules configured yet; return empty.
        Ok(Response::new(PermissionRules {
            rules: vec![],
            denied_tools: vec![],
            require_approval: vec![],
        }))
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Create a `ConfigServiceImpl` with default `ServerConfig`.
    fn test_service() -> ConfigServiceImpl {
        ConfigServiceImpl::new(ServerConfig::default())
    }

    #[tokio::test]
    async fn get_settings_returns_defaults() {
        let svc = test_service();

        let resp = svc
            .get_settings(Request::new(GetSettingsRequest {
                scope: String::new(),
            }))
            .await
            .unwrap();

        let settings = resp.into_inner();

        // Daemon settings
        let daemon = settings.daemon.expect("daemon settings present");
        assert_eq!(daemon.max_subprocesses, 5);
        assert_eq!(daemon.log_level, "info");
        // Default ServerConfig has tcp_addr = 127.0.0.1:50051
        assert_eq!(daemon.port, 50051);

        // Session settings
        let sessions = settings.sessions.expect("session settings present");
        assert_eq!(sessions.default_model, "claude-sonnet-4-5-20250929");
        assert!(sessions.auto_compact);
        assert_eq!(sessions.auto_compact_threshold, 80);

        // Permission settings
        let permissions = settings.permissions.expect("permission settings present");
        assert_eq!(permissions.connected_timeout_secs, 300);
        assert_eq!(permissions.disconnected_timeout_secs, 60);
        assert!(!permissions.enable_auto_approve);
        assert!(permissions.auto_approve_directories.is_empty());
        assert!(permissions.activity_refresh_enabled);

        // Feature flags (empty by default)
        assert!(settings.feature_flags.is_empty());
    }

    #[tokio::test]
    async fn get_settings_scope_is_ignored() {
        let svc = test_service();

        let resp = svc
            .get_settings(Request::new(GetSettingsRequest {
                scope: "user".into(),
            }))
            .await
            .unwrap();

        // Should still return defaults regardless of scope
        let settings = resp.into_inner();
        assert!(settings.daemon.is_some());
        assert!(settings.sessions.is_some());
        assert!(settings.permissions.is_some());
    }

    #[tokio::test]
    async fn get_settings_port_reflects_config() {
        let addr: std::net::SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let config = ServerConfig::tcp(addr);
        let svc = ConfigServiceImpl::new(config);

        let resp = svc
            .get_settings(Request::new(GetSettingsRequest {
                scope: String::new(),
            }))
            .await
            .unwrap();

        let daemon = resp.into_inner().daemon.unwrap();
        assert_eq!(daemon.port, 9999);
    }

    #[tokio::test]
    async fn get_settings_no_tcp_addr_port_is_zero() {
        let config = ServerConfig {
            tcp_addr: None,
            ..Default::default()
        };
        let svc = ConfigServiceImpl::new(config);

        let resp = svc
            .get_settings(Request::new(GetSettingsRequest {
                scope: String::new(),
            }))
            .await
            .unwrap();

        let daemon = resp.into_inner().daemon.unwrap();
        assert_eq!(daemon.port, 0);
    }

    #[tokio::test]
    async fn update_settings_returns_unimplemented() {
        let svc = test_service();

        let result = svc
            .update_settings(Request::new(UpdateSettingsRequest {
                settings: None,
                scope: String::new(),
            }))
            .await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::Unimplemented);
        assert!(status.message().contains("UpdateSettings"));
    }

    #[tokio::test]
    async fn list_mcp_servers_returns_empty() {
        let svc = test_service();

        let resp = svc
            .list_mcp_servers(Request::new(ListMcpServersRequest {
                status_filter: String::new(),
            }))
            .await
            .unwrap();

        assert!(resp.into_inner().servers.is_empty());
    }

    #[tokio::test]
    async fn list_mcp_servers_filter_ignored() {
        let svc = test_service();

        let resp = svc
            .list_mcp_servers(Request::new(ListMcpServersRequest {
                status_filter: "running".into(),
            }))
            .await
            .unwrap();

        // Filter is ignored; still returns empty list
        assert!(resp.into_inner().servers.is_empty());
    }

    #[tokio::test]
    async fn get_permissions_returns_empty() {
        let svc = test_service();

        let resp = svc
            .get_permissions(Request::new(GetPermissionsRequest {
                session_id: String::new(),
            }))
            .await
            .unwrap();

        let rules = resp.into_inner();
        assert!(rules.rules.is_empty());
        assert!(rules.denied_tools.is_empty());
        assert!(rules.require_approval.is_empty());
    }

    #[tokio::test]
    async fn get_permissions_session_id_ignored() {
        let svc = test_service();

        let resp = svc
            .get_permissions(Request::new(GetPermissionsRequest {
                session_id: "some-session-id".into(),
            }))
            .await
            .unwrap();

        // session_id is ignored; still returns empty rules
        let rules = resp.into_inner();
        assert!(rules.rules.is_empty());
    }
}
