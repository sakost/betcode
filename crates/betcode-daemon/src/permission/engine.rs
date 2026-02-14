//! Daemon permission engine.
//!
//! Integrates betcode-core permission rules with session grants and pending requests.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info};

use betcode_core::permissions::{PermissionAction, PermissionEngine};

use crate::storage::Database;

use super::pending::{PendingConfig, PendingManager, PendingRequest, PendingRequestParams};
use super::types::{
    PermissionError, PermissionEvaluation, PermissionResponse, ProcessedResponse, SessionGrant,
};

/// Parameters for a permission evaluation request.
pub struct PermissionEvalRequest<'a> {
    /// Session identifier.
    pub session_id: &'a str,
    /// Unique request identifier.
    pub request_id: &'a str,
    /// Tool name being requested.
    pub tool_name: &'a str,
    /// Human-readable description.
    pub description: &'a str,
    /// Tool input as JSON.
    pub input_json: &'a str,
    /// Optional file path context.
    pub path: Option<&'a Path>,
    /// Client that should respond (if any).
    pub target_client: Option<&'a str>,
    /// Whether the client is currently connected.
    pub client_connected: bool,
}

/// Daemon permission engine with session grants and pending tracking.
pub struct DaemonPermissionEngine {
    rule_engine: PermissionEngine,
    pending: PendingManager,
    session_grants: Arc<RwLock<HashMap<String, Vec<SessionGrant>>>>,
    db: Option<Database>,
}

impl DaemonPermissionEngine {
    /// Create a new daemon permission engine.
    pub fn new(rule_engine: PermissionEngine, pending_config: PendingConfig) -> Self {
        Self {
            rule_engine,
            pending: PendingManager::new(pending_config),
            session_grants: Arc::new(RwLock::new(HashMap::new())),
            db: None,
        }
    }

    /// Create with database for persistent grants.
    pub fn with_database(
        rule_engine: PermissionEngine,
        pending_config: PendingConfig,
        db: Database,
    ) -> Self {
        Self {
            rule_engine,
            pending: PendingManager::new(pending_config),
            session_grants: Arc::new(RwLock::new(HashMap::new())),
            db: Some(db),
        }
    }

    /// Evaluate a permission request.
    pub async fn evaluate(&self, req: &PermissionEvalRequest<'_>) -> PermissionEvaluation {
        // 1. Check session grants first
        if let Some(grant) = self
            .check_session_grant(req.session_id, req.tool_name, req.path)
            .await
        {
            debug!(
                session_id = req.session_id,
                tool_name = req.tool_name,
                granted = grant,
                "Session grant hit"
            );
            return if grant {
                PermissionEvaluation::Allowed { cached: true }
            } else {
                PermissionEvaluation::Denied { cached: true }
            };
        }

        // 2. Check database grants
        if let Some(ref db) = self.db {
            if let Ok(Some(grant)) = db.get_permission_grant(req.session_id, req.tool_name).await {
                let granted = grant.action == "allow";
                debug!(
                    session_id = req.session_id,
                    tool_name = req.tool_name,
                    granted,
                    "Database grant hit"
                );
                return if granted {
                    PermissionEvaluation::Allowed { cached: true }
                } else {
                    PermissionEvaluation::Denied { cached: true }
                };
            }
        }

        // 3. Evaluate against rules
        let decision = self.rule_engine.evaluate(req.tool_name, req.path);

        match decision.action {
            PermissionAction::Allow => {
                debug!(
                    session_id = req.session_id,
                    tool_name = req.tool_name,
                    rule = ?decision.rule_id,
                    "Rule allows"
                );
                PermissionEvaluation::Allowed { cached: false }
            }
            PermissionAction::Deny => {
                debug!(
                    session_id = req.session_id,
                    tool_name = req.tool_name,
                    rule = ?decision.rule_id,
                    "Rule denies"
                );
                PermissionEvaluation::Denied { cached: false }
            }
            PermissionAction::Ask | PermissionAction::AskSession => {
                let request = self
                    .pending
                    .create(PendingRequestParams {
                        request_id: req.request_id.to_string(),
                        session_id: req.session_id.to_string(),
                        tool_name: req.tool_name.to_string(),
                        description: req.description.to_string(),
                        input_json: req.input_json.to_string(),
                        target_client: req.target_client.map(String::from),
                        client_connected: req.client_connected,
                    })
                    .await;

                info!(
                    session_id = req.session_id,
                    request_id = req.request_id,
                    tool_name = req.tool_name,
                    "Permission request pending"
                );
                PermissionEvaluation::Pending { request }
            }
        }
    }

