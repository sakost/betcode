//! `GitRepoService` gRPC implementation.

use std::path::Path;

use tonic::{Request, Response, Status};
use tracing::{info, instrument};

use betcode_proto::v1::{
    git_repo_service_server::GitRepoService, GetRepoRequest, GitRepoDetail, ListReposRequest,
    ListReposResponse, RegisterRepoRequest, ScanReposRequest, UnregisterRepoRequest,
    UnregisterRepoResponse, UpdateRepoRequest,
};

use crate::storage::{Database, GitRepoRow};

/// `GitRepoService` implementation backed by `Database`.
#[derive(Clone)]
pub struct GitRepoServiceImpl {
    db: Database,
}

impl GitRepoServiceImpl {
    /// Create a new `GitRepoService`.
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// Convert a `GitRepoRow` into a proto `GitRepoDetail`.
fn to_detail(row: GitRepoRow, worktree_count: u32) -> GitRepoDetail {
    GitRepoDetail {
        id: row.id,
        name: row.name,
        repo_path: row.repo_path,
        worktree_mode: row.worktree_mode,
        local_subfolder: row.local_subfolder,
        custom_path: row.custom_path.unwrap_or_default(),
        setup_script: row.setup_script.unwrap_or_default(),
        auto_gitignore: row.auto_gitignore != 0,
        worktree_count,
        created_at: Some(prost_types::Timestamp {
            seconds: row.created_at,
            nanos: 0,
        }),
        last_active: Some(prost_types::Timestamp {
            seconds: row.last_active,
            nanos: 0,
        }),
    }
}

#[tonic::async_trait]
impl GitRepoService for GitRepoServiceImpl {
    #[instrument(skip(self, request), fields(rpc = "RegisterRepo"))]
    async fn register_repo(
        &self,
        request: Request<RegisterRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        let req = request.into_inner();

        let repo_path = req.repo_path.as_str();
        if repo_path.is_empty() {
            return Err(Status::invalid_argument("repo_path is required"));
        }

        // Default name to last path component
        let name = if req.name.is_empty() {
            Path::new(repo_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        } else {
            req.name
        };

        let worktree_mode = if req.worktree_mode.is_empty() {
            "global"
        } else {
            req.worktree_mode.as_str()
        };

        let local_subfolder = if req.local_subfolder.is_empty() {
            ".worktree"
        } else {
            req.local_subfolder.as_str()
        };

        let custom_path = if req.custom_path.is_empty() {
            None
        } else {
            Some(req.custom_path.as_str())
        };

        let setup_script = if req.setup_script.is_empty() {
            None
        } else {
            Some(req.setup_script.as_str())
        };

        let id = uuid::Uuid::new_v4().to_string();

        let row = self
            .db
            .create_git_repo(
                &id,
                &name,
                repo_path,
                worktree_mode,
                local_subfolder,
                custom_path,
                setup_script,
                req.auto_gitignore,
            )
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        info!(id = %row.id, name = %row.name, "Repository registered via gRPC");

        Ok(Response::new(to_detail(row, 0)))
    }

    #[instrument(skip(self, request), fields(rpc = "UnregisterRepo"))]
    async fn unregister_repo(
        &self,
        request: Request<UnregisterRepoRequest>,
    ) -> Result<Response<UnregisterRepoResponse>, Status> {
        let req = request.into_inner();

        // Count worktrees before removal
        let worktrees = self
            .db
            .list_worktrees(Some(&req.id))
            .await
            .unwrap_or_default();

        #[allow(clippy::cast_possible_truncation)]
        let worktrees_removed = worktrees.len() as u32;

        let removed = self
            .db
            .remove_git_repo(&req.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if removed {
            info!(id = %req.id, "Repository unregistered via gRPC");
        }

        Ok(Response::new(UnregisterRepoResponse {
            removed,
            worktrees_removed: if removed { worktrees_removed } else { 0 },
        }))
    }

    #[instrument(skip(self, _request), fields(rpc = "ListRepos"))]
    async fn list_repos(
        &self,
        _request: Request<ListReposRequest>,
    ) -> Result<Response<ListReposResponse>, Status> {
        let rows = self
            .db
            .list_git_repos()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut repos = Vec::with_capacity(rows.len());
        for row in rows {
            let wt_count = self
                .db
                .list_worktrees(Some(&row.id))
                .await
                .map(|v| v.len())
                .unwrap_or(0);
            #[allow(clippy::cast_possible_truncation)]
            repos.push(to_detail(row, wt_count as u32));
        }

        Ok(Response::new(ListReposResponse { repos }))
    }

    #[instrument(skip(self, request), fields(rpc = "GetRepo"))]
    async fn get_repo(
        &self,
        request: Request<GetRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        let req = request.into_inner();

        let row = self
            .db
            .get_git_repo(&req.id)
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;

        let wt_count = self
            .db
            .list_worktrees(Some(&row.id))
            .await
            .map(|v| v.len())
            .unwrap_or(0);

        #[allow(clippy::cast_possible_truncation)]
        Ok(Response::new(to_detail(row, wt_count as u32)))
    }

    #[instrument(skip(self, request), fields(rpc = "UpdateRepo"))]
    async fn update_repo(
        &self,
        request: Request<UpdateRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        let req = request.into_inner();

        let custom_path = if req.custom_path.is_empty() {
            None
        } else {
            Some(req.custom_path.as_str())
        };

        let setup_script = if req.setup_script.is_empty() {
            None
        } else {
            Some(req.setup_script.as_str())
        };

        let row = self
            .db
            .update_git_repo(
                &req.id,
                &req.name,
                &req.worktree_mode,
                &req.local_subfolder,
                custom_path,
                setup_script,
                req.auto_gitignore,
            )
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;

        info!(id = %row.id, "Repository updated via gRPC");

        let wt_count = self
            .db
            .list_worktrees(Some(&row.id))
            .await
            .map(|v| v.len())
            .unwrap_or(0);

        #[allow(clippy::cast_possible_truncation)]
        Ok(Response::new(to_detail(row, wt_count as u32)))
    }

    #[instrument(skip(self, request), fields(rpc = "ScanRepos"))]
    async fn scan_repos(
        &self,
        request: Request<ScanReposRequest>,
    ) -> Result<Response<ListReposResponse>, Status> {
        let req = request.into_inner();

        if req.scan_path.is_empty() {
            return Err(Status::invalid_argument("scan_path is required"));
        }

        let scan_path = Path::new(&req.scan_path);
        if !scan_path.is_dir() {
            return Err(Status::not_found(format!(
                "Directory not found: {}",
                scan_path.display()
            )));
        }

        let max_depth = if req.max_depth == 0 { 2 } else { req.max_depth };

        let mut repos = Vec::new();
        scan_for_repos(scan_path, max_depth, &mut repos);

        let mut registered = Vec::new();
        for repo_path in repos {
            let path_str = repo_path.to_string_lossy();

            // Skip if already registered
            if self.db.get_git_repo_by_path(&path_str).await.is_ok() {
                continue;
            }

            let name = repo_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let id = uuid::Uuid::new_v4().to_string();
            match self
                .db
                .create_git_repo(&id, &name, &path_str, "global", ".worktree", None, None, true)
                .await
            {
                Ok(row) => {
                    info!(id = %row.id, name = %row.name, "Repository auto-registered from scan");
                    registered.push(to_detail(row, 0));
                }
                Err(e) => {
                    tracing::warn!(path = %path_str, error = %e, "Failed to register scanned repo");
                }
            }
        }

        Ok(Response::new(ListReposResponse { repos: registered }))
    }
}

/// Recursively scan for directories containing a `.git` folder.
fn scan_for_repos(dir: &Path, depth: u32, results: &mut Vec<std::path::PathBuf>) {
    if depth == 0 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
            // Parent is a git repo
            results.push(dir.to_path_buf());
            return; // Don't recurse into git repos
        }
        scan_for_repos(&path, depth - 1, results);
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::storage::Database;

    async fn test_service() -> GitRepoServiceImpl {
        let db = Database::open_in_memory().await.unwrap();
        GitRepoServiceImpl::new(db)
    }

    #[tokio::test]
    async fn register_and_get_repo() {
        let svc = test_service().await;

        let resp = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: "/tmp/my-repo".into(),
                name: "my-repo".into(),
                worktree_mode: "global".into(),
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        let detail = resp.into_inner();
        assert_eq!(detail.name, "my-repo");
        assert_eq!(detail.repo_path, "/tmp/my-repo");
        assert_eq!(detail.worktree_mode, "global");
        assert!(detail.auto_gitignore);

        // Get by ID
        let get_resp = svc
            .get_repo(Request::new(GetRepoRequest {
                id: detail.id.clone(),
            }))
            .await
            .unwrap();
        assert_eq!(get_resp.into_inner().name, "my-repo");
    }

    #[tokio::test]
    async fn register_defaults_name_from_path() {
        let svc = test_service().await;

        let resp = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: "/home/user/projects/cool-project".into(),
                name: String::new(), // should default
                worktree_mode: String::new(),
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        assert_eq!(resp.into_inner().name, "cool-project");
    }

    #[tokio::test]
    async fn register_empty_path_returns_error() {
        let svc = test_service().await;

        let result = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: String::new(),
                name: String::new(),
                worktree_mode: String::new(),
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[tokio::test]
    async fn list_repos_empty() {
        let svc = test_service().await;
        let resp = svc
            .list_repos(Request::new(ListReposRequest {}))
            .await
            .unwrap();
        assert!(resp.into_inner().repos.is_empty());
    }

    #[tokio::test]
    async fn list_repos_returns_registered() {
        let svc = test_service().await;

        svc.register_repo(Request::new(RegisterRepoRequest {
            repo_path: "/tmp/repo-a".into(),
            name: "a".into(),
            worktree_mode: "global".into(),
            local_subfolder: String::new(),
            custom_path: String::new(),
            setup_script: String::new(),
            auto_gitignore: true,
        }))
        .await
        .unwrap();

        svc.register_repo(Request::new(RegisterRepoRequest {
            repo_path: "/tmp/repo-b".into(),
            name: "b".into(),
            worktree_mode: "local".into(),
            local_subfolder: ".wt".into(),
            custom_path: String::new(),
            setup_script: String::new(),
            auto_gitignore: false,
        }))
        .await
        .unwrap();

        let resp = svc
            .list_repos(Request::new(ListReposRequest {}))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().repos.len(), 2);
    }

    #[tokio::test]
    async fn unregister_repo() {
        let svc = test_service().await;

        let reg = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: "/tmp/to-remove".into(),
                name: "to-remove".into(),
                worktree_mode: "global".into(),
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        let resp = svc
            .unregister_repo(Request::new(UnregisterRepoRequest {
                id: reg.into_inner().id,
                remove_worktrees: false,
            }))
            .await
            .unwrap();

        assert!(resp.into_inner().removed);
    }

    #[tokio::test]
    async fn unregister_nonexistent_returns_false() {
        let svc = test_service().await;
        let resp = svc
            .unregister_repo(Request::new(UnregisterRepoRequest {
                id: "nope".into(),
                remove_worktrees: false,
            }))
            .await
            .unwrap();
        assert!(!resp.into_inner().removed);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_not_found() {
        let svc = test_service().await;
        let result = svc
            .get_repo(Request::new(GetRepoRequest {
                id: "nope".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn update_repo() {
        let svc = test_service().await;

        let reg = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: "/tmp/updatable".into(),
                name: "old-name".into(),
                worktree_mode: "global".into(),
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        let id = reg.into_inner().id;

        let resp = svc
            .update_repo(Request::new(UpdateRepoRequest {
                id: id.clone(),
                name: "new-name".into(),
                worktree_mode: "custom".into(),
                local_subfolder: ".worktree".into(),
                custom_path: "/custom/path".into(),
                setup_script: "make build".into(),
                auto_gitignore: false,
            }))
            .await
            .unwrap();

        let detail = resp.into_inner();
        assert_eq!(detail.name, "new-name");
        assert_eq!(detail.worktree_mode, "custom");
        assert_eq!(detail.custom_path, "/custom/path");
        assert!(!detail.auto_gitignore);
    }

    #[tokio::test]
    async fn scan_repos_empty_path_returns_error() {
        let svc = test_service().await;
        let result = svc
            .scan_repos(Request::new(ScanReposRequest {
                scan_path: String::new(),
                max_depth: 0,
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[tokio::test]
    async fn scan_repos_discovers_git_repos() {
        let svc = test_service().await;
        let tmp = tempfile::tempdir().unwrap();

        // Create fake git repos
        let repo_a = tmp.path().join("repo-a");
        std::fs::create_dir_all(repo_a.join(".git")).unwrap();
        let repo_b = tmp.path().join("repo-b");
        std::fs::create_dir_all(repo_b.join(".git")).unwrap();
        // Non-repo directory
        std::fs::create_dir_all(tmp.path().join("not-a-repo")).unwrap();

        let resp = svc
            .scan_repos(Request::new(ScanReposRequest {
                scan_path: tmp.path().to_string_lossy().into(),
                max_depth: 2,
            }))
            .await
            .unwrap();

        let repos = resp.into_inner().repos;
        assert_eq!(repos.len(), 2);
        let names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"repo-a"));
        assert!(names.contains(&"repo-b"));
    }

    #[tokio::test]
    async fn scan_repos_skips_already_registered() {
        let svc = test_service().await;
        let tmp = tempfile::tempdir().unwrap();

        let repo = tmp.path().join("existing");
        std::fs::create_dir_all(repo.join(".git")).unwrap();

        // Pre-register
        svc.register_repo(Request::new(RegisterRepoRequest {
            repo_path: repo.to_string_lossy().into(),
            name: "existing".into(),
            worktree_mode: "global".into(),
            local_subfolder: String::new(),
            custom_path: String::new(),
            setup_script: String::new(),
            auto_gitignore: true,
        }))
        .await
        .unwrap();

        // Scan should skip already-registered
        let resp = svc
            .scan_repos(Request::new(ScanReposRequest {
                scan_path: tmp.path().to_string_lossy().into(),
                max_depth: 2,
            }))
            .await
            .unwrap();

        assert!(resp.into_inner().repos.is_empty());
    }
}
