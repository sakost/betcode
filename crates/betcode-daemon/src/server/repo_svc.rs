//! `GitRepoService` gRPC implementation.

use std::path::Path;

use tonic::{Request, Response, Status};
use tracing::{info, instrument, warn};

use betcode_proto::v1::{
    BranchInfo, CreateBranchRequest, DeleteBranchRequest, DeleteBranchResponse, GetBranchRequest,
    GetRepoRequest, GitRepoDetail, ListBranchesRequest, ListBranchesResponse, ListReposRequest,
    ListReposResponse, RegisterRepoRequest, ScanReposRequest, UnregisterRepoRequest,
    UnregisterRepoResponse, UpdateRepoRequest, WorktreeMode,
    git_repo_service_server::GitRepoService,
};

use crate::storage::{Database, DatabaseError, GitRepoParams, GitRepoRow};
use crate::worktree::WorktreeManager;

/// Timeout for git subprocess commands.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// `GitRepoService` implementation backed by `Database`.
#[derive(Clone)]
pub struct GitRepoServiceImpl {
    db: Database,
    worktree_manager: WorktreeManager,
}

impl GitRepoServiceImpl {
    /// Create a new `GitRepoService`.
    pub const fn new(db: Database, worktree_manager: WorktreeManager) -> Self {
        Self {
            db,
            worktree_manager,
        }
    }
}

/// Convert a DB `worktree_mode` string to the proto `WorktreeMode` i32 value.
fn worktree_mode_to_proto(s: &str) -> i32 {
    match s {
        "global" => WorktreeMode::Global as i32,
        "local" => WorktreeMode::Local as i32,
        "custom" => WorktreeMode::Custom as i32,
        other => {
            warn!(
                worktree_mode = other,
                "Unknown worktree_mode in database, defaulting to Unspecified"
            );
            WorktreeMode::Unspecified as i32
        }
    }
}

/// Convert a proto `WorktreeMode` i32 value to the DB string representation.
fn worktree_mode_to_str(mode: i32) -> Result<&'static str, Status> {
    match WorktreeMode::try_from(mode) {
        Ok(WorktreeMode::Global | WorktreeMode::Unspecified) => Ok("global"),
        Ok(WorktreeMode::Local) => Ok("local"),
        Ok(WorktreeMode::Custom) => Ok("custom"),
        Err(_) => Err(Status::invalid_argument(format!(
            "Invalid worktree_mode value: {mode}"
        ))),
    }
}

