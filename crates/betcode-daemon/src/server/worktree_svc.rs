//! WorktreeService gRPC implementation.

use std::path::Path;

use tonic::{Request, Response, Status};
use tracing::info;

use betcode_proto::v1::{
    worktree_service_server::WorktreeService, CreateWorktreeRequest, GetWorktreeRequest,
    ListWorktreesRequest, ListWorktreesResponse, RemoveWorktreeRequest, RemoveWorktreeResponse,
    WorktreeDetail,
};

use crate::worktree::{WorktreeInfo, WorktreeManager};

/// WorktreeService implementation backed by WorktreeManager.
pub struct WorktreeServiceImpl {
    manager: WorktreeManager,
}

impl WorktreeServiceImpl {
    /// Create a new WorktreeService.
    pub fn new(manager: WorktreeManager) -> Self {
        Self { manager }
    }
}

/// Convert a WorktreeInfo into a proto WorktreeDetail.
fn to_detail(info: WorktreeInfo) -> WorktreeDetail {
    WorktreeDetail {
        id: info.worktree.id,
        name: info.worktree.name,
        path: info.worktree.path,
        branch: info.worktree.branch,
        repo_path: info.worktree.repo_path,
        setup_script: info.worktree.setup_script.unwrap_or_default(),
        exists_on_disk: info.exists_on_disk,
        session_count: info.session_count as u32,
        created_at: Some(prost_types::Timestamp {
            seconds: info.worktree.created_at,
            nanos: 0,
        }),
        last_active: Some(prost_types::Timestamp {
            seconds: info.worktree.last_active,
            nanos: 0,
        }),
    }
}

#[tonic::async_trait]
impl WorktreeService for WorktreeServiceImpl {
    async fn create_worktree(
        &self,
        request: Request<CreateWorktreeRequest>,
    ) -> Result<Response<WorktreeDetail>, Status> {
        let req = request.into_inner();

        let setup_script = if req.setup_script.is_empty() {
            None
        } else {
            Some(req.setup_script.as_str())
        };

        let wt = self
            .manager
            .create(
                &req.name,
                Path::new(&req.repo_path),
                &req.branch,
                setup_script,
            )
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        info!(
            id = %wt.id,
            name = %wt.name,
            branch = %wt.branch,
            "Worktree created via gRPC"
        );

        // Fetch full info (includes exists_on_disk, session_count)
        let info = self
            .manager
            .get(&wt.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(to_detail(info)))
    }

    async fn remove_worktree(
        &self,
        request: Request<RemoveWorktreeRequest>,
    ) -> Result<Response<RemoveWorktreeResponse>, Status> {
        let req = request.into_inner();

        let removed = self
            .manager
            .remove(&req.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if removed {
            info!(id = %req.id, "Worktree removed via gRPC");
        }

        Ok(Response::new(RemoveWorktreeResponse { removed }))
    }

    async fn list_worktrees(
        &self,
        request: Request<ListWorktreesRequest>,
    ) -> Result<Response<ListWorktreesResponse>, Status> {
        let req = request.into_inner();

        let repo_path = if req.repo_path.is_empty() {
            None
        } else {
            Some(req.repo_path.as_str())
        };

        let infos = self
            .manager
            .list(repo_path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let worktrees = infos.into_iter().map(to_detail).collect();

        Ok(Response::new(ListWorktreesResponse { worktrees }))
    }

    async fn get_worktree(
        &self,
        request: Request<GetWorktreeRequest>,
    ) -> Result<Response<WorktreeDetail>, Status> {
        let req = request.into_inner();

        let info = self
            .manager
            .get(&req.id)
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;

        Ok(Response::new(to_detail(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;
    use crate::worktree::WorktreeManager;

    async fn test_service() -> WorktreeServiceImpl {
        let db = Database::open_in_memory().await.unwrap();
        let manager = WorktreeManager::new(db);
        WorktreeServiceImpl::new(manager)
    }

    #[tokio::test]
    async fn list_empty() {
        let svc = test_service().await;
        let resp = svc
            .list_worktrees(Request::new(ListWorktreesRequest {
                repo_path: String::new(),
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().worktrees.is_empty());
    }

    #[tokio::test]
    async fn get_nonexistent_returns_not_found() {
        let svc = test_service().await;
        let result = svc
            .get_worktree(Request::new(GetWorktreeRequest {
                id: "nope".to_string(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_false() {
        let svc = test_service().await;
        let resp = svc
            .remove_worktree(Request::new(RemoveWorktreeRequest {
                id: "nope".to_string(),
            }))
            .await
            .unwrap();
        assert!(!resp.into_inner().removed);
    }

    #[tokio::test]
    async fn create_with_invalid_name_returns_error() {
        let svc = test_service().await;
        let result = svc
            .create_worktree(Request::new(CreateWorktreeRequest {
                name: "../escape".to_string(),
                repo_path: "/tmp/repo".to_string(),
                branch: "main".to_string(),
                setup_script: String::new(),
            }))
            .await;
        assert!(result.is_err());
    }
}
