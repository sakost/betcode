//! `SubagentService` gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status};
use tracing::{info, instrument, warn};

use betcode_proto::v1::{
    CancelSubagentRequest, CancelSubagentResponse, CreateOrchestrationRequest,
    CreateOrchestrationResponse, ListSubagentsRequest, ListSubagentsResponse, OrchestrationEvent,
    OrchestrationStrategy, RevokeAutoApproveRequest, RevokeAutoApproveResponse,
    SendToSubagentRequest, SendToSubagentResponse, SpawnSubagentRequest, SpawnSubagentResponse,
    SubagentEvent, SubagentInfo, SubagentStatus, WatchOrchestrationRequest, WatchSubagentRequest,
    subagent_service_server::SubagentService,
};

use crate::orchestration::manager::{ManagerError, SubagentConfig, SubagentManager};
use crate::storage::Database;

/// `SubagentService` implementation backed by `SubagentManager`.
pub struct SubagentServiceImpl {
    manager: Arc<SubagentManager>,
    db: Database,
}

impl SubagentServiceImpl {
    /// Create a new `SubagentServiceImpl`.
    pub const fn new(manager: Arc<SubagentManager>, db: Database) -> Self {
        Self { manager, db }
    }
}

type SubagentEventStream = Pin<Box<dyn Stream<Item = Result<SubagentEvent, Status>> + Send>>;
type OrchestrationEventStream =
    Pin<Box<dyn Stream<Item = Result<OrchestrationEvent, Status>> + Send>>;

/// Map a `ManagerError` to a gRPC `Status`.
fn manager_err_to_status(e: &ManagerError) -> Status {
    match e {
        ManagerError::PoolFull => Status::resource_exhausted(e.to_string()),
        ManagerError::SpawnFailed { .. } => Status::internal(e.to_string()),
        ManagerError::NotFound { .. } | ManagerError::OrchestrationNotFound { .. } => {
            Status::not_found(e.to_string())
        }
        ManagerError::AlreadyCompleted { .. } => Status::failed_precondition(e.to_string()),
        ManagerError::Database(db_err) => {
            use crate::storage::DatabaseError;
            match db_err {
                DatabaseError::NotFound(_) => Status::not_found(e.to_string()),
                _ => Status::internal(e.to_string()),
            }
        }
        ManagerError::Validation { .. } => Status::invalid_argument(e.to_string()),
    }
}

/// Convert a DB status string to a proto `SubagentStatus`.
fn status_str_to_proto(s: &str) -> i32 {
    match s {
        "pending" => SubagentStatus::Pending.into(),
        "running" => SubagentStatus::Running.into(),
        "completed" => SubagentStatus::Completed.into(),
        "failed" => SubagentStatus::Failed.into(),
        "cancelled" => SubagentStatus::Cancelled.into(),
        _ => SubagentStatus::Unspecified.into(),
    }
}

/// Convert an `OrchestrationStrategy` enum value from its i32 form.
const fn strategy_from_i32(v: i32) -> OrchestrationStrategy {
    match v {
        2 => OrchestrationStrategy::Sequential,
        3 => OrchestrationStrategy::Dag,
        // 1 (Parallel) and all unknown values default to Parallel
        _ => OrchestrationStrategy::Parallel,
    }
}

#[tonic::async_trait]
impl SubagentService for SubagentServiceImpl {
    type WatchSubagentStream = SubagentEventStream;
    type WatchOrchestrationStream = OrchestrationEventStream;

