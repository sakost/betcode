//! Database queries for `git_repos` table.

use betcode_core::db::unix_timestamp;

use super::db::{Database, DatabaseError};
use super::models::GitRepoRow;

/// Shared UPDATE statement used by both full and partial repo updates.
const UPDATE_GIT_REPO_SQL: &str = "UPDATE git_repos SET name = ?, worktree_mode = ?, \
    local_subfolder = ?, custom_path = ?, setup_script = ?, auto_gitignore = ?, \
    last_active = ? WHERE id = ?";

/// Common git repo fields shared between create and update operations.
pub struct GitRepoParams<'a> {
    pub name: &'a str,
    pub worktree_mode: &'a str,
    pub local_subfolder: &'a str,
    pub custom_path: Option<&'a str>,
    pub setup_script: Option<&'a str>,
    pub auto_gitignore: bool,
}

impl GitRepoParams<'_> {
    /// Bind the common repo fields to an existing query builder.
    fn bind_to<'q>(
        &'q self,
        query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    ) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        query
            .bind(self.name)
            .bind(self.worktree_mode)
            .bind(self.local_subfolder)
            .bind(self.custom_path)
            .bind(self.setup_script)
            .bind(i64::from(self.auto_gitignore))
    }
}

impl Database {
    // =========================================================================
    // GitRepo queries
    // =========================================================================

    /// Create a new git repo record.
    pub async fn create_git_repo(
        &self,
        id: &str,
        repo_path: &str,
        params: &GitRepoParams<'_>,
    ) -> Result<GitRepoRow, DatabaseError> {
        let now = unix_timestamp();

        let base = sqlx::query(
            "INSERT INTO git_repos (id, repo_path, name, worktree_mode, local_subfolder, custom_path, setup_script, auto_gitignore, created_at, last_active) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(repo_path);
        params
            .bind_to(base)
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

    /// Get a git repo by `repo_path`.
    pub async fn get_git_repo_by_path(&self, repo_path: &str) -> Result<GitRepoRow, DatabaseError> {
        sqlx::query_as::<_, GitRepoRow>("SELECT * FROM git_repos WHERE repo_path = ?")
            .bind(repo_path)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("GitRepo at {repo_path}")))
    }

    /// List all git repos.
    pub async fn list_git_repos(&self) -> Result<Vec<GitRepoRow>, DatabaseError> {
        let repos =
            sqlx::query_as::<_, GitRepoRow>("SELECT * FROM git_repos ORDER BY last_active DESC")
                .fetch_all(self.pool())
                .await?;

        Ok(repos)
    }

