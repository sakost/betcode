//! Database queries for `git_repos` table.

use betcode_core::db::unix_timestamp;

use super::db::{Database, DatabaseError};
use super::models::GitRepoRow;

impl Database {
    // =========================================================================
    // GitRepo queries
    // =========================================================================

    /// Create a new git repo record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_git_repo(
        &self,
        id: &str,
        name: &str,
        repo_path: &str,
        worktree_mode: &str,
        local_subfolder: &str,
        custom_path: Option<&str>,
        setup_script: Option<&str>,
        auto_gitignore: bool,
    ) -> Result<GitRepoRow, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO git_repos (id, name, repo_path, worktree_mode, local_subfolder, custom_path, setup_script, auto_gitignore, created_at, last_active) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(repo_path)
        .bind(worktree_mode)
        .bind(local_subfolder)
        .bind(custom_path)
        .bind(setup_script)
        .bind(i64::from(auto_gitignore))
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_git_repo(id).await
    }

    /// Get a git repo by ID.
    pub async fn get_git_repo(&self, id: &str) -> Result<GitRepoRow, DatabaseError> {
        sqlx::query_as::<_, GitRepoRow>("SELECT * FROM git_repos WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("GitRepo {id}")))
    }

    /// Get a git repo by repo_path.
    pub async fn get_git_repo_by_path(&self, repo_path: &str) -> Result<GitRepoRow, DatabaseError> {
        sqlx::query_as::<_, GitRepoRow>("SELECT * FROM git_repos WHERE repo_path = ?")
            .bind(repo_path)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("GitRepo at {repo_path}")))
    }

    /// List all git repos.
    pub async fn list_git_repos(&self) -> Result<Vec<GitRepoRow>, DatabaseError> {
        let repos = sqlx::query_as::<_, GitRepoRow>(
            "SELECT * FROM git_repos ORDER BY last_active DESC",
        )
        .fetch_all(self.pool())
        .await?;

        Ok(repos)
    }

    /// Update a git repo's configuration.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_git_repo(
        &self,
        id: &str,
        name: &str,
        worktree_mode: &str,
        local_subfolder: &str,
        custom_path: Option<&str>,
        setup_script: Option<&str>,
        auto_gitignore: bool,
    ) -> Result<GitRepoRow, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "UPDATE git_repos SET name = ?, worktree_mode = ?, local_subfolder = ?, \
             custom_path = ?, setup_script = ?, auto_gitignore = ?, last_active = ? \
             WHERE id = ?",
        )
        .bind(name)
        .bind(worktree_mode)
        .bind(local_subfolder)
        .bind(custom_path)
        .bind(setup_script)
        .bind(i64::from(auto_gitignore))
        .bind(now)
        .bind(id)
        .execute(self.pool())
        .await?;

        self.get_git_repo(id).await
    }

    /// Remove a git repo and all its worktrees.
    pub async fn remove_git_repo(&self, id: &str) -> Result<bool, DatabaseError> {
        // Clear worktree_id on sessions referencing any worktree of this repo
        sqlx::query(
            "UPDATE sessions SET worktree_id = NULL WHERE worktree_id IN \
             (SELECT id FROM worktrees WHERE repo_id = ?)",
        )
        .bind(id)
        .execute(self.pool())
        .await?;

        // Delete worktrees belonging to this repo
        sqlx::query("DELETE FROM worktrees WHERE repo_id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        // Delete the repo itself
        let result = sqlx::query("DELETE FROM git_repos WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Update the `last_active` timestamp on a git repo.
    pub async fn touch_git_repo(&self, id: &str) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE git_repos SET last_active = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use crate::storage::Database;

    #[tokio::test]
    async fn create_and_get_git_repo() {
        let db = Database::open_in_memory().await.unwrap();
        let repo = db
            .create_git_repo(
                "r1", "myrepo", "/path/to/repo", "global", ".worktree", None, None, true,
            )
            .await
            .unwrap();
        assert_eq!(repo.id, "r1");
        assert_eq!(repo.name, "myrepo");
        assert_eq!(repo.worktree_mode, "global");
        assert_eq!(repo.auto_gitignore, 1);

        let fetched = db.get_git_repo("r1").await.unwrap();
        assert_eq!(fetched.repo_path, "/path/to/repo");
    }

    #[tokio::test]
    async fn get_by_path() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_git_repo(
            "r1", "myrepo", "/path/to/repo", "global", ".worktree", None, None, true,
        )
        .await
        .unwrap();

        let repo = db.get_git_repo_by_path("/path/to/repo").await.unwrap();
        assert_eq!(repo.id, "r1");
    }

    #[tokio::test]
    async fn list_repos() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_git_repo("r1", "a", "/a", "global", ".worktree", None, None, true)
            .await
            .unwrap();
        db.create_git_repo("r2", "b", "/b", "local", ".wt", None, None, false)
            .await
            .unwrap();

        let repos = db.list_git_repos().await.unwrap();
        assert_eq!(repos.len(), 2);
    }

    #[tokio::test]
    async fn update_repo() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_git_repo(
            "r1", "old", "/repo", "global", ".worktree", None, None, true,
        )
        .await
        .unwrap();

        let updated = db
            .update_git_repo(
                "r1",
                "new-name",
                "custom",
                ".worktree",
                Some("/custom/path"),
                Some("make build"),
                false,
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "new-name");
        assert_eq!(updated.worktree_mode, "custom");
        assert_eq!(updated.custom_path.as_deref(), Some("/custom/path"));
        assert_eq!(updated.auto_gitignore, 0);
    }

    #[tokio::test]
    async fn remove_repo_cascades_to_worktrees() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_git_repo(
            "r1", "repo", "/repo", "global", ".worktree", None, None, true,
        )
        .await
        .unwrap();
        db.create_worktree("wt1", "feat", "/tmp/wt1", "feat", "r1", None)
            .await
            .unwrap();

        let removed = db.remove_git_repo("r1").await.unwrap();
        assert!(removed);

        // Worktree should be gone too
        assert!(db.get_worktree("wt1").await.is_err());
        // Repo should be gone
        assert!(db.get_git_repo("r1").await.is_err());
    }

    #[tokio::test]
    async fn remove_nonexistent_returns_false() {
        let db = Database::open_in_memory().await.unwrap();
        assert!(!db.remove_git_repo("nope").await.unwrap());
    }

    #[tokio::test]
    async fn duplicate_repo_path_fails() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_git_repo(
            "r1", "a", "/same/path", "global", ".worktree", None, None, true,
        )
        .await
        .unwrap();
        let result = db
            .create_git_repo(
                "r2", "b", "/same/path", "global", ".worktree", None, None, true,
            )
            .await;
        assert!(result.is_err());
    }
}
