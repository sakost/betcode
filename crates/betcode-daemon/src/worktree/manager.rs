//! Worktree manager: git worktree operations + DB persistence.

use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{info, warn};

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
            "name contains invalid characters: {}",
            name
        )));
    }
    Ok(())
}

/// Validate a git branch name against safe character set.
fn validate_branch(branch: &str) -> Result<(), WorktreeError> {
    validate_name(branch)
        .map_err(|_| WorktreeError::InvalidName(format!("invalid branch name: {}", branch)))
}

/// Manages git worktrees and their lifecycle.
pub struct WorktreeManager {
    db: Database,
}

impl WorktreeManager {
    /// Create a new worktree manager.
    pub fn new(db: Database) -> Self {
        Self { db }
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
    pub async fn create(
        &self,
        name: &str,
        repo_path: &Path,
        branch: &str,
        setup_script: Option<&str>,
    ) -> Result<Worktree, WorktreeError> {
        validate_name(name)?;
        validate_branch(branch)?;

        let worktree_path = repo_path.join(name);

        if worktree_path.exists() {
            return Err(WorktreeError::PathExists(
                worktree_path.display().to_string(),
            ));
        }

        // Run git worktree add
        let output = tokio::process::Command::new("git")
            .args(["worktree", "add", "-b", branch])
            .arg(&worktree_path)
            .current_dir(repo_path)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(WorktreeError::Git(format!(
                "git worktree add failed: {}",
                stderr.trim()
            )));
        }

        info!(
            name,
            path = %worktree_path.display(),
            branch,
            "Created git worktree"
        );

        // Record in database
        let id = uuid::Uuid::new_v4().to_string();
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

        // Run setup script if provided; clean up on failure
        if let Some(script) = setup_script {
            if let Err(e) = self.run_setup_script(&worktree_path, script).await {
                warn!(id = %id, error = %e, "Setup script failed, cleaning up worktree");
                let _ = self.remove(&id).await;
                return Err(e);
            }
        }

        Ok(wt)
    }

    /// Remove a git worktree and its database record.
    pub async fn remove(&self, id: &str) -> Result<bool, WorktreeError> {
        let wt = match self.db.get_worktree(id).await {
            Ok(wt) => wt,
            Err(_) => return Ok(false),
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

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(id, error = %stderr.trim(), "git worktree remove failed, removing DB record anyway");
            } else {
                info!(id, path = %path.display(), "Removed git worktree");
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
    /// Updates last_active timestamp.
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

    async fn test_manager() -> WorktreeManager {
        let db = Database::open_in_memory().await.unwrap();
        WorktreeManager::new(db)
    }

    #[tokio::test]
    async fn manager_creation() {
        let mgr = test_manager().await;
        let list = mgr.list(None).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn list_empty_repo() {
        let mgr = test_manager().await;
        let list = mgr.list(Some("/nonexistent")).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn get_nonexistent_worktree() {
        let mgr = test_manager().await;
        assert!(mgr.get("nope").await.is_err());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_false() {
        let mgr = test_manager().await;
        assert!(!mgr.remove("nope").await.unwrap());
    }

    #[tokio::test]
    async fn worktree_path_nonexistent_errors() {
        let mgr = test_manager().await;
        assert!(mgr.worktree_path("nope").await.is_err());
    }

    #[tokio::test]
    async fn list_with_db_records() {
        let db = Database::open_in_memory().await.unwrap();
        // Insert directly into DB to test list without git
        db.create_worktree("wt-1", "feat-a", "/tmp/wt-1", "feat-a", "/repo", None)
            .await
            .unwrap();
        db.create_worktree("wt-2", "feat-b", "/tmp/wt-2", "feat-b", "/repo", None)
            .await
            .unwrap();

        let mgr = WorktreeManager::new(db);
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

        let mgr = WorktreeManager::new(db);
        let list = mgr.list(None).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_count, 2);
    }

    #[tokio::test]
    async fn remove_cleans_db() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_worktree("wt-1", "feat", "/tmp/nonexistent-wt", "feat", "/repo", None)
            .await
            .unwrap();

        let mgr = WorktreeManager::new(db);
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
}
