//! `VersionService` gRPC implementation.
//!
//! Provides version discovery and capability negotiation for clients
//! connecting to the daemon.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::OnceCell;
use tonic::{Request, Response, Status};
use tracing::instrument;

use betcode_proto::v1::{
    CapabilitySet, ClaudeCodeInfo, CompatibilityLevel, GetVersionRequest, GetVersionResponse,
    NegotiateRequest, NegotiateResponse, VersionConstraints,
    version_service_server::VersionService,
};

use super::config::ServerConfig;

/// Minimum client version required to connect.
const MIN_CLIENT_VERSION: &str = "0.1.0";

/// Compare two semver version strings.
/// Returns `true` if `version` >= `min_version`.
/// Non-parseable versions return `false`.
fn version_satisfies_min(version: &str, min_version: &str) -> bool {
    match (
        semver::Version::parse(version),
        semver::Version::parse(min_version),
    ) {
        (Ok(v), Ok(m)) => v >= m,
        _ => false,
    }
}

/// Cached result of running `claude --version`.
static CLAUDE_VERSION_CACHE: OnceCell<Option<String>> = OnceCell::const_new();

/// Detect the installed Claude Code version by running `claude --version`.
async fn detect_claude_version() -> Option<String> {
    let output = tokio::process::Command::new("claude")
        .arg("--version")
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() { None } else { Some(raw) }
    } else {
        None
    }
}

/// Get (or lazily detect) the Claude Code version string.
async fn claude_version() -> Option<String> {
    CLAUDE_VERSION_CACHE
        .get_or_init(detect_claude_version)
        .await
        .clone()
}

/// `VersionService` implementation.
#[derive(Clone)]
pub struct VersionServiceImpl {
    feature_flags: Arc<HashMap<String, bool>>,
}

impl VersionServiceImpl {
    /// Create a new `VersionServiceImpl`.
    pub fn new(_config: ServerConfig, feature_flags: HashMap<String, bool>) -> Self {
        Self {
            feature_flags: Arc::new(feature_flags),
        }
    }

    /// Build the capability set from current configuration.
    fn build_capability_set(&self) -> CapabilitySet {
        CapabilitySet {
            streaming_supported: true,
            compression_supported: false,
            max_message_size: 10 * 1024 * 1024, // 10 MB
            available_tools: Vec::new(),
            available_models: vec!["claude-sonnet-4-5-20250929".to_string()],
            subagents_enabled: self
                .feature_flags
                .get("subagents")
                .copied()
                .unwrap_or(false),
            worktrees_enabled: true,
            feature_flags: self
                .feature_flags
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        }
    }

    /// Check whether a requested feature is supported.
    fn is_feature_supported(feature: &str) -> bool {
        matches!(
            feature,
            "streaming" | "worktrees" | "plugins" | "session_grants" | "e2e_encryption"
        )
    }
}

#[tonic::async_trait]
impl VersionService for VersionServiceImpl {
    #[instrument(skip(self, _request), fields(rpc = "GetVersion"))]
    async fn get_version(
        &self,
        _request: Request<GetVersionRequest>,
    ) -> Result<Response<GetVersionResponse>, Status> {
        let server_version = env!("CARGO_PKG_VERSION").to_string();
        let api_version = "v1".to_string();

        let features: Vec<String> = self
            .feature_flags
            .iter()
            .filter(|(_, v)| **v)
            .map(|(k, _)| k.clone())
            .collect();

        let claude_ver = claude_version().await;
        let claude_code = claude_ver.map(|v| ClaudeCodeInfo {
            version: v,
            api_version: "v1".to_string(),
            compatibility: CompatibilityLevel::FullyCompatible.into(),
        });

        let constraints = VersionConstraints {
            min_client_version: "0.1.0".to_string(),
            recommended_client: server_version.clone(),
            deprecated_features: Vec::new(),
            feature_replacements: HashMap::new(),
        };

        Ok(Response::new(GetVersionResponse {
            api_version,
            server_version,
            features,
            claude_code,
            constraints: Some(constraints),
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "NegotiateCapabilities"))]
    async fn negotiate_capabilities(
        &self,
        request: Request<NegotiateRequest>,
    ) -> Result<Response<NegotiateResponse>, Status> {
        let req = request.into_inner();

        let mut granted_features = Vec::new();
        let mut warnings = Vec::new();

        for feature in &req.requested_features {
            if Self::is_feature_supported(feature) {
                granted_features.push(feature.clone());
            } else {
                warnings.push(format!("Unsupported feature: {feature}"));
            }
        }

        // Version check: reject clients older than MIN_CLIENT_VERSION
        let (accepted, rejection_reason) = if req.client_version.is_empty() {
            warnings.push("Client version not specified; cannot verify compatibility".to_string());
            (true, String::new())
        } else if !version_satisfies_min(&req.client_version, MIN_CLIENT_VERSION) {
            (
                false,
                format!(
                    "Client version {} is below minimum required version {MIN_CLIENT_VERSION}",
                    req.client_version
                ),
            )
        } else {
            (true, String::new())
        };

        let capabilities = self.build_capability_set();

        Ok(Response::new(NegotiateResponse {
            accepted,
            rejection_reason,
            upgrade_url: String::new(),
            granted_features,
            warnings,
            capabilities: Some(capabilities),
        }))
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_pass_by_value
)]
mod tests {
    use super::*;

    fn test_service() -> VersionServiceImpl {
        let mut flags = HashMap::new();
        flags.insert("subagents".to_string(), true);
        flags.insert("e2e_encryption".to_string(), true);
        VersionServiceImpl::new(ServerConfig::default(), flags)
    }

    fn test_service_no_flags() -> VersionServiceImpl {
        VersionServiceImpl::new(ServerConfig::default(), HashMap::new())
    }