    #[instrument(skip(self, request), fields(rpc = "SpawnSubagent"))]
    async fn spawn_subagent(
        &self,
        request: Request<SpawnSubagentRequest>,
    ) -> Result<Response<SpawnSubagentResponse>, Status> {
        let req = request.into_inner();

        if req.parent_session_id.is_empty() {
            return Err(Status::invalid_argument(
                "parent_session_id must not be empty",
            ));
        }
        if req.prompt.is_empty() {
            return Err(Status::invalid_argument("prompt must not be empty"));
        }
        if req.auto_approve && req.allowed_tools.is_empty() {
            return Err(Status::invalid_argument(
                "auto_approve requires non-empty allowed_tools",
            ));
        }

        let subagent_id = uuid::Uuid::new_v4().to_string();
        let working_dir = if req.working_directory.is_empty() {
            std::env::current_dir().unwrap_or_default()
        } else {
            std::path::PathBuf::from(&req.working_directory)
        };

        let config = SubagentConfig {
            id: subagent_id.clone(),
            parent_session_id: req.parent_session_id.clone(),
            prompt: req.prompt,
            model: if req.model.is_empty() {
                None
            } else {
                Some(req.model)
            },
            working_directory: working_dir,
            allowed_tools: req.allowed_tools,
            max_turns: req.max_turns,
            auto_approve: req.auto_approve,
            timeout_secs: 0,
        };

        let id = self
            .manager
            .spawn(config)
            .await
            .map_err(|e| manager_err_to_status(&e))?;

        info!(
            subagent_id = %id,
            parent_session_id = %req.parent_session_id,
            "Subagent spawned"
        );

        Ok(Response::new(SpawnSubagentResponse {
            subagent_id: id,
            session_id: String::new(),
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "WatchSubagent"))]
    async fn watch_subagent(
        &self,
        request: Request<WatchSubagentRequest>,
    ) -> Result<Response<Self::WatchSubagentStream>, Status> {
        let req = request.into_inner();

        if req.subagent_id.is_empty() {
            return Err(Status::invalid_argument("subagent_id must not be empty"));
        }

        let rx = self
            .manager
            .subscribe(&req.subagent_id)
            .await
            .map_err(|e| manager_err_to_status(&e))?;

        let stream = ReceiverStream::new(rx).map(Ok);
        Ok(Response::new(Box::pin(stream)))
    }

    #[instrument(skip(self, request), fields(rpc = "SendToSubagent"))]
    async fn send_to_subagent(
        &self,
        request: Request<SendToSubagentRequest>,
    ) -> Result<Response<SendToSubagentResponse>, Status> {
        let req = request.into_inner();

        if req.subagent_id.is_empty() {
            return Err(Status::invalid_argument("subagent_id must not be empty"));
        }

        let delivered = self
            .manager
            .send_input(&req.subagent_id, &req.content)
            .await
            .map_err(|e| manager_err_to_status(&e))?;

        Ok(Response::new(SendToSubagentResponse { delivered }))
    }

    #[instrument(skip(self, request), fields(rpc = "CancelSubagent"))]
    async fn cancel_subagent(
        &self,
        request: Request<CancelSubagentRequest>,
    ) -> Result<Response<CancelSubagentResponse>, Status> {
        let req = request.into_inner();

        if req.subagent_id.is_empty() {
            return Err(Status::invalid_argument("subagent_id must not be empty"));
        }

        let cancelled = self
            .manager
            .cancel(&req.subagent_id, &req.reason)
            .await
            .map_err(|e| manager_err_to_status(&e))?;

        Ok(Response::new(CancelSubagentResponse {
            cancelled,
            final_status: if cancelled {
                "cancelled".to_string()
            } else {
                "not_found".to_string()
            },
            tool_calls_executed: 0,
            tool_calls_auto_approved: 0,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "ListSubagents"))]
    async fn list_subagents(
        &self,
        request: Request<ListSubagentsRequest>,
    ) -> Result<Response<ListSubagentsResponse>, Status> {
        let req = request.into_inner();

        if req.parent_session_id.is_empty() {
            return Err(Status::invalid_argument(
                "parent_session_id must not be empty",
            ));
        }

        let status_filter = if req.status_filter.is_empty() {
            None
        } else {
            Some(req.status_filter.as_str())
        };

        let rows = self
            .db
            .list_subagents_for_session(&req.parent_session_id, status_filter)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let subagents: Vec<SubagentInfo> = rows
            .into_iter()
            .map(|row| {
                let allowed_tools: Vec<String> =
                    serde_json::from_str(&row.allowed_tools).unwrap_or_default();

                SubagentInfo {
                    id: row.id,
                    parent_session_id: row.parent_session_id,
                    session_id: String::new(),
                    name: String::new(),
                    prompt: row.prompt,
                    model: row.model.unwrap_or_default(),
                    working_directory: row.working_directory.unwrap_or_default(),
                    status: status_str_to_proto(&row.status),
                    auto_approve: row.auto_approve != 0,
                    #[allow(clippy::cast_possible_truncation)]
                    max_turns: row.max_turns as i32,
                    allowed_tools,
                    result_summary: row.result_summary.unwrap_or_default(),
                    created_at: Some(prost_types::Timestamp {
                        seconds: row.created_at,
                        nanos: 0,
                    }),
                    completed_at: row.completed_at.map(|ts| prost_types::Timestamp {
                        seconds: ts,
                        nanos: 0,
                    }),
                }
            })
            .collect();

        Ok(Response::new(ListSubagentsResponse { subagents }))
    }

    #[instrument(skip(self, request), fields(rpc = "CreateOrchestration"))]
    async fn create_orchestration(
        &self,
        request: Request<CreateOrchestrationRequest>,
    ) -> Result<Response<CreateOrchestrationResponse>, Status> {
        let req = request.into_inner();

        if req.parent_session_id.is_empty() {
            return Err(Status::invalid_argument(
                "parent_session_id must not be empty",
            ));
        }
        if req.steps.is_empty() {
            return Err(Status::invalid_argument("steps must not be empty"));
        }

        let strategy = strategy_from_i32(req.strategy);

        // Assign IDs to steps that don't have them
        let steps: Vec<betcode_proto::v1::OrchestrationStep> = req
            .steps
            .into_iter()
            .enumerate()
            .map(|(i, mut step)| {
                if step.id.is_empty() {
                    step.id = format!("step-{i}");
                }

                // For sequential strategy, chain dependencies
                if strategy == OrchestrationStrategy::Sequential && i > 0 {
                    let prev_id = format!("step-{}", i - 1);
                    if !step.depends_on.contains(&prev_id) {
                        step.depends_on.push(prev_id);
                    }
                }

                step
            })
            .collect();

        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let total_steps = steps.len() as i32;
        let orchestration_id = uuid::Uuid::new_v4().to_string();

        self.manager
            .run_orchestration(
                orchestration_id.clone(),
                req.parent_session_id.clone(),
                strategy,
                steps,
            )
            .await
            .map_err(|e| manager_err_to_status(&e))?;

        info!(
            orchestration_id = %orchestration_id,
            parent_session_id = %req.parent_session_id,
            total_steps,
            "Orchestration created"
        );

        Ok(Response::new(CreateOrchestrationResponse {
            orchestration_id,
            total_steps,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "WatchOrchestration"))]
    async fn watch_orchestration(
        &self,
        request: Request<WatchOrchestrationRequest>,
    ) -> Result<Response<Self::WatchOrchestrationStream>, Status> {
        let req = request.into_inner();

        if req.orchestration_id.is_empty() {
            return Err(Status::invalid_argument(
                "orchestration_id must not be empty",
            ));
        }

        // Subscribe to the orchestration's broadcast channel
        let mut broadcast_rx = self
            .manager
            .subscribe_orchestration(&req.orchestration_id)
            .await
            .map_err(|e| manager_err_to_status(&e))?;

        // Convert broadcast::Receiver into a Stream via async_stream
        let stream = async_stream::stream! {
            loop {
                match broadcast_rx.recv().await {
                    Ok(event) => yield Ok(event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            orchestration_id = %req.orchestration_id,
                            skipped = n,
                            "WatchOrchestration subscriber lagged, skipped events"
                        );
                        // Continue receiving; the subscriber just missed some events
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Channel closed — orchestration finished
                        break;
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    #[instrument(skip(self, request), fields(rpc = "RevokeAutoApprove"))]
    async fn revoke_auto_approve(
        &self,
        request: Request<RevokeAutoApproveRequest>,
    ) -> Result<Response<RevokeAutoApproveResponse>, Status> {
        let req = request.into_inner();

        if req.subagent_id.is_empty() {
            return Err(Status::invalid_argument("subagent_id must not be empty"));
        }

        let revoked = self
            .manager
            .revoke_auto_approve(&req.subagent_id)
            .await
            .map_err(|e| manager_err_to_status(&e))?;

        if revoked {
            info!(subagent_id = %req.subagent_id, "Auto-approve revoked");
        } else {
            warn!(
                subagent_id = %req.subagent_id,
                "Subagent not found for auto-approve revocation"
            );
        }

        // Get current status from DB
        let status = match self.db.get_subagent(&req.subagent_id).await {
            Ok(sa) => sa.status,
            Err(_) => "unknown".to_string(),
        };

        Ok(Response::new(RevokeAutoApproveResponse {
            revoked,
            pending_tool_calls: 0,
            subagent_status: status,
        }))
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::orchestration::pool::SubprocessPool;

    async fn test_service() -> SubagentServiceImpl {
        let db = Database::open_in_memory().await.unwrap();
        db.create_session("parent-1", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();

        let pool = Arc::new(SubprocessPool::new(3));
        let manager = Arc::new(SubagentManager::new(pool, db.clone()));

        SubagentServiceImpl::new(manager, db)
    }

    // jscpd:ignore-start -- validation tests are intentionally repetitive
    #[tokio::test]
    async fn spawn_rejects_empty_parent() {
        let svc = test_service().await;
        let req = Request::new(SpawnSubagentRequest {
            parent_session_id: String::new(),
            prompt: "test".to_string(),
            ..Default::default()
        });
        let err = svc.spawn_subagent(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn spawn_rejects_empty_prompt() {
        let svc = test_service().await;
        let req = Request::new(SpawnSubagentRequest {
            parent_session_id: "parent-1".to_string(),
            prompt: String::new(),
            ..Default::default()
        });
        let err = svc.spawn_subagent(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn spawn_rejects_auto_approve_without_tools() {
        let svc = test_service().await;
        let req = Request::new(SpawnSubagentRequest {
            parent_session_id: "parent-1".to_string(),
            prompt: "test".to_string(),
            auto_approve: true,
            allowed_tools: vec![],
            ..Default::default()
        });
        let err = svc.spawn_subagent(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
    // jscpd:ignore-end

    #[tokio::test]
    async fn list_subagents_rejects_empty_session() {
        let svc = test_service().await;
        let req = Request::new(ListSubagentsRequest {
            parent_session_id: String::new(),
            status_filter: String::new(),
        });
        let err = svc.list_subagents(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn list_subagents_returns_empty_for_new_session() {
        let svc = test_service().await;
        let req = Request::new(ListSubagentsRequest {
            parent_session_id: "parent-1".to_string(),
            status_filter: String::new(),
        });
        let resp = svc.list_subagents(req).await.unwrap();
        assert!(resp.into_inner().subagents.is_empty());
    }

    #[tokio::test]
    async fn cancel_rejects_empty_id() {
        let svc = test_service().await;
        let req = Request::new(CancelSubagentRequest {
            subagent_id: String::new(),
            reason: String::new(),
            force: false,
            cleanup_worktree: false,
        });
        let err = svc.cancel_subagent(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn send_rejects_empty_id() {
        let svc = test_service().await;
        let req = Request::new(SendToSubagentRequest {
            subagent_id: String::new(),
            content: "hello".to_string(),
        });
        let err = svc.send_to_subagent(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn watch_rejects_empty_id() {
        let svc = test_service().await;
        let req = Request::new(WatchSubagentRequest {
            subagent_id: String::new(),
            from_sequence: 0,
        });
        match svc.watch_subagent(req).await {
            Err(err) => assert_eq!(err.code(), tonic::Code::InvalidArgument),
            Ok(_) => panic!("expected InvalidArgument error for empty subagent_id"),
        }
    }

    #[tokio::test]
    async fn revoke_rejects_empty_id() {
        let svc = test_service().await;
        let req = Request::new(RevokeAutoApproveRequest {
            subagent_id: String::new(),
            reason: String::new(),
            terminate_if_pending: false,
        });
        let err = svc.revoke_auto_approve(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    // jscpd:ignore-start -- validation tests are intentionally repetitive
    #[tokio::test]
    async fn create_orchestration_rejects_empty_session() {
        let svc = test_service().await;
        let req = Request::new(CreateOrchestrationRequest {
            parent_session_id: String::new(),
            steps: vec![],
            strategy: 1,
        });
        let err = svc.create_orchestration(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn create_orchestration_rejects_empty_steps() {
        let svc = test_service().await;
        let req = Request::new(CreateOrchestrationRequest {
            parent_session_id: "parent-1".to_string(),
            steps: vec![],
            strategy: 1,
        });
        let err = svc.create_orchestration(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
    // jscpd:ignore-end

    #[tokio::test]
    async fn watch_orchestration_rejects_empty_id() {
        let svc = test_service().await;
        let req = Request::new(WatchOrchestrationRequest {
            orchestration_id: String::new(),
        });
        match svc.watch_orchestration(req).await {
            Err(err) => assert_eq!(err.code(), tonic::Code::InvalidArgument),
            Ok(_) => panic!("expected InvalidArgument error for empty orchestration_id"),
        }
    }

    #[tokio::test]
    async fn watch_orchestration_not_found() {
        let svc = test_service().await;
        let req = Request::new(WatchOrchestrationRequest {
            orchestration_id: "nonexistent".to_string(),
        });
        match svc.watch_orchestration(req).await {
            Err(err) => assert_eq!(err.code(), tonic::Code::NotFound),
            Ok(_) => panic!("expected NotFound error for nonexistent orchestration"),
        }
    }

    #[test]
    fn status_str_to_proto_conversion() {
        assert_eq!(
            status_str_to_proto("pending"),
            i32::from(SubagentStatus::Pending)
        );
        assert_eq!(
            status_str_to_proto("running"),
            i32::from(SubagentStatus::Running)
        );
        assert_eq!(
            status_str_to_proto("completed"),
            i32::from(SubagentStatus::Completed)
        );
        assert_eq!(
            status_str_to_proto("failed"),
            i32::from(SubagentStatus::Failed)
        );
        assert_eq!(
            status_str_to_proto("cancelled"),
            i32::from(SubagentStatus::Cancelled)
        );
        assert_eq!(
            status_str_to_proto("unknown"),
            i32::from(SubagentStatus::Unspecified)
        );
    }

    #[test]
    fn strategy_from_i32_values() {
        assert_eq!(strategy_from_i32(1), OrchestrationStrategy::Parallel);
        assert_eq!(strategy_from_i32(2), OrchestrationStrategy::Sequential);
        assert_eq!(strategy_from_i32(3), OrchestrationStrategy::Dag);
        assert_eq!(strategy_from_i32(99), OrchestrationStrategy::Parallel); // default
    }
}
