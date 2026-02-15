//! Worktree manager: git worktree operations + DB persistence.

use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{debug, info, warn};

use super::repo::{GitRepo, WorktreeMode};
use crate::storage::{Database, DatabaseError, Worktree};

/// Errors from worktree operations.
#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("Git command failed: {0}")]
    Git(String),

    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("Worktree not found: {0}")]
    NotFound(String),

    #[error("Worktree path already exists: {0}")]
    PathExists(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Setup script failed: {0}")]
    SetupFailed(String),

    #[error("Invalid name: {0}")]
    InvalidName(String),
}

/// Summary info about a worktree, combining DB record with on-disk status.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Database record.
    pub worktree: Worktree,
    /// Whether the worktree directory exists on disk.
    pub exists_on_disk: bool,
    /// Number of sessions bound to this worktree.
    pub session_count: usize,
}

/// Validate a worktree/branch name: alphanumeric, hyphens, underscores, slashes, dots.
/// Rejects path traversal (`..`), leading dashes, and control characters.
fn validate_name(name: &str) -> Result<(), WorktreeError> {
    if name.is_empty() {
        return Err(WorktreeError::InvalidName("name cannot be empty".into()));
    }
    if name.starts_with('-') {
        return Err(WorktreeError::InvalidName(
            "name cannot start with a dash".into(),
        ));
    }
    if name.contains("..") {
        return Err(WorktreeError::InvalidName(
            "name cannot contain '..'".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(WorktreeError::InvalidName(format!(
            "name contains invalid characters: {name}"
        )));
    }
    Ok(())
}

/// Validate a git branch name against safe character set.
fn validate_branch(branch: &str) -> Result<(), WorktreeError> {
    validate_name(branch)
        .map_err(|_| WorktreeError::InvalidName(format!("invalid branch name: {branch}")))
}

/// Manages git worktrees and their lifecycle.
#[derive(Clone)]
pub struct WorktreeManager {
    db: Database,
    worktree_base_dir: PathBuf,
}

impl WorktreeManager {
    /// Create a new worktree manager.
    pub const fn new(db: Database, worktree_base_dir: PathBuf) -> Self {
        Self {
            db,
            worktree_base_dir,
        }
    }

    /// Create a new git worktree and record it in the database.
    ///
    /// Runs `git worktree add <path> -b <branch>` in the repo directory,
    /// then optionally runs a setup script (e.g. `npm install`).
    ///
    /// # Safety
    /// The `setup_script` parameter is executed as a shell command.
    /// Callers must ensure this value comes from a trusted source
    /// (e.g. project configuration, not direct user input).
    #[allow(clippy::too_many_lines)]
    pub async fn create(
        &self,
        name: &str,
        repo: &GitRepo,
        branch: &str,
        setup_script: Option<&str>,
    ) -> Result<Worktree, WorktreeError> {
        debug!(name, repo_path = %repo.repo_path.display(), branch, ?setup_script, "create: validating inputs");
        validate_name(name)?;
        validate_branch(branch)?;

        if !repo.repo_path.exists() {
            return Err(WorktreeError::NotFound(format!(
                "Repository not found at {} on this machine",
                repo.repo_path.display()
            )));
        }

        // Compute worktree path using the GitRepo's worktree mode
        let worktree_dir = repo.worktree_base_dir(&self.worktree_base_dir);
        let id = uuid::Uuid::new_v4().to_string();
        let worktree_path = worktree_dir.join(&id);
        debug!(worktree_path = %worktree_path.display(), "create: computed worktree path");

        if worktree_path.exists() {
            return Err(WorktreeError::PathExists(
                worktree_path.display().to_string(),
            ));
        }

        // Ensure parent directory exists
        tokio::fs::create_dir_all(&worktree_dir).await?;

        // Auto-gitignore for local mode
        if matches!(repo.worktree_mode, WorktreeMode::Local) && repo.auto_gitignore {
            let gitignore_path = repo.repo_path.join(".gitignore");
            let subfolder_name = repo.local_subfolder.to_string_lossy();
            let (needs_add, is_new) = if gitignore_path.exists() {
                let content = tokio::fs::read_to_string(&gitignore_path).await?;
                let found = content
                    .lines()
                    .any(|line| line.trim() == subfolder_name.as_ref());
                (!found, false)
            } else {
                (true, true)
            };
            if needs_add {
                use tokio::io::AsyncWriteExt;
                let mut file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .truncate(false)
                    .open(&gitignore_path)
                    .await?;
                let entry = if is_new {
                    format!("{subfolder_name}\n")
                } else {
                    format!("\n{subfolder_name}\n")
                };
                file.write_all(entry.as_bytes()).await?;
                info!(path = %gitignore_path.display(), subfolder = %subfolder_name, "Added worktree subfolder to .gitignore");
            }
        }

        // Run git worktree add
        debug!(
            repo_path = %repo.repo_path.display(),
            worktree_path = %worktree_path.display(),
            branch,
            "create: spawning git worktree add"
        );
        let start = std::time::Instant::now();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            tokio::process::Command::new("git")
                .args(["worktree", "add", "-b", branch])
                .arg(&worktree_path)
                .current_dir(&repo.repo_path)
                .env_remove("GIT_DIR")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_WORK_TREE")
                .output(),
        )
        .await
        .map_err(|_| {
            WorktreeError::Git(format!(
                "git worktree add timed out after 120s for repo {}",
                repo.repo_path.display()
            ))
        })??;
        let elapsed = start.elapsed();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!(
                elapsed_ms = elapsed.as_millis(),
                status = %output.status,
                stderr = %stderr.trim(),
                "create: git worktree add failed"
            );
            return Err(WorktreeError::Git(format!(
                "git worktree add failed: {}",
                stderr.trim()
            )));
        }

        debug!(
            elapsed_ms = elapsed.as_millis(),
            "create: git worktree add completed"
        );

        // Use repo's default setup script if none provided explicitly
        let effective_script = setup_script.or(repo.setup_script.as_deref());

        info!(
            name,
            path = %worktree_path.display(),
            branch,
            "Created git worktree"
        );
        debug!(id = %id, "create: inserting into database");
        let db_start = std::time::Instant::now();
        let wt = self
            .db
            .create_worktree(
                &id,
                name,
                &worktree_path.to_string_lossy(),
                branch,
                &repo.id,
                effective_script,
            )
            .await?;
        debug!(
            elapsed_ms = db_start.elapsed().as_millis(),
            "create: database insert completed"
        );

        // Run setup script if provided; clean up on failure
        if let Some(script) = effective_script {
            debug!(script, "create: running setup script");
            let script_start = std::time::Instant::now();
            if let Err(e) = self.run_setup_script(&worktree_path, script).await {
                warn!(id = %id, error = %e, elapsed_ms = script_start.elapsed().as_millis(), "Setup script failed, cleaning up worktree");
                let _ = self.remove(&id).await;
                return Err(e);
            }
            debug!(
                elapsed_ms = script_start.elapsed().as_millis(),
                "create: setup script completed"
            );
        }

        Ok(wt)
    }

    /// Remove a git worktree and its database record.
    pub async fn remove(&self, id: &str) -> Result<bool, WorktreeError> {
        let Ok(wt) = self.db.get_worktree(id).await else {
            return Ok(false);
        };

        let path = Path::new(&wt.path);

        // Remove via git if the path exists
        if path.exists() {
            // Look up the repo to get the actual repo path for git commands
            let repo_path = self
                .db
                .get_git_repo(&wt.repo_id)
                .await
                .ok()
                .map(|r| PathBuf::from(r.repo_path));

            let current_dir = repo_path.as_deref().unwrap_or(path);

            let output = tokio::process::Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .current_dir(current_dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_WORK_TREE")
                .output()
                .await?;

            if output.status.success() {
                info!(id, path = %path.display(), "Removed git worktree");
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(id, error = %stderr.trim(), "git worktree remove failed, removing DB record anyway");
            }
        }

        // Remove from DB (also clears session bindings)
        self.db.remove_worktree(id).await?;

        Ok(true)
    }

    /// List worktrees with on-disk status and session counts.
    pub async fn list(&self, repo_id: Option<&str>) -> Result<Vec<WorktreeInfo>, WorktreeError> {
        let worktrees = self.db.list_worktrees(repo_id).await?;

        let mut infos = Vec::with_capacity(worktrees.len());
        for wt in worktrees {
            let exists_on_disk = Path::new(&wt.path).exists();
            let sessions = self.db.get_worktree_sessions(&wt.id).await?;

            infos.push(WorktreeInfo {
                worktree: wt,
                exists_on_disk,
                session_count: sessions.len(),
            });
        }

        Ok(infos)
    }

    /// Get a single worktree with status info.
    pub async fn get(&self, id: &str) -> Result<WorktreeInfo, WorktreeError> {
        let wt = self.db.get_worktree(id).await?;
        let exists_on_disk = Path::new(&wt.path).exists();
        let sessions = self.db.get_worktree_sessions(&wt.id).await?;

        Ok(WorktreeInfo {
            worktree: wt,
            exists_on_disk,
            session_count: sessions.len(),
        })
    }

    /// Get the worktree path for starting a session.
    /// Updates `last_active` timestamp.
    pub async fn worktree_path(&self, id: &str) -> Result<PathBuf, WorktreeError> {
        let wt = self.db.get_worktree(id).await?;
        let path = PathBuf::from(&wt.path);

        if !path.exists() {
            return Err(WorktreeError::NotFound(format!(
                "Worktree {} path does not exist on disk: {}",
                id,
                path.display()
            )));
        }

        self.db.touch_worktree(id).await?;
        Ok(path)
    }

    /// Run a setup script in a worktree directory.
    async fn run_setup_script(&self, path: &Path, script: &str) -> Result<(), WorktreeError> {
        info!(path = %path.display(), script, "Running worktree setup script");

        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let flag = if cfg!(windows) { "/C" } else { "-c" };

        let output = tokio::process::Command::new(shell)
            .args([flag, script])
            .current_dir(path)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(WorktreeError::SetupFailed(format!(
                "Setup script '{}' failed: {}",
                script,
                stderr.trim()
            )));
        }

        info!(path = %path.display(), "Setup script completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_manager() -> (WorktreeManager, tempfile::TempDir) {
        let db = Database::open_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mgr = WorktreeManager::new(db, tmp.path().to_path_buf());
        (mgr, tmp)
    }

    /// Register a standard test git repo in the database with common defaults.
    async fn register_test_repo(db: &Database, id: &str, name: &str, repo_path: &str) {
        db.create_git_repo(
            id,
            repo_path,
            &crate::storage::GitRepoParams {
                name,
                worktree_mode: "global",
                local_subfolder: ".worktree",
                custom_path: None,
                setup_script: None,
                auto_gitignore: true,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn manager_creation() {
        let (mgr, _tmp) = test_manager().await;
        let list = mgr.list(None).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn list_empty_repo() {
        let (mgr, _tmp) = test_manager().await;
        let list = mgr.list(Some("/nonexistent")).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn get_nonexistent_worktree() {
        let (mgr, _tmp) = test_manager().await;
        assert!(mgr.get("nope").await.is_err());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_false() {
        let (mgr, _tmp) = test_manager().await;
        assert!(!mgr.remove("nope").await.unwrap());
    }

    #[tokio::test]
    async fn worktree_path_nonexistent_errors() {
        let (mgr, _tmp) = test_manager().await;
        assert!(mgr.worktree_path("nope").await.is_err());
    }

    #[tokio::test]
    async fn list_with_db_records() {
        let db = Database::open_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        register_test_repo(&db, "r1", "repo", "/repo").await;
        // Insert directly into DB to test list without git
        db.create_worktree("wt-1", "feat-a", "/tmp/wt-1", "feat-a", "r1", None)
            .await
            .unwrap();
        db.create_worktree("wt-2", "feat-b", "/tmp/wt-2", "feat-b", "r1", None)
            .await
            .unwrap();

        let mgr = WorktreeManager::new(db, tmp.path().to_path_buf());
        let list = mgr.list(None).await.unwrap();
        assert_eq!(list.len(), 2);

        // Paths don't exist on disk
        for info in &list {
            assert!(!info.exists_on_disk);
            assert_eq!(info.session_count, 0);
        }
    }

    #[tokio::test]
    async fn list_with_session_counts() {
        let db = Database::open_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        register_test_repo(&db, "r1", "repo", "/repo").await;
        db.create_worktree("wt-1", "feat", "/tmp/wt-1", "feat", "r1", None)
            .await
            .unwrap();
        db.create_session("s1", "claude-sonnet-4", "/tmp/wt-1")
            .await
            .unwrap();
        db.create_session("s2", "claude-sonnet-4", "/tmp/wt-1")
            .await
            .unwrap();
        db.bind_session_to_worktree("s1", "wt-1").await.unwrap();
        db.bind_session_to_worktree("s2", "wt-1").await.unwrap();

        let mgr = WorktreeManager::new(db, tmp.path().to_path_buf());
        let list = mgr.list(None).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_count, 2);
    }

    #[tokio::test]
    async fn remove_cleans_db() {
        let db = Database::open_in_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        register_test_repo(&db, "r1", "repo", "/repo").await;
        db.create_worktree("wt-1", "feat", "/tmp/nonexistent-wt", "feat", "r1", None)
            .await
            .unwrap();

        let mgr = WorktreeManager::new(db, tmp.path().to_path_buf());
        // Path doesn't exist, git will fail but DB should still be cleaned
        assert!(mgr.remove("wt-1").await.unwrap());
        assert!(mgr.get("wt-1").await.is_err());
    }

    #[test]
    fn validate_name_accepts_valid() {
        assert!(validate_name("feat-login").is_ok());
        assert!(validate_name("feature/auth").is_ok());
        assert!(validate_name("v1.2.3").is_ok());
        assert!(validate_name("my_worktree").is_ok());
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn validate_name_rejects_path_traversal() {
        assert!(validate_name("../etc/passwd").is_err());
        assert!(validate_name("foo/../bar").is_err());
    }

    #[test]
    fn validate_name_rejects_leading_dash() {
        assert!(validate_name("-flag").is_err());
    }

    #[test]
    fn validate_name_rejects_special_chars() {
        assert!(validate_name("foo bar").is_err());
        assert!(validate_name("foo;bar").is_err());
        assert!(validate_name("foo`cmd`").is_err());
    }

    #[test]
    fn validate_branch_delegates() {
        assert!(validate_branch("feat/login").is_ok());
        assert!(validate_branch("").is_err());
        assert!(validate_branch("..").is_err());
    }

    fn make_test_repo(repo_path: PathBuf) -> GitRepo {
        GitRepo {
            id: "r1".into(),
            name: "testrepo".into(),
            repo_path,
            worktree_mode: WorktreeMode::Global,
            local_subfolder: PathBuf::from(".worktree"),
            setup_script: None,
            auto_gitignore: true,
            created_at: 0,
            last_active: 0,
        }
    }

    #[tokio::test]
    async fn create_uses_dedicated_dir() {
        let db = Database::open_in_memory().await.unwrap();
        let wt_base = tempfile::tempdir().unwrap();
        let repo_dir = tempfile::tempdir().unwrap();

        // Initialize a git repo in the temp directory
        let status = std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo_dir.path())
            .output()
            .unwrap();
        assert!(status.status.success(), "git init failed");

        // Create an initial commit so we can branch
        let status = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo_dir.path())
            .output()
            .unwrap();
        assert!(status.status.success(), "git commit failed");

        // Register the repo in DB (needed for FK)
        db.create_git_repo(
            "r1",
            &repo_dir.path().to_string_lossy(),
            &crate::storage::GitRepoParams {
                name: "testrepo",
                worktree_mode: "global",
                local_subfolder: ".worktree",
                custom_path: None,
                setup_script: None,
                auto_gitignore: true,
            },
        )
        .await
        .unwrap();

        let repo = make_test_repo(repo_dir.path().to_path_buf());
        let mgr = WorktreeManager::new(db, wt_base.path().to_path_buf());
        let wt = mgr.create("feat", &repo, "feat", None).await.unwrap();

        // Worktree path should be under worktree_base_dir
        assert!(
            wt.path.starts_with(wt_base.path().to_str().unwrap()),
            "worktree path {} should start with base dir {}",
            wt.path,
            wt_base.path().display()
        );

        // Should have repo_id set
        assert_eq!(wt.repo_id, "r1");
    }

    #[tokio::test]
    #[allow(clippy::panic)]
    async fn create_nonexistent_repo_returns_not_found() {
        let (mgr, _tmp) = test_manager().await;
        let repo = make_test_repo(PathBuf::from("/nonexistent/repo/path"));
        let result = mgr.create("feat", &repo, "feat", None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            WorktreeError::NotFound(msg) => {
                assert!(msg.contains("Repository not found"));
                assert!(msg.contains("/nonexistent/repo/path"));
            }
            other => panic!("Expected NotFound, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_repo_path_without_filename_uses_fallback() {
        let (mgr, tmp) = test_manager().await;
        // `/` exists but has no file_name() component — should use "unknown" fallback.
        // The git command will fail since `/` is not a git repo, but we can verify
        // the directory was created with the "unknown" fallback name.
        let repo = make_test_repo(PathBuf::from("/"));
        let result = mgr.create("feat", &repo, "feat", None).await;
        // Git will fail, but the directory should have been created
        assert!(result.is_err());
        assert!(
            tmp.path().join("unknown").exists(),
            "Should have created 'unknown' directory as fallback for repo_path '/'"
        );
    }
}
