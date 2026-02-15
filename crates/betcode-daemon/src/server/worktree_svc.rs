//! `WorktreeService` gRPC implementation.

use tonic::{Request, Response, Status};
use tracing::{debug, info, instrument};

use betcode_proto::v1::{
    CreateWorktreeRequest, GetWorktreeRequest, ListWorktreesRequest, ListWorktreesResponse,
    RemoveWorktreeRequest, RemoveWorktreeResponse, WorktreeDetail,
    worktree_service_server::WorktreeService,
};

use crate::storage::Database;
use crate::worktree::{GitRepo, WorktreeInfo, WorktreeManager};

/// `WorktreeService` implementation backed by `WorktreeManager`.
#[derive(Clone)]
pub struct WorktreeServiceImpl {
    manager: WorktreeManager,
    db: Database,
}

impl WorktreeServiceImpl {
    /// Create a new `WorktreeService`.
    pub const fn new(manager: WorktreeManager, db: Database) -> Self {
        Self { manager, db }
    }
}

/// Convert a `WorktreeInfo` into a proto `WorktreeDetail`.
fn to_detail(info: WorktreeInfo) -> WorktreeDetail {
    WorktreeDetail {
        id: info.worktree.id,
        name: info.worktree.name,
        path: info.worktree.path,
        branch: info.worktree.branch,
        repo_id: info.worktree.repo_id,
        setup_script: info.worktree.setup_script.unwrap_or_default(),
        exists_on_disk: info.exists_on_disk,
        #[allow(clippy::cast_possible_truncation)]
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
    #[instrument(skip(self, request), fields(rpc = "CreateWorktree"))]
    async fn create_worktree(
        &self,
        request: Request<CreateWorktreeRequest>,
    ) -> Result<Response<WorktreeDetail>, Status> {
        let req = request.into_inner();
        debug!(
            name = %req.name,
            repo_id = %req.repo_id,
            branch = %req.branch,
            setup_script = %req.setup_script,
            "CreateWorktree RPC received"
        );

        let start = std::time::Instant::now();

        // Look up the registered repo
        let repo_row = self
            .db
            .get_git_repo(&req.repo_id)
            .await
            .map_err(|e| Status::not_found(format!("Repository not found: {e}")))?;
        let repo = GitRepo::from(repo_row);

        let setup_script = if req.setup_script.is_empty() {
            None
        } else {
            Some(req.setup_script.as_str())
        };

        let wt = self
            .manager
            .create(&req.name, &repo, &req.branch, setup_script)
            .await
            .map_err(|e| {
                debug!(elapsed_ms = start.elapsed().as_millis(), error = %e, "CreateWorktree manager.create failed");
                Status::internal(e.to_string())
            })?;

        debug!(
            id = %wt.id,
            elapsed_ms = start.elapsed().as_millis(),
            "CreateWorktree manager.create succeeded, fetching full info"
        );

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

        debug!(
            id = %wt.id,
            total_elapsed_ms = start.elapsed().as_millis(),
            "CreateWorktree RPC complete"
        );

        Ok(Response::new(to_detail(info)))
    }

    #[instrument(skip(self, request), fields(rpc = "RemoveWorktree"))]
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

    #[instrument(skip(self, request), fields(rpc = "ListWorktrees"))]
    async fn list_worktrees(
        &self,
        request: Request<ListWorktreesRequest>,
    ) -> Result<Response<ListWorktreesResponse>, Status> {
        let req = request.into_inner();

        let repo_id = if req.repo_id.is_empty() {
            None
        } else {
            Some(req.repo_id.as_str())
        };

        let infos = self
            .manager
            .list(repo_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let worktrees = infos.into_iter().map(to_detail).collect();

        Ok(Response::new(ListWorktreesResponse { worktrees }))
    }

    #[instrument(skip(self, request), fields(rpc = "GetWorktree"))]
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
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::storage::Database;
    use crate::worktree::WorktreeManager;

    async fn test_service() -> (WorktreeServiceImpl, tempfile::TempDir) {
        let db = Database::open_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(db.clone(), tmp.path().to_path_buf());
        (WorktreeServiceImpl::new(manager, db), tmp)
    }

    #[tokio::test]
    async fn list_empty() {
        let (svc, _tmp) = test_service().await;
        let resp = svc
            .list_worktrees(Request::new(ListWorktreesRequest {
                repo_id: String::new(),
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().worktrees.is_empty());
    }

    #[tokio::test]
    async fn get_nonexistent_returns_not_found() {
        let (svc, _tmp) = test_service().await;
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
        let (svc, _tmp) = test_service().await;
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
        let (svc, _tmp) = test_service().await;
        // Register a repo first
        svc.db
            .create_git_repo(
                "r1",
                "/tmp/repo",
                &crate::storage::GitRepoParams {
                    name: "repo",
                    worktree_mode: "global",
                    local_subfolder: ".worktree",
                    custom_path: None,
                    setup_script: None,
                    auto_gitignore: true,
                },
            )
            .await
            .unwrap();
        let result = svc
            .create_worktree(Request::new(CreateWorktreeRequest {
                name: "../escape".to_string(),
                repo_id: "r1".to_string(),
                branch: "main".to_string(),
                setup_script: String::new(),
            }))
            .await;
        assert!(result.is_err());
    }
}