/// Convert a `GitRepoRow` into a proto `GitRepoDetail`.
fn to_detail(row: GitRepoRow, worktree_count: u32) -> GitRepoDetail {
    GitRepoDetail {
        id: row.id,
        name: row.name,
        repo_path: row.repo_path,
        worktree_mode: worktree_mode_to_proto(&row.worktree_mode),
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

/// Validate a branch name: alphanumeric, hyphens, underscores, slashes, dots.
/// Rejects path traversal (`..`), leading dashes, and control characters.
fn validate_branch_name(name: &str) -> Result<(), Status> {
    if name.is_empty() {
        return Err(Status::invalid_argument("branch name is required"));
    }
    if name.starts_with('-') {
        return Err(Status::invalid_argument(
            "branch name cannot start with a dash",
        ));
    }
    if name.contains("..") {
        return Err(Status::invalid_argument("branch name cannot contain '..'"));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(Status::invalid_argument(format!(
            "branch name contains invalid characters: {name}"
        )));
    }
    Ok(())
}

/// Run a git command in the given repo directory and return stdout on success.
async fn run_git(repo_path: &Path, args: &[&str]) -> Result<String, Status> {
    let output = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .output(),
    )
    .await
    .map_err(|_| {
        Status::internal(format!(
            "git command timed out after {}s",
            GIT_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| Status::internal(format!("failed to spawn git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Status::internal(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        )));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| Status::internal(format!("git output is not valid UTF-8: {e}")))
}

/// Separator used in `git for-each-ref --format` output (ASCII Unit Separator).
const REF_FIELD_SEP: char = '\x1f';

/// Pre-built `git for-each-ref --format` template.
fn ref_format_str() -> String {
    let s = REF_FIELD_SEP;
    format!("%(refname:short){s}%(objectname){s}%(subject){s}%(HEAD){s}%(worktreepath)")
}

/// Parse a single line of `git for-each-ref` output into a `BranchInfo`.
///
/// Expected format (fields separated by `REF_FIELD_SEP`):
///   refname:short | objectname | subject | HEAD | worktreepath
fn parse_branch_line(line: &str, is_remote: bool) -> Option<BranchInfo> {
    let fields: Vec<&str> = line.split(REF_FIELD_SEP).collect();
    if fields.len() < 4 {
        return None;
    }
    Some(BranchInfo {
        name: fields[0].to_string(),
        commit_sha: fields[1].to_string(),
        commit_message: fields[2].to_string(),
        is_head: fields[3].trim() == "*",
        has_worktree: fields.get(4).is_some_and(|wt| !wt.is_empty()),
        is_remote,
    })
}

impl GitRepoServiceImpl {
    /// Look up a registered repo by ID and return its path.
    async fn resolve_repo_path(&self, repo_id: &str) -> Result<std::path::PathBuf, Status> {
        if repo_id.is_empty() {
            return Err(Status::invalid_argument("repo_id is required"));
        }
        let row = self.db.get_git_repo(repo_id).await.map_err(|e| match e {
            DatabaseError::NotFound(_) => Status::not_found(format!("repo not found: {repo_id}")),
            _ => Status::internal(e.to_string()),
        })?;
        let path = std::path::PathBuf::from(&row.repo_path);
        if !path.is_dir() {
            return Err(Status::failed_precondition(format!(
                "repo directory does not exist: {}",
                row.repo_path
            )));
        }
        Ok(path)
    }

    /// Fetch `BranchInfo` for a single branch ref via `git for-each-ref`.
    async fn get_branch_info(
        &self,
        repo_path: &Path,
        branch_name: &str,
        is_remote: bool,
    ) -> Result<BranchInfo, Status> {
        let ref_prefix = if is_remote {
            format!("refs/remotes/{branch_name}")
        } else {
            format!("refs/heads/{branch_name}")
        };
        let fmt = ref_format_str();
        let stdout = run_git(repo_path, &["for-each-ref", "--format", &fmt, &ref_prefix]).await?;
        let line = stdout
            .lines()
            .next()
            .ok_or_else(|| Status::not_found(format!("branch not found: {branch_name}")))?;
        parse_branch_line(line, is_remote).ok_or_else(|| {
            Status::internal(format!("failed to parse branch info for {branch_name}"))
        })
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

        // Validate the path is a git repository
        let path = Path::new(repo_path);
        if !path.is_dir() {
            return Err(Status::invalid_argument(format!(
                "Directory not found: {repo_path}"
            )));
        }
        if !path.join(".git").exists() {
            return Err(Status::invalid_argument(format!(
                "Not a git repository (no .git): {repo_path}"
            )));
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

        let worktree_mode = worktree_mode_to_str(req.worktree_mode)?;

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
                repo_path,
                &GitRepoParams {
                    name: &name,
                    worktree_mode,
                    local_subfolder,
                    custom_path,
                    setup_script,
                    auto_gitignore: req.auto_gitignore,
                },
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

        let mut worktrees_removed: u32 = 0;

        if req.remove_worktrees {
            let worktrees = self
                .db
                .list_worktrees(Some(&req.id))
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            for wt in &worktrees {
                match self.worktree_manager.remove(&wt.id).await {
                    Ok(_) => worktrees_removed += 1,
                    Err(e) => {
                        tracing::warn!(id = %wt.id, error = %e, "Failed to remove worktree during repo unregistration");
                    }
                }
            }
        }

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

    #[instrument(skip(self, request), fields(rpc = "ListRepos"))]
    async fn list_repos(
        &self,
        request: Request<ListReposRequest>,
    ) -> Result<Response<ListReposResponse>, Status> {
        let req = request.into_inner();

        let total_count = self
            .db
            .count_git_repos()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let rows = if req.limit > 0 || req.offset > 0 {
            self.db
                .list_git_repos_paginated(req.limit, req.offset)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
        } else {
            self.db
                .list_git_repos()
                .await
                .map_err(|e| Status::internal(e.to_string()))?
        };

        let wt_counts = self
            .db
            .count_worktrees_by_repo()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let repos = rows
            .into_iter()
            .map(|row| {
                let wt_count = wt_counts.get(&row.id).copied().unwrap_or(0);
                to_detail(row, wt_count)
            })
            .collect();

        Ok(Response::new(ListReposResponse { repos, total_count }))
    }

    #[instrument(skip(self, request), fields(rpc = "GetRepo"))]
    async fn get_repo(
        &self,
        request: Request<GetRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        let req = request.into_inner();

        let row = self.db.get_git_repo(&req.id).await.map_err(|e| match e {
            DatabaseError::NotFound(_) => Status::not_found(e.to_string()),
            _ => Status::internal(e.to_string()),
        })?;

        let wt_count = self
            .db
            .count_worktrees_for_repo(&row.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(to_detail(row, wt_count)))
    }

    #[instrument(skip(self, request), fields(rpc = "UpdateRepo"))]
    async fn update_repo(
        &self,
        request: Request<UpdateRepoRequest>,
    ) -> Result<Response<GitRepoDetail>, Status> {
        let req = request.into_inner();

        // Convert proto worktree_mode i32 → &str (only when provided).
        let wt_mode_str: Option<String> = req
            .worktree_mode
            .map(|m| worktree_mode_to_str(m).map(String::from))
            .transpose()?;

        // Map proto optional-string semantics into the storage layer's
        // `Option<Option<&str>>`:
        //   proto None       → None            (don't change)
        //   proto Some("")   → Some(None)      (clear / NULL)
        //   proto Some("v")  → Some(Some("v")) (set)
        let custom_path: Option<Option<&str>> = req
            .custom_path
            .as_deref()
            .map(|s| if s.is_empty() { None } else { Some(s) });
        let setup_script: Option<Option<&str>> = req
            .setup_script
            .as_deref()
            .map(|s| if s.is_empty() { None } else { Some(s) });

        let row = self
            .db
            .update_git_repo_partial(
                &req.id,
                req.name.as_deref(),
                wt_mode_str.as_deref(),
                req.local_subfolder.as_deref(),
                custom_path,
                setup_script,
                req.auto_gitignore,
            )
            .await
            .map_err(|e| match e {
                DatabaseError::NotFound(_) => Status::not_found(e.to_string()),
                _ => Status::internal(e.to_string()),
            })?;

        info!(id = %row.id, "Repository updated via gRPC");

        let wt_count = self
            .db
            .count_worktrees_for_repo(&row.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(to_detail(row, wt_count)))
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

        let scan_path_owned = scan_path.to_path_buf();
        let repos = tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            scan_for_repos(&scan_path_owned, max_depth, &mut results);
            results
        })
        .await
        .map_err(|e| Status::internal(format!("Scan task failed: {e}")))?;

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
                .create_git_repo(
                    &id,
                    &path_str,
                    &GitRepoParams {
                        name: &name,
                        worktree_mode: "global",
                        local_subfolder: ".worktree",
                        custom_path: None,
                        setup_script: None,
                        auto_gitignore: true,
                    },
                )
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

        // total_count = total repos in system after scan
        let total_count = self
            .db
            .count_git_repos()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListReposResponse {
            repos: registered,
            total_count,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "ListBranches"))]
    async fn list_branches(
        &self,
        request: Request<ListBranchesRequest>,
    ) -> Result<Response<ListBranchesResponse>, Status> {
        let req = request.into_inner();
        let repo_path = self.resolve_repo_path(&req.repo_id).await?;

        let fmt = ref_format_str();

        let mut branches = Vec::new();

        // Local branches
        let local_stdout = run_git(
            &repo_path,
            &["for-each-ref", "--format", &fmt, "refs/heads/"],
        )
        .await?;
        for line in local_stdout.lines().filter(|l| !l.is_empty()) {
            if let Some(info) = parse_branch_line(line, false) {
                branches.push(info);
            }
        }

        // Remote branches (optional)
        if req.include_remote {
            let remote_stdout = run_git(
                &repo_path,
                &["for-each-ref", "--format", &fmt, "refs/remotes/"],
            )
            .await?;
            for line in remote_stdout.lines().filter(|l| !l.is_empty()) {
                // Skip symbolic remote HEAD refs (e.g. "origin/HEAD")
                let name = line.split(REF_FIELD_SEP).next().unwrap_or("");
                if name.ends_with("/HEAD") {
                    continue;
                }
                if let Some(info) = parse_branch_line(line, true) {
                    branches.push(info);
                }
            }
        }

        Ok(Response::new(ListBranchesResponse { branches }))
    }

    #[instrument(skip(self, request), fields(rpc = "CreateBranch"))]
    async fn create_branch(
        &self,
        request: Request<CreateBranchRequest>,
    ) -> Result<Response<BranchInfo>, Status> {
        let req = request.into_inner();
        let repo_path = self.resolve_repo_path(&req.repo_id).await?;
        validate_branch_name(&req.name)?;

        let mut args = vec!["branch", req.name.as_str()];
        if !req.start_point.is_empty() {
            // start_point can be a branch name, tag, or commit SHA -- just
            // reject obviously dangerous values (leading dash, path traversal).
            if req.start_point.starts_with('-') {
                return Err(Status::invalid_argument(
                    "start_point cannot start with a dash",
                ));
            }
            if req.start_point.contains("..") {
                return Err(Status::invalid_argument("start_point cannot contain '..'"));
            }
            args.push(req.start_point.as_str());
        }

        run_git(&repo_path, &args).await?;
        info!(name = %req.name, "Branch created via gRPC");

        let info = self.get_branch_info(&repo_path, &req.name, false).await?;
        Ok(Response::new(info))
    }

    #[instrument(skip(self, request), fields(rpc = "DeleteBranch"))]
    async fn delete_branch(
        &self,
        request: Request<DeleteBranchRequest>,
    ) -> Result<Response<DeleteBranchResponse>, Status> {
        let req = request.into_inner();
        let repo_path = self.resolve_repo_path(&req.repo_id).await?;
        validate_branch_name(&req.name)?;

        let flag = if req.force { "-D" } else { "-d" };
        let result = run_git(&repo_path, &["branch", flag, &req.name]).await;

        match result {
            Ok(_) => {
                info!(name = %req.name, force = req.force, "Branch deleted via gRPC");
                Ok(Response::new(DeleteBranchResponse { deleted: true }))
            }
            Err(status) => {
                // If the branch doesn't exist or is not fully merged, git returns an error
                let msg = status.message();
                if msg.contains("not found") || msg.contains("not a valid branch name") {
                    return Err(Status::not_found(format!("branch not found: {}", req.name)));
                }
                Err(status)
            }
        }
    }

    #[instrument(skip(self, request), fields(rpc = "GetBranch"))]
    async fn get_branch(
        &self,
        request: Request<GetBranchRequest>,
    ) -> Result<Response<BranchInfo>, Status> {
        let req = request.into_inner();
        let repo_path = self.resolve_repo_path(&req.repo_id).await?;
        validate_branch_name(&req.name)?;

        let info = self.get_branch_info(&repo_path, &req.name, false).await?;
        Ok(Response::new(info))
    }
}

/// Well-known directories that are never standalone git repositories.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "__pycache__",
    "vendor",
    ".cache",
    "dist",
    "build",
];

/// Recursively scan for directories containing a `.git` folder.
fn scan_for_repos(dir: &Path, depth: u32, results: &mut Vec<std::path::PathBuf>) {
    if depth == 0 {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        // Skip symlinks entirely
        if file_type.is_symlink() {
            continue;
        }

        if !file_type.is_dir() {
            continue;
        }

        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Check for .git to detect repo
        if name == ".git" {
            results.push(dir.to_path_buf());
            return;
        }

        // Skip hidden directories
        if name.starts_with('.') {
            continue;
        }

        // Skip well-known non-repo directories
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }

        scan_for_repos(&entry.path(), depth - 1, results);
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::storage::Database;

    /// Create a test service with a temp dir for fake repos.
    async fn test_service() -> (GitRepoServiceImpl, tempfile::TempDir) {
        let db = Database::open_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let wt_dir = tmp.path().join("worktrees");
        let wm = WorktreeManager::new(db.clone(), wt_dir);
        (GitRepoServiceImpl::new(db, wm), tmp)
    }

    /// Create a fake git repo directory (with .git subdir).
    fn make_fake_repo(parent: &std::path::Path, name: &str) -> String {
        let repo = parent.join(name);
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        repo.to_string_lossy().into()
    }

    #[tokio::test]
    async fn register_and_get_repo() {
        let (svc, tmp) = test_service().await;
        let repo_path = make_fake_repo(tmp.path(), "my-repo");

        let resp = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: repo_path.clone(),
                name: "my-repo".into(),
                worktree_mode: WorktreeMode::Global as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        let detail = resp.into_inner();
        assert_eq!(detail.name, "my-repo");
        assert_eq!(detail.repo_path, repo_path);
        assert_eq!(detail.worktree_mode, WorktreeMode::Global as i32);
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
        let (svc, tmp) = test_service().await;
        let repo_path = make_fake_repo(tmp.path(), "cool-project");

        let resp = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path,
                name: String::new(), // should default
                worktree_mode: WorktreeMode::Unspecified as i32,
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
    async fn register_nonexistent_path_returns_error() {
        let (svc, _tmp) = test_service().await;

        let result = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: "/nonexistent/path".into(),
                name: String::new(),
                worktree_mode: WorktreeMode::Unspecified as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn register_non_git_dir_returns_error() {
        let (svc, tmp) = test_service().await;
        // Directory exists but no .git
        let path = tmp.path().join("not-a-repo");
        std::fs::create_dir_all(&path).unwrap();

        let result = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: path.to_string_lossy().into(),
                name: String::new(),
                worktree_mode: WorktreeMode::Unspecified as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn register_invalid_worktree_mode_returns_error() {
        let (svc, tmp) = test_service().await;
        let repo_path = make_fake_repo(tmp.path(), "repo");

        let result = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path,
                name: "repo".into(),
                worktree_mode: 99, // invalid enum value
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn register_empty_path_returns_error() {
        let (svc, _tmp) = test_service().await;

        let result = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: String::new(),
                name: String::new(),
                worktree_mode: WorktreeMode::Unspecified as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn register_repo_unspecified_mode_defaults_to_global() {
        let (svc, tmp) = test_service().await;
        let repo_path = make_fake_repo(tmp.path(), "unspecified-mode");

        let resp = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path,
                name: "unspecified-mode".into(),
                worktree_mode: WorktreeMode::Unspecified as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        let detail = resp.into_inner();
        // Unspecified (0) is stored as "global" in DB and returned as Global
        assert_eq!(detail.worktree_mode, WorktreeMode::Global as i32);
    }

    #[tokio::test]
    async fn list_repos_empty() {
        let (svc, _tmp) = test_service().await;
        let resp = svc
            .list_repos(Request::new(ListReposRequest {
                limit: 0,
                offset: 0,
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().repos.is_empty());
    }

    #[tokio::test]
    async fn list_repos_returns_registered() {
        let (svc, tmp) = test_service().await;
        let path_a = make_fake_repo(tmp.path(), "repo-a");
        let path_b = make_fake_repo(tmp.path(), "repo-b");

        svc.register_repo(Request::new(RegisterRepoRequest {
            repo_path: path_a,
            name: "a".into(),
            worktree_mode: WorktreeMode::Global as i32,
            local_subfolder: String::new(),
            custom_path: String::new(),
            setup_script: String::new(),
            auto_gitignore: true,
        }))
        .await
        .unwrap();

        svc.register_repo(Request::new(RegisterRepoRequest {
            repo_path: path_b,
            name: "b".into(),
            worktree_mode: WorktreeMode::Local as i32,
            local_subfolder: ".wt".into(),
            custom_path: String::new(),
            setup_script: String::new(),
            auto_gitignore: false,
        }))
        .await
        .unwrap();

        let resp = svc
            .list_repos(Request::new(ListReposRequest {
                limit: 0,
                offset: 0,
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().repos.len(), 2);
    }

    #[tokio::test]
    async fn list_repos_with_pagination() {
        let (svc, tmp) = test_service().await;

        // Register 3 repos
        for i in 0..3 {
            let name = format!("repo-{i}");
            let path = make_fake_repo(tmp.path(), &name);
            svc.register_repo(Request::new(RegisterRepoRequest {
                repo_path: path,
                name: name.clone(),
                worktree_mode: WorktreeMode::Global as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();
        }

        // Get first page (limit 2)
        let resp = svc
            .list_repos(Request::new(ListReposRequest {
                limit: 2,
                offset: 0,
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().repos.len(), 2);

        // Get second page (limit 2, offset 2)
        let resp = svc
            .list_repos(Request::new(ListReposRequest {
                limit: 2,
                offset: 2,
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().repos.len(), 1);

        // Offset past all results
        let resp = svc
            .list_repos(Request::new(ListReposRequest {
                limit: 10,
                offset: 10,
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().repos.is_empty());
    }

    #[tokio::test]
    async fn unregister_repo() {
        let (svc, tmp) = test_service().await;
        let repo_path = make_fake_repo(tmp.path(), "to-remove");

        let reg = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path,
                name: "to-remove".into(),
                worktree_mode: WorktreeMode::Global as i32,
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
        let (svc, _tmp) = test_service().await;
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
    async fn unregister_repo_with_remove_worktrees() {
        let db = Database::open_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = make_fake_repo(tmp.path(), "wt-repo");

        let wt_dir = tmp.path().join("worktrees");
        let wm = WorktreeManager::new(db.clone(), wt_dir);
        let svc = GitRepoServiceImpl::new(db.clone(), wm);

        // Register a repo
        let reg = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: repo_path.clone(),
                name: "wt-repo".into(),
                worktree_mode: WorktreeMode::Global as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();
        let repo_id = reg.into_inner().id;

        // Create worktree DB records (simulating created worktrees)
        db.create_worktree(
            "wt1",
            "feat-a",
            "/tmp/nonexistent-wt-a",
            "feat-a",
            &repo_id,
            None,
        )
        .await
        .unwrap();
        db.create_worktree(
            "wt2",
            "feat-b",
            "/tmp/nonexistent-wt-b",
            "feat-b",
            &repo_id,
            None,
        )
        .await
        .unwrap();

        // Unregister with remove_worktrees=true
        let resp = svc
            .unregister_repo(Request::new(UnregisterRepoRequest {
                id: repo_id.clone(),
                remove_worktrees: true,
            }))
            .await
            .unwrap();

        let inner = resp.into_inner();
        assert!(inner.removed);
        assert_eq!(inner.worktrees_removed, 2);

        // Repo should be gone from DB
        assert!(db.get_git_repo(&repo_id).await.is_err());
        // Worktree DB records should also be gone
        assert!(db.get_worktree("wt1").await.is_err());
        assert!(db.get_worktree("wt2").await.is_err());
    }

    #[tokio::test]
    async fn get_nonexistent_returns_not_found() {
        let (svc, _tmp) = test_service().await;
        let result = svc
            .get_repo(Request::new(GetRepoRequest { id: "nope".into() }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn update_repo() {
        let (svc, tmp) = test_service().await;
        let repo_path = make_fake_repo(tmp.path(), "updatable");

        let reg = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path,
                name: "old-name".into(),
                worktree_mode: WorktreeMode::Global as i32,
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
                name: Some("new-name".into()),
                worktree_mode: Some(WorktreeMode::Custom as i32),
                local_subfolder: Some(".worktree".into()),
                custom_path: Some("/custom/path".into()),
                setup_script: Some("make build".into()),
                auto_gitignore: Some(false),
            }))
            .await
            .unwrap();

        let detail = resp.into_inner();
        assert_eq!(detail.name, "new-name");
        assert_eq!(detail.worktree_mode, WorktreeMode::Custom as i32);
        assert_eq!(detail.custom_path, "/custom/path");
        assert!(!detail.auto_gitignore);
    }

    #[tokio::test]
    async fn update_repo_partial_fields() {
        let (svc, tmp) = test_service().await;
        let repo_path = make_fake_repo(tmp.path(), "partial-update");

        let reg = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path,
                name: "original-name".into(),
                worktree_mode: WorktreeMode::Global as i32,
                local_subfolder: ".worktree".into(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        let id = reg.into_inner().id;

        // Only update name, leave everything else unchanged
        let resp = svc
            .update_repo(Request::new(UpdateRepoRequest {
                id: id.clone(),
                name: Some("updated-name".into()),
                worktree_mode: None,
                local_subfolder: None,
                custom_path: None,
                setup_script: None,
                auto_gitignore: None,
            }))
            .await
            .unwrap();

        let detail = resp.into_inner();
        assert_eq!(detail.name, "updated-name");
        // All other fields should remain at their original values
        assert_eq!(detail.worktree_mode, WorktreeMode::Global as i32);
        assert_eq!(detail.local_subfolder, ".worktree");
        assert!(detail.auto_gitignore);
    }

    #[tokio::test]
    async fn scan_repos_empty_path_returns_error() {
        let (svc, _tmp) = test_service().await;
        let result = svc
            .scan_repos(Request::new(ScanReposRequest {
                scan_path: String::new(),
                max_depth: 0,
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn scan_repos_discovers_git_repos() {
        let (svc, _tmp) = test_service().await;
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
        let (svc, _tmp) = test_service().await;
        let tmp = tempfile::tempdir().unwrap();

        let repo = tmp.path().join("existing");
        std::fs::create_dir_all(repo.join(".git")).unwrap();

        // Pre-register
        svc.register_repo(Request::new(RegisterRepoRequest {
            repo_path: repo.to_string_lossy().into(),
            name: "existing".into(),
            worktree_mode: WorktreeMode::Global as i32,
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

    #[tokio::test]
    async fn scan_repos_skips_hidden_directories() {
        let (svc, _tmp) = test_service().await;
        let tmp = tempfile::tempdir().unwrap();

        // Create a visible repo
        let visible = tmp.path().join("visible-repo");
        std::fs::create_dir_all(visible.join(".git")).unwrap();

        // Create a hidden directory with a .git inside (should be skipped)
        let hidden = tmp.path().join(".hidden-repo");
        std::fs::create_dir_all(hidden.join(".git")).unwrap();

        let resp = svc
            .scan_repos(Request::new(ScanReposRequest {
                scan_path: tmp.path().to_string_lossy().into(),
                max_depth: 2,
            }))
            .await
            .unwrap();

        let repos = resp.into_inner().repos;
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "visible-repo");
    }

    #[tokio::test]
    async fn scan_repos_skips_well_known_dirs() {
        let (svc, _tmp) = test_service().await;
        let tmp = tempfile::tempdir().unwrap();

        // Create a real repo
        let real = tmp.path().join("real-repo");
        std::fs::create_dir_all(real.join(".git")).unwrap();

        // Create a node_modules dir with a .git inside (should be skipped)
        let nm = tmp.path().join("node_modules");
        std::fs::create_dir_all(nm.join("some-package").join(".git")).unwrap();

        // Create a target dir with a .git inside (should be skipped)
        let target = tmp.path().join("target");
        std::fs::create_dir_all(target.join("debug").join(".git")).unwrap();

        let resp = svc
            .scan_repos(Request::new(ScanReposRequest {
                scan_path: tmp.path().to_string_lossy().into(),
                max_depth: 3,
            }))
            .await
            .unwrap();

        let repos = resp.into_inner().repos;
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "real-repo");
    }

    // =========================================================================
    // Branch CRUD tests
    // =========================================================================

    /// Initialize a real git repo with an initial commit, register it, and
    /// return the service + repo_id + tempdir.
    async fn test_service_with_git_repo() -> (GitRepoServiceImpl, String, tempfile::TempDir) {
        let (svc, tmp) = test_service().await;

        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        // git init + initial commit (clear leaked env vars from pre-commit hooks)
        let init = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&repo_dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(init.status.success(), "git init failed");

        // Configure git user for commits
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&repo_dir)
            .env_remove("GIT_DIR")
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&repo_dir)
            .env_remove("GIT_DIR")
            .output()
            .unwrap();

        let commit = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial commit"])
            .current_dir(&repo_dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(commit.status.success(), "git commit failed");

        let resp = svc
            .register_repo(Request::new(RegisterRepoRequest {
                repo_path: repo_dir.to_string_lossy().into(),
                name: "repo".into(),
                worktree_mode: WorktreeMode::Global as i32,
                local_subfolder: String::new(),
                custom_path: String::new(),
                setup_script: String::new(),
                auto_gitignore: true,
            }))
            .await
            .unwrap();

        let repo_id = resp.into_inner().id;
        (svc, repo_id, tmp)
    }

    #[tokio::test]
    async fn list_branches_returns_initial_branch() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        let resp = svc
            .list_branches(Request::new(ListBranchesRequest {
                repo_id,
                include_remote: false,
            }))
            .await
            .unwrap();

        let branches = resp.into_inner().branches;
        assert_eq!(branches.len(), 1);
        // The initial branch is HEAD
        assert!(branches[0].is_head);
        assert!(!branches[0].commit_sha.is_empty());
        assert_eq!(branches[0].commit_message, "initial commit");
        assert!(!branches[0].is_remote);
    }

    #[tokio::test]
    async fn list_branches_empty_repo_id_returns_error() {
        let (svc, _tmp) = test_service().await;

        let result = svc
            .list_branches(Request::new(ListBranchesRequest {
                repo_id: String::new(),
                include_remote: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn list_branches_unknown_repo_returns_not_found() {
        let (svc, _tmp) = test_service().await;

        let result = svc
            .list_branches(Request::new(ListBranchesRequest {
                repo_id: "nonexistent".into(),
                include_remote: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn create_and_get_branch() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        let resp = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id: repo_id.clone(),
                name: "feat/login".into(),
                start_point: String::new(),
            }))
            .await
            .unwrap();

        let info = resp.into_inner();
        assert_eq!(info.name, "feat/login");
        assert!(!info.commit_sha.is_empty());
        assert!(!info.is_head); // new branch is not checked out
        assert!(!info.is_remote);

        // Get the branch
        let get_resp = svc
            .get_branch(Request::new(GetBranchRequest {
                repo_id,
                name: "feat/login".into(),
            }))
            .await
            .unwrap();

        let got = get_resp.into_inner();
        assert_eq!(got.name, "feat/login");
        assert_eq!(got.commit_sha, info.commit_sha);
    }

    #[tokio::test]
    async fn create_branch_with_start_point() {
        let (svc, repo_id, tmp) = test_service_with_git_repo().await;

        // Create a second commit so start_point differs from HEAD
        let repo_dir = tmp.path().join("repo");
        let commit2 = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "second commit"])
            .current_dir(&repo_dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(commit2.status.success());

        // Get HEAD SHA (second commit)
        let head_sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD~1"])
            .current_dir(&repo_dir)
            .env_remove("GIT_DIR")
            .output()
            .unwrap();
        let first_commit_sha = String::from_utf8(head_sha.stdout).unwrap();
        let first_commit_sha = first_commit_sha.trim();

        // Create branch from first commit
        let resp = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id,
                name: "from_first".into(),
                start_point: first_commit_sha.to_string(),
            }))
            .await
            .unwrap();

        let info = resp.into_inner();
        assert_eq!(info.name, "from_first");
        assert_eq!(info.commit_sha, first_commit_sha);
        assert_eq!(info.commit_message, "initial commit");
    }

    #[tokio::test]
    async fn create_branch_empty_name_returns_error() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        let result = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id,
                name: String::new(),
                start_point: String::new(),
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn create_branch_invalid_name_returns_error() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        // Leading dash
        let result = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id: repo_id.clone(),
                name: "-bad".into(),
                start_point: String::new(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);

        // Path traversal
        let result = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id: repo_id.clone(),
                name: "../etc/passwd".into(),
                start_point: String::new(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);

        // Special characters
        let result = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id,
                name: "bad name".into(),
                start_point: String::new(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn create_branch_duplicate_returns_error() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        svc.create_branch(Request::new(CreateBranchRequest {
            repo_id: repo_id.clone(),
            name: "dup".into(),
            start_point: String::new(),
        }))
        .await
        .unwrap();

        let result = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id,
                name: "dup".into(),
                start_point: String::new(),
            }))
            .await;

        assert!(result.is_err());
        // git returns non-zero for duplicate branch
        assert_eq!(result.unwrap_err().code(), tonic::Code::Internal);
    }

    #[tokio::test]
    async fn delete_branch() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        // Create a branch to delete
        svc.create_branch(Request::new(CreateBranchRequest {
            repo_id: repo_id.clone(),
            name: "to_delete".into(),
            start_point: String::new(),
        }))
        .await
        .unwrap();

        let resp = svc
            .delete_branch(Request::new(DeleteBranchRequest {
                repo_id: repo_id.clone(),
                name: "to_delete".into(),
                force: false,
            }))
            .await
            .unwrap();

        assert!(resp.into_inner().deleted);

        // Branch should no longer exist
        let result = svc
            .get_branch(Request::new(GetBranchRequest {
                repo_id,
                name: "to_delete".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn delete_branch_invalid_name_returns_error() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        let result = svc
            .delete_branch(Request::new(DeleteBranchRequest {
                repo_id,
                name: String::new(),
                force: false,
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn get_branch_nonexistent_returns_not_found() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        let result = svc
            .get_branch(Request::new(GetBranchRequest {
                repo_id,
                name: "no_such_branch".into(),
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn list_branches_after_create() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        svc.create_branch(Request::new(CreateBranchRequest {
            repo_id: repo_id.clone(),
            name: "feat_a".into(),
            start_point: String::new(),
        }))
        .await
        .unwrap();

        svc.create_branch(Request::new(CreateBranchRequest {
            repo_id: repo_id.clone(),
            name: "feat_b".into(),
            start_point: String::new(),
        }))
        .await
        .unwrap();

        let resp = svc
            .list_branches(Request::new(ListBranchesRequest {
                repo_id,
                include_remote: false,
            }))
            .await
            .unwrap();

        let branches = resp.into_inner().branches;
        // Initial branch + feat_a + feat_b = 3
        assert_eq!(branches.len(), 3);
        let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"feat_a"));
        assert!(names.contains(&"feat_b"));

        // Exactly one should be HEAD
        let head_count = branches.iter().filter(|b| b.is_head).count();
        assert_eq!(head_count, 1);
    }

    #[tokio::test]
    async fn get_branch_invalid_name_returns_error() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        let result = svc
            .get_branch(Request::new(GetBranchRequest {
                repo_id,
                name: "-invalid".into(),
            }))
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn create_branch_bad_start_point_returns_error() {
        let (svc, repo_id, _tmp) = test_service_with_git_repo().await;

        // Leading dash in start_point
        let result = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id: repo_id.clone(),
                name: "ok_name".into(),
                start_point: "-bad".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);

        // Path traversal in start_point
        let result = svc
            .create_branch(Request::new(CreateBranchRequest {
                repo_id,
                name: "ok_name2".into(),
                start_point: "../../etc".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }
}