    /// List git repos with pagination (limit/offset).
    ///
    /// When `limit` is 0 it is treated as "no limit" (`SQLite` `LIMIT -1`).
    pub async fn list_git_repos_paginated(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<GitRepoRow>, DatabaseError> {
        let effective_limit: i64 = if limit == 0 { -1 } else { i64::from(limit) };
        let repos = sqlx::query_as::<_, GitRepoRow>(
            "SELECT * FROM git_repos ORDER BY last_active DESC LIMIT ? OFFSET ?",
        )
        .bind(effective_limit)
        .bind(offset)
        .fetch_all(self.pool())
        .await?;

        Ok(repos)
    }

    /// Update a git repo's configuration.
    pub async fn update_git_repo(
        &self,
        id: &str,
        params: &GitRepoParams<'_>,
    ) -> Result<GitRepoRow, DatabaseError> {
        let now = unix_timestamp();

        params
            .bind_to(sqlx::query(UPDATE_GIT_REPO_SQL))
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;

        self.get_git_repo(id).await
    }

    /// Atomically fetch-then-update a git repo inside a single transaction.
    ///
    /// Only the fields that are `Some(…)` are changed; `None` means "keep the
    /// existing value". For nullable columns (`custom_path`, `setup_script`)
    /// the outer `Option` controls whether to touch the column and the inner
    /// `Option` distinguishes "set to value" from "clear (set to NULL)".
    #[allow(clippy::too_many_arguments)]
    pub async fn update_git_repo_partial(
        &self,
        id: &str,
        name: Option<&str>,
        worktree_mode: Option<&str>,
        local_subfolder: Option<&str>,
        custom_path: Option<Option<&str>>,
        setup_script: Option<Option<&str>>,
        auto_gitignore: Option<bool>,
    ) -> Result<GitRepoRow, DatabaseError> {
        let mut tx = self.pool().begin().await?;

        // Fetch existing row inside the transaction.
        let existing = sqlx::query_as::<_, GitRepoRow>("SELECT * FROM git_repos WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("GitRepo {id}")))?;

        let final_name = name.unwrap_or(&existing.name);
        let final_wt = worktree_mode.unwrap_or(&existing.worktree_mode);
        let final_sub = local_subfolder.unwrap_or(&existing.local_subfolder);
        let final_cp: Option<&str> = match custom_path {
            Some(v) => v,
            None => existing.custom_path.as_deref(),
        };
        let final_ss: Option<&str> = match setup_script {
            Some(v) => v,
            None => existing.setup_script.as_deref(),
        };
        let final_ag = auto_gitignore.unwrap_or(existing.auto_gitignore != 0);
        let now = unix_timestamp();

        let merged = GitRepoParams {
            name: final_name,
            worktree_mode: final_wt,
            local_subfolder: final_sub,
            custom_path: final_cp,
            setup_script: final_ss,
            auto_gitignore: final_ag,
        };
        merged
            .bind_to(sqlx::query(UPDATE_GIT_REPO_SQL))
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let updated = sqlx::query_as::<_, GitRepoRow>("SELECT * FROM git_repos WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(updated)
    }

    /// Remove a git repo and all its worktrees (transactionally).
    pub async fn remove_git_repo(&self, id: &str) -> Result<bool, DatabaseError> {
        let mut tx = self.pool().begin().await?;

        // Clear worktree_id on sessions referencing any worktree of this repo
        sqlx::query(
            "UPDATE sessions SET worktree_id = NULL WHERE worktree_id IN \
             (SELECT id FROM worktrees WHERE repo_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;

        // Delete worktrees belonging to this repo
        sqlx::query("DELETE FROM worktrees WHERE repo_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        // Delete the repo itself
        let result = sqlx::query("DELETE FROM git_repos WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(result.rows_affected() > 0)
    }

    /// Count worktrees for a single repo.
    pub async fn count_worktrees_for_repo(&self, repo_id: &str) -> Result<u32, DatabaseError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM worktrees WHERE repo_id = ?")
            .bind(repo_id)
            .fetch_one(self.pool())
            .await?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(row.0 as u32)
    }

    /// Count worktrees per repo in a single query. Returns a map of `repo_id` -> count.
    pub async fn count_worktrees_by_repo(
        &self,
    ) -> Result<std::collections::HashMap<String, u32>, DatabaseError> {
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT repo_id, COUNT(*) FROM worktrees GROUP BY repo_id")
                .fetch_all(self.pool())
                .await?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(rows
            .into_iter()
            .map(|(id, count)| (id, count as u32))
            .collect())
    }

    /// Count the total number of registered git repos.
    pub async fn count_git_repos(&self) -> Result<u32, DatabaseError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM git_repos")
            .fetch_one(self.pool())
            .await?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(row.0 as u32)
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

    /// Create a simple git repo with sensible defaults for testing.
    async fn create_test_repo(
        db: &Database,
        id: &str,
        name: &str,
        path: &str,
    ) -> super::GitRepoRow {
        db.create_git_repo(
            id,
            path,
            &super::GitRepoParams {
                name,
                worktree_mode: "global",
                local_subfolder: ".worktree",
                custom_path: None,
                setup_script: None,
                auto_gitignore: true,
            },
        )
        .await
        .unwrap()
    }

    /// Seed two repos ("ra", "rb") with 3 worktrees: 2 for "ra" and 1 for "rb".
    async fn seed_repos_and_worktrees(db: &Database) {
        create_test_repo(db, "ra", "repo-a", "/repo-a").await;
        create_test_repo(db, "rb", "repo-b", "/repo-b").await;

        db.create_worktree("wt1", "feat1", "/tmp/wt1", "feat1", "ra", None)
            .await
            .unwrap();
        db.create_worktree("wt2", "feat2", "/tmp/wt2", "feat2", "ra", None)
            .await
            .unwrap();
        db.create_worktree("wt3", "feat3", "/tmp/wt3", "feat3", "rb", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_and_get_git_repo() {
        let db = Database::open_in_memory().await.unwrap();
        let repo = db
            .create_git_repo(
                "r1",
                "/path/to/repo",
                &super::GitRepoParams {
                    name: "myrepo",
                    worktree_mode: "global",
                    local_subfolder: ".worktree",
                    custom_path: None,
                    setup_script: None,
                    auto_gitignore: true,
                },
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
            "r1",
            "/path/to/repo",
            &super::GitRepoParams {
                name: "myrepo",
                worktree_mode: "global",
                local_subfolder: ".worktree",
                custom_path: None,
                setup_script: None,
                auto_gitignore: true,
            },
        )
        .await
        .unwrap();

        let repo = db.get_git_repo_by_path("/path/to/repo").await.unwrap();
        assert_eq!(repo.id, "r1");
    }

    #[tokio::test]
    async fn list_repos() {
        let db = Database::open_in_memory().await.unwrap();
        create_test_repo(&db, "r1", "a", "/a").await;
        db.create_git_repo(
            "r2",
            "/b",
            &super::GitRepoParams {
                name: "b",
                worktree_mode: "local",
                local_subfolder: ".wt",
                custom_path: None,
                setup_script: None,
                auto_gitignore: false,
            },
        )
        .await
        .unwrap();

        let repos = db.list_git_repos().await.unwrap();
        assert_eq!(repos.len(), 2);
    }

    #[tokio::test]
    async fn update_repo() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_git_repo(
            "r1",
            "/repo",
            &super::GitRepoParams {
                name: "old",
                worktree_mode: "global",
                local_subfolder: ".worktree",
                custom_path: None,
                setup_script: None,
                auto_gitignore: true,
            },
        )
        .await
        .unwrap();

        let updated = db
            .update_git_repo(
                "r1",
                &super::GitRepoParams {
                    name: "new-name",
                    worktree_mode: "custom",
                    local_subfolder: ".worktree",
                    custom_path: Some("/custom/path"),
                    setup_script: Some("make build"),
                    auto_gitignore: false,
                },
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
            "r1",
            "/repo",
            &super::GitRepoParams {
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
    async fn list_repos_paginated() {
        let db = Database::open_in_memory().await.unwrap();
        // Create 3 repos
        for i in 0..3 {
            create_test_repo(
                &db,
                &format!("r{i}"),
                &format!("repo-{i}"),
                &format!("/path/{i}"),
            )
            .await;
        }

        // Page 1: limit 2, offset 0
        let page1 = db.list_git_repos_paginated(2, 0).await.unwrap();
        assert_eq!(page1.len(), 2);

        // Page 2: limit 2, offset 2
        let page2 = db.list_git_repos_paginated(2, 2).await.unwrap();
        assert_eq!(page2.len(), 1);

        // Offset past all results
        let empty = db.list_git_repos_paginated(10, 10).await.unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn duplicate_repo_path_fails() {
        let db = Database::open_in_memory().await.unwrap();
        create_test_repo(&db, "r1", "a", "/same/path").await;
        let result = db
            .create_git_repo(
                "r2",
                "/same/path",
                &super::GitRepoParams {
                    name: "b",
                    worktree_mode: "global",
                    local_subfolder: ".worktree",
                    custom_path: None,
                    setup_script: None,
                    auto_gitignore: true,
                },
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn count_worktrees_for_repo() {
        let db = Database::open_in_memory().await.unwrap();
        seed_repos_and_worktrees(&db).await;

        assert_eq!(db.count_worktrees_for_repo("ra").await.unwrap(), 2);
        assert_eq!(db.count_worktrees_for_repo("rb").await.unwrap(), 1);
        assert_eq!(db.count_worktrees_for_repo("nonexistent").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn count_git_repos() {
        let db = Database::open_in_memory().await.unwrap();
        assert_eq!(db.count_git_repos().await.unwrap(), 0);

        create_test_repo(&db, "r1", "a", "/a").await;
        assert_eq!(db.count_git_repos().await.unwrap(), 1);

        create_test_repo(&db, "r2", "b", "/b").await;
        assert_eq!(db.count_git_repos().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn list_repos_paginated_limit_zero_returns_all_with_offset() {
        let db = Database::open_in_memory().await.unwrap();
        for i in 0..5 {
            create_test_repo(
                &db,
                &format!("r{i}"),
                &format!("repo-{i}"),
                &format!("/path/{i}"),
            )
            .await;
        }

        // limit=0, offset=0 should return all 5
        let all = db.list_git_repos_paginated(0, 0).await.unwrap();
        assert_eq!(all.len(), 5);

        // limit=0, offset=2 should return 3 (all minus first 2)
        let from_offset = db.list_git_repos_paginated(0, 2).await.unwrap();
        assert_eq!(from_offset.len(), 3);

        // limit=0, offset=10 should return 0 (past all)
        let empty = db.list_git_repos_paginated(0, 10).await.unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn count_worktrees_by_repo() {
        let db = Database::open_in_memory().await.unwrap();

        // Empty map when no worktrees exist
        let counts = db.count_worktrees_by_repo().await.unwrap();
        assert!(counts.is_empty());

        // Seed 2 repos + 3 worktrees, plus an extra repo with no worktrees
        seed_repos_and_worktrees(&db).await;
        create_test_repo(&db, "rc", "repo-c", "/repo-c").await;

        let counts = db.count_worktrees_by_repo().await.unwrap();
        assert_eq!(counts.get("ra").copied(), Some(2));
        assert_eq!(counts.get("rb").copied(), Some(1));
        // Repos with no worktrees are not in the map
        assert_eq!(counts.get("rc"), None);
    }

    #[tokio::test]
    async fn update_git_repo_partial_transactional() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_git_repo(
            "r1",
            "/repo",
            &super::GitRepoParams {
                name: "original",
                worktree_mode: "global",
                local_subfolder: ".worktree",
                custom_path: Some("/old/path"),
                setup_script: Some("old script"),
                auto_gitignore: true,
            },
        )
        .await
        .unwrap();

        // Update only name, leave everything else untouched
        let updated = db
            .update_git_repo_partial("r1", Some("renamed"), None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.worktree_mode, "global");
        assert_eq!(updated.custom_path.as_deref(), Some("/old/path"));
        assert_eq!(updated.setup_script.as_deref(), Some("old script"));
        assert_eq!(updated.auto_gitignore, 1);

        // Clear custom_path (set to NULL), set new setup_script
        let updated = db
            .update_git_repo_partial(
                "r1",
                None,
                None,
                None,
                Some(None),               // clear custom_path
                Some(Some("new script")), // set new setup_script
                Some(false),
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "renamed"); // unchanged from previous
        assert!(updated.custom_path.is_none());
        assert_eq!(updated.setup_script.as_deref(), Some("new script"));
        assert_eq!(updated.auto_gitignore, 0);

        // Not found should error
        let err = db
            .update_git_repo_partial("nonexistent", Some("x"), None, None, None, None, None)
            .await;
        assert!(err.is_err());
    }
}