    fn negotiate_request(version: &str, features: Vec<String>) -> Request<NegotiateRequest> {
        Request::new(NegotiateRequest {
            client_version: version.to_string(),
            client_type: "cli".to_string(),
            requested_features: features,
            client_capabilities: HashMap::new(),
        })
    }

    #[tokio::test]
    async fn get_version_returns_server_version() {
        let svc = test_service();
        let resp = svc
            .get_version(Request::new(GetVersionRequest {}))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert_eq!(inner.api_version, "v1");
        assert!(!inner.server_version.is_empty());
        assert!(inner.constraints.is_some());
    }

    #[tokio::test]
    async fn get_version_includes_enabled_features() {
        let svc = test_service();
        let resp = svc
            .get_version(Request::new(GetVersionRequest {}))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert!(inner.features.contains(&"subagents".to_string()));
        assert!(inner.features.contains(&"e2e_encryption".to_string()));
    }

    #[tokio::test]
    async fn get_version_no_flags_empty_features() {
        let svc = test_service_no_flags();
        let resp = svc
            .get_version(Request::new(GetVersionRequest {}))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert!(inner.features.is_empty());
    }

    #[tokio::test]
    async fn get_version_constraints_min_version() {
        let svc = test_service();
        let resp = svc
            .get_version(Request::new(GetVersionRequest {}))
            .await
            .unwrap();
        let constraints = resp.into_inner().constraints.unwrap();

        assert_eq!(constraints.min_client_version, "0.1.0");
    }

    #[tokio::test]
    async fn negotiate_grants_supported_features() {
        let svc = test_service();
        let features = vec![
            "streaming".to_string(),
            "worktrees".to_string(),
            "nonexistent".to_string(),
        ];
        let resp = svc
            .negotiate_capabilities(negotiate_request("0.1.0", features))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert!(inner.accepted);
        assert!(inner.granted_features.contains(&"streaming".to_string()));
        assert!(inner.granted_features.contains(&"worktrees".to_string()));
        assert!(!inner.granted_features.contains(&"nonexistent".to_string()));
    }

    #[tokio::test]
    async fn negotiate_warns_unsupported_features() {
        let svc = test_service();
        let resp = svc
            .negotiate_capabilities(negotiate_request(
                "0.1.0",
                vec!["nonexistent_feature".to_string()],
            ))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert!(inner.accepted);
        assert!(
            inner
                .warnings
                .iter()
                .any(|w| w.contains("nonexistent_feature"))
        );
    }

    #[tokio::test]
    async fn negotiate_warns_empty_client_version() {
        let svc = test_service();
        let resp = svc
            .negotiate_capabilities(negotiate_request("", vec![]))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert!(inner.accepted);
        assert!(
            inner
                .warnings
                .iter()
                .any(|w| w.contains("Client version not specified"))
        );
    }

    #[tokio::test]
    async fn negotiate_returns_capability_set() {
        let svc = test_service();
        let resp = svc
            .negotiate_capabilities(negotiate_request("0.1.0", vec![]))
            .await
            .unwrap();
        let caps = resp.into_inner().capabilities.unwrap();

        assert!(caps.streaming_supported);
        assert!(!caps.compression_supported);
        assert!(caps.subagents_enabled);
        assert!(caps.worktrees_enabled);
        assert_eq!(caps.max_message_size, 10 * 1024 * 1024);
    }

    #[tokio::test]
    async fn negotiate_subagents_disabled_when_flag_off() {
        let svc = test_service_no_flags();
        let resp = svc
            .negotiate_capabilities(negotiate_request("0.1.0", vec![]))
            .await
            .unwrap();
        let caps = resp.into_inner().capabilities.unwrap();

        assert!(!caps.subagents_enabled);
    }

    #[tokio::test]
    async fn negotiate_rejects_old_client_version() {
        let svc = test_service();
        let resp = svc
            .negotiate_capabilities(negotiate_request("0.0.9", vec![]))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert!(!inner.accepted);
        assert!(inner.rejection_reason.contains("below minimum"));
    }

    #[tokio::test]
    async fn negotiate_accepts_exact_min_version() {
        let svc = test_service();
        let resp = svc
            .negotiate_capabilities(negotiate_request("0.1.0", vec![]))
            .await
            .unwrap();
        let inner = resp.into_inner();

        assert!(inner.accepted);
        assert!(inner.rejection_reason.is_empty());
    }

    #[tokio::test]
    async fn negotiate_accepts_newer_client() {
        let svc = test_service();
        let resp = svc
            .negotiate_capabilities(negotiate_request("1.0.0", vec![]))
            .await
            .unwrap();

        assert!(resp.into_inner().accepted);
    }

    #[test]
    fn version_comparison_basic() {
        assert!(version_satisfies_min("0.1.0", "0.1.0"));
        assert!(version_satisfies_min("0.2.0", "0.1.0"));
        assert!(version_satisfies_min("1.0.0", "0.1.0"));
        assert!(!version_satisfies_min("0.0.9", "0.1.0"));
        assert!(!version_satisfies_min("0.0.1", "0.1.0"));
    }

    #[test]
    fn version_comparison_with_prerelease() {
        // In semver, pre-release versions are less than their release counterpart
        assert!(!version_satisfies_min("0.1.0-alpha.1", "0.1.0"));
        assert!(version_satisfies_min("0.1.1-alpha.1", "0.1.0"));
        assert!(!version_satisfies_min("0.0.9-beta.1", "0.1.0"));
    }

    #[test]
    fn version_comparison_invalid() {
        assert!(!version_satisfies_min("invalid", "0.1.0"));
        assert!(!version_satisfies_min("", "0.1.0"));
        assert!(!version_satisfies_min("1.0", "0.1.0"));
    }
}