    /// Process a permission response from a client.
    pub async fn process_response(
        &self,
        response: PermissionResponse,
    ) -> Result<ProcessedResponse, PermissionError> {
        let request = self
            .pending
            .take(&response.request_id)
            .await
            .ok_or_else(|| PermissionError::RequestNotFound {
                request_id: response.request_id.clone(),
            })?;

        if response.remember_session {
            self.add_session_grant(
                &request.session_id,
                &request.tool_name,
                None,
                response.granted,
            )
            .await;
        }

        if response.remember_permanent {
            if let Some(ref db) = self.db {
                let action = if response.granted { "allow" } else { "deny" };
                if let Err(e) = db
                    .insert_permission_grant(&request.session_id, &request.tool_name, None, action)
                    .await
                {
                    tracing::error!(?e, "Failed to store permission grant");
                }
            }
        }

        info!(
            request_id = %response.request_id,
            tool_name = %request.tool_name,
            granted = response.granted,
            "Permission response processed"
        );

        Ok(ProcessedResponse {
            request,
            granted: response.granted,
        })
    }

    /// Add a session-scoped grant.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn add_session_grant(
        &self,
        session_id: &str,
        tool_name: &str,
        path_pattern: Option<&str>,
        granted: bool,
    ) {
        let mut guard = self.session_grants.write().await;
        guard
            .entry(session_id.to_string())
            .or_default()
            .push(SessionGrant {
                tool_name: tool_name.to_string(),
                path_pattern: path_pattern.map(String::from),
                granted,
            });
        drop(guard);

        debug!(session_id, tool_name, granted, "Added session grant");
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn check_session_grant(
        &self,
        session_id: &str,
        tool_name: &str,
        path: Option<&Path>,
    ) -> Option<bool> {
        let grants = self.session_grants.read().await;
        let session_grants = grants.get(session_id)?;

        for grant in session_grants.iter().rev() {
            if grant.tool_name == tool_name {
                if let Some(ref pattern) = grant.path_pattern {
                    if let Some(p) = path {
                        if p.to_string_lossy().starts_with(pattern) {
                            return Some(grant.granted);
                        }
                    }
                } else {
                    return Some(grant.granted);
                }
            }
        }
        None
    }

    /// Clear session grants.
    pub async fn clear_session_grants(&self, session_id: &str) {
        self.session_grants.write().await.remove(session_id);
        debug!(session_id, "Cleared session grants");
    }

    /// Get a pending request by ID.
    pub async fn get_pending(&self, request_id: &str) -> Option<PendingRequest> {
        self.pending.get(request_id).await
    }

    /// Get all pending requests for a session.
    pub async fn get_pending_for_session(&self, session_id: &str) -> Vec<PendingRequest> {
        self.pending.get_for_session(session_id).await
    }

    /// Update client connection status for pending requests.
    pub async fn update_client_status(&self, client_id: &str, connected: bool) {
        self.pending
            .update_client_status(client_id, connected)
            .await;
    }

    /// Clean up expired pending requests.
    pub async fn cleanup_expired(&self) -> Vec<String> {
        self.pending.cleanup_expired().await
    }

    /// Get the underlying rule engine.
    pub const fn rule_engine(&self) -> &PermissionEngine {
        &self.rule_engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> DaemonPermissionEngine {
        DaemonPermissionEngine::new(PermissionEngine::new(), PendingConfig::default())
    }

    fn eval_req<'a>(
        session_id: &'a str,
        request_id: &'a str,
        tool_name: &'a str,
    ) -> PermissionEvalRequest<'a> {
        PermissionEvalRequest {
            session_id,
            request_id,
            tool_name,
            description: "",
            input_json: "{}",
            path: None,
            target_client: None,
            client_connected: true,
        }
    }

    #[tokio::test]
    async fn allows_read_by_default() {
        let engine = test_engine();

        let result = engine
            .evaluate(&eval_req("session-1", "req-1", "Read"))
            .await;

        assert!(matches!(
            result,
            PermissionEvaluation::Allowed { cached: false }
        ));
    }

    #[tokio::test]
    async fn asks_for_bash_by_default() {
        let engine = test_engine();

        let result = engine
            .evaluate(&eval_req("session-1", "req-1", "Bash"))
            .await;

        assert!(matches!(result, PermissionEvaluation::Pending { .. }));
    }

    #[tokio::test]
    async fn session_grant_cached() {
        let engine = test_engine();

        engine
            .add_session_grant("session-1", "Bash", None, true)
            .await;

        let result = engine
            .evaluate(&eval_req("session-1", "req-1", "Bash"))
            .await;

        assert!(matches!(
            result,
            PermissionEvaluation::Allowed { cached: true }
        ));
    }

    #[tokio::test]
    async fn process_permission_response() {
        let engine = test_engine();

        engine
            .evaluate(&eval_req("session-1", "req-1", "Bash"))
            .await;

        let response = PermissionResponse {
            request_id: "req-1".to_string(),
            granted: true,
            remember_session: true,
            remember_permanent: false,
        };

        let result = engine.process_response(response).await.unwrap();
        assert!(result.granted);

        let result = engine
            .evaluate(&eval_req("session-1", "req-2", "Bash"))
            .await;

        assert!(matches!(
            result,
            PermissionEvaluation::Allowed { cached: true }
        ));
    }
}
