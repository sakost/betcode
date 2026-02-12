//! Worktree manager: git worktree operations + DB persistence.

use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{debug, info, warn};

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
        repo_path: &Path,
        branch: &str,
        setup_script: Option<&str>,
    ) -> Result<Worktree, WorktreeError> {
        debug!(name, repo_path = %repo_path.display(), branch, ?setup_script, "create: validating inputs");
        validate_name(name)?;
        validate_branch(branch)?;

        if !repo_path.exists() {
            return Err(WorktreeError::NotFound(format!(
                "Repository not found at {} on this machine",
                repo_path.display()
            )));
        }

        // Compute worktree path under dedicated base directory:
        // <worktree_base_dir>/<repo_name>/<worktree_id>/
        let repo_name = repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let id = uuid::Uuid::new_v4().to_string();
        let worktree_dir = self.worktree_base_dir.join(repo_name);
        let worktree_path = worktree_dir.join(&id);
        debug!(worktree_path = %worktree_path.display(), "create: computed worktree path");

        if worktree_path.exists() {
            return Err(WorktreeError::PathExists(
                worktree_path.display().to_string(),
            ));
        }

        // Ensure parent directory exists
        tokio::fs::create_dir_all(&worktree_dir).await?;

        // Run git worktree add
        debug!(
            repo_path = %repo_path.display(),
            worktree_path = %worktree_path.display(),
            branch,
            "create: spawning git worktree add"
        );
        let start = std::time::Instant::now();
        let output = tokio::process::Command::new("git")
            .args(["worktree", "add", "-b", branch])
            .arg(&worktree_path)
            .current_dir(repo_path)
            .output()
            .await?;
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
                &repo_path.to_string_lossy(),
                setup_script,
            )
            .await?;
        debug!(elapsed_ms = db_start.elapsed().as_millis(), "create: database insert completed");

        // Run setup script if provided; clean up on failure
        if let Some(script) = setup_script {
            debug!(script, "create: running setup script");
            let script_start = std::time::Instant::now();
            if let Err(e) = self.run_setup_script(&worktree_path, script).await {
                warn!(id = %id, error = %e, elapsed_ms = script_start.elapsed().as_millis(), "Setup script failed, cleaning up worktree");
                let _ = self.remove(&id).await;
                return Err(e);
            }
            debug!(elapsed_ms = script_start.elapsed().as_millis(), "create: setup script completed");
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
            let output = tokio::process::Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .current_dir(&wt.repo_path)
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
    pub async fn list(&self, repo_path: Option<&str>) -> Result<Vec<WorktreeInfo>, WorktreeError> {
        let worktrees = self.db.list_worktrees(repo_path).await?;

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
        // Insert directly into DB to test list without git
        db.create_worktree("wt-1", "feat-a", "/tmp/wt-1", "feat-a", "/repo", None)
            .await
            .unwrap();
        db.create_worktree("wt-2", "feat-b", "/tmp/wt-2", "feat-b", "/repo", None)
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
        db.create_worktree("wt-1", "feat", "/tmp/wt-1", "feat", "/repo", None)
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
        db.create_worktree("wt-1", "feat", "/tmp/nonexistent-wt", "feat", "/repo", None)
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

        let mgr = WorktreeManager::new(db, wt_base.path().to_path_buf());
        let wt = mgr
            .create("feat", repo_dir.path(), "feat", None)
            .await
            .unwrap();

        // Worktree path should be under worktree_base_dir
        assert!(
            wt.path.starts_with(wt_base.path().to_str().unwrap()),
            "worktree path {} should start with base dir {}",
            wt.path,
            wt_base.path().display()
        );

        // Worktree path should contain the repo directory name
        let repo_name = repo_dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            wt.path.contains(repo_name),
            "worktree path {} should contain repo name {}",
            wt.path,
            repo_name
        );
    }

    #[tokio::test]
    async fn create_nonexistent_repo_returns_not_found() {
        let (mgr, _tmp) = test_manager().await;
        let result = mgr
            .create("feat", Path::new("/nonexistent/repo/path"), "feat", None)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            WorktreeError::NotFound(msg) => {
                assert!(msg.contains("Repository not found"));
                assert!(msg.contains("/nonexistent/repo/path"));
            }
            other => panic!("Expected NotFound, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn create_repo_path_without_filename_uses_fallback() {
        let (mgr, tmp) = test_manager().await;
        // `/` exists but has no file_name() component — should use "unknown" fallback.
        // The git command will fail since `/` is not a git repo, but we can verify
        // the directory was created with the "unknown" fallback name.
        let result = mgr.create("feat", Path::new("/"), "feat", None).await;
        // Git will fail, but the directory should have been created
        assert!(result.is_err());
        assert!(
            tmp.path().join("unknown").exists(),
            "Should have created 'unknown' directory as fallback for repo_path '/'"
        );
    }
}
