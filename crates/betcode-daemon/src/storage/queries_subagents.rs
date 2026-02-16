//! Database queries for subagent orchestration tables.

use betcode_core::db::unix_timestamp;

use super::db::{Database, DatabaseError};
use super::models::{OrchestrationRow, OrchestrationStepRow, SubagentRow};

impl Database {
    // =========================================================================
    // Subagent queries
    // =========================================================================

    /// Create a new subagent record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_subagent(
        &self,
        id: &str,
        parent_session_id: &str,
        prompt: &str,
        model: Option<&str>,
        max_turns: i64,
        auto_approve: bool,
        allowed_tools: &str,
        working_directory: Option<&str>,
    ) -> Result<SubagentRow, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            r"
            INSERT INTO subagents
                (id, parent_session_id, prompt, model, max_turns, auto_approve,
                 allowed_tools, working_directory, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(id)
        .bind(parent_session_id)
        .bind(prompt)
        .bind(model)
        .bind(max_turns)
        .bind(i64::from(auto_approve))
        .bind(allowed_tools)
        .bind(working_directory)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_subagent(id).await
    }

    /// Get a subagent by ID.
    pub async fn get_subagent(&self, id: &str) -> Result<SubagentRow, DatabaseError> {
        sqlx::query_as::<_, SubagentRow>("SELECT * FROM subagents WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Subagent {id}")))
    }

    /// List subagents for a parent session, optionally filtered by status.
    pub async fn list_subagents_for_session(
        &self,
        parent_session_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<SubagentRow>, DatabaseError> {
        let subagents = if let Some(status) = status_filter {
            sqlx::query_as::<_, SubagentRow>(
                "SELECT * FROM subagents WHERE parent_session_id = ? AND status = ? ORDER BY created_at DESC",
            )
            .bind(parent_session_id)
            .bind(status)
            .fetch_all(self.pool())
            .await?
        } else {
            sqlx::query_as::<_, SubagentRow>(
                "SELECT * FROM subagents WHERE parent_session_id = ? ORDER BY created_at DESC",
            )
            .bind(parent_session_id)
            .fetch_all(self.pool())
            .await?
        };

        Ok(subagents)
    }

    /// Update a subagent's status. Optionally sets `exit_code`,
    /// `result_summary`, `started_at`, and `completed_at` depending on the
    /// new status.
    pub async fn update_subagent_status(
        &self,
        id: &str,
        status: &str,
        exit_code: Option<i64>,
        result_summary: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        // Set started_at when transitioning to running, completed_at for
        // terminal states.
        match status {
            "running" => {
                sqlx::query("UPDATE subagents SET status = ?, started_at = ? WHERE id = ?")
                    .bind(status)
                    .bind(now)
                    .bind(id)
                    .execute(self.pool())
                    .await?;
            }
            "completed" | "failed" | "cancelled" => {
                sqlx::query(
                    "UPDATE subagents SET status = ?, exit_code = ?, result_summary = ?, completed_at = ? WHERE id = ?",
                )
                .bind(status)
                .bind(exit_code)
                .bind(result_summary)
                .bind(now)
                .bind(id)
                .execute(self.pool())
                .await?;
            }
            _ => {
                sqlx::query("UPDATE subagents SET status = ? WHERE id = ?")
                    .bind(status)
                    .bind(id)
                    .execute(self.pool())
                    .await?;
            }
        }

        Ok(())
    }

    // =========================================================================
    // Orchestration queries
    // =========================================================================

    /// Create a new orchestration record.
    pub async fn create_orchestration(
        &self,
        id: &str,
        parent_session_id: &str,
        strategy: &str,
    ) -> Result<OrchestrationRow, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            r"
            INSERT INTO orchestrations (id, parent_session_id, strategy, created_at)
            VALUES (?, ?, ?, ?)
            ",
        )
        .bind(id)
        .bind(parent_session_id)
        .bind(strategy)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_orchestration(id).await
    }

    /// Get an orchestration by ID.
    pub async fn get_orchestration(&self, id: &str) -> Result<OrchestrationRow, DatabaseError> {
        sqlx::query_as::<_, OrchestrationRow>("SELECT * FROM orchestrations WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Orchestration {id}")))
    }

    /// Update an orchestration's status.
    pub async fn update_orchestration_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        let completed_at: Option<i64> = match status {
            "completed" | "failed" => Some(now),
            _ => None,
        };

        sqlx::query(
            "UPDATE orchestrations SET status = ?, completed_at = COALESCE(?, completed_at) WHERE id = ?",
        )
        .bind(status)
        .bind(completed_at)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    // =========================================================================
    // Orchestration step queries
    // =========================================================================

    /// Create an orchestration step.
    pub async fn create_orchestration_step(
        &self,
        id: &str,
        orchestration_id: &str,
        step_index: i64,
        prompt: &str,
        depends_on: &str,
    ) -> Result<OrchestrationStepRow, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            r"
            INSERT INTO orchestration_steps
                (id, orchestration_id, step_index, prompt, depends_on, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(id)
        .bind(orchestration_id)
        .bind(step_index)
        .bind(prompt)
        .bind(depends_on)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_orchestration_step(id).await
    }

    /// Get an orchestration step by ID.
    pub async fn get_orchestration_step(
        &self,
        id: &str,
    ) -> Result<OrchestrationStepRow, DatabaseError> {
        sqlx::query_as::<_, OrchestrationStepRow>("SELECT * FROM orchestration_steps WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("OrchestrationStep {id}")))
    }

    /// Get all steps for an orchestration, ordered by `step_index`.
    pub async fn get_steps_for_orchestration(
        &self,
        orchestration_id: &str,
    ) -> Result<Vec<OrchestrationStepRow>, DatabaseError> {
        let steps = sqlx::query_as::<_, OrchestrationStepRow>(
            "SELECT * FROM orchestration_steps WHERE orchestration_id = ? ORDER BY step_index ASC",
        )
        .bind(orchestration_id)
        .fetch_all(self.pool())
        .await?;

        Ok(steps)
    }

    /// Update an orchestration step's status and optionally link it to a subagent.
    pub async fn update_step_status(
        &self,
        id: &str,
        status: &str,
        subagent_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        if let Some(sa_id) = subagent_id {
            sqlx::query("UPDATE orchestration_steps SET status = ?, subagent_id = ? WHERE id = ?")
                .bind(status)
                .bind(sa_id)
                .bind(id)
                .execute(self.pool())
                .await?;
        } else {
            sqlx::query("UPDATE orchestration_steps SET status = ? WHERE id = ?")
                .bind(status)
                .bind(id)
                .execute(self.pool())
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use crate::storage::Database;

    /// Seed a parent session used as the FK target for subagent tests.
    async fn seed_parent_session(db: &Database) {
        db.create_session("parent-1", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();
    }

    // =========================================================================
    // Subagent tests
    // =========================================================================

    #[tokio::test]
    async fn create_and_get_subagent() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        let sa = db
            .create_subagent(
                "sa-1",
                "parent-1",
                "Write unit tests",
                Some("claude-haiku-4"),
                20,
                false,
                "[]",
                Some("/tmp/work"),
            )
            .await
            .unwrap();

        assert_eq!(sa.id, "sa-1");
        assert_eq!(sa.parent_session_id, "parent-1");
        assert_eq!(sa.prompt, "Write unit tests");
        assert_eq!(sa.model.as_deref(), Some("claude-haiku-4"));
        assert_eq!(sa.max_turns, 20);
        assert_eq!(sa.auto_approve, 0);
        assert_eq!(sa.status, "pending");
        assert!(sa.exit_code.is_none());
        assert!(sa.result_summary.is_none());
        assert!(sa.started_at.is_none());
        assert!(sa.completed_at.is_none());

        let fetched = db.get_subagent("sa-1").await.unwrap();
        assert_eq!(fetched.id, "sa-1");
    }

    #[tokio::test]
    async fn create_subagent_with_auto_approve() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        let sa = db
            .create_subagent(
                "sa-2",
                "parent-1",
                "Run tests",
                None,
                10,
                true,
                r#"["Read","Bash"]"#,
                None,
            )
            .await
            .unwrap();

        assert_eq!(sa.auto_approve, 1);
        assert_eq!(sa.allowed_tools, r#"["Read","Bash"]"#);
        assert!(sa.model.is_none());
        assert!(sa.working_directory.is_none());
    }

    #[tokio::test]
    async fn get_nonexistent_subagent_returns_not_found() {
        let db = Database::open_in_memory().await.unwrap();
        let result = db.get_subagent("nonexistent").await;
        assert!(matches!(
            result,
            Err(crate::storage::DatabaseError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn list_subagents_for_session() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_subagent("sa-1", "parent-1", "task 1", None, 10, false, "[]", None)
            .await
            .unwrap();
        db.create_subagent("sa-2", "parent-1", "task 2", None, 10, false, "[]", None)
            .await
            .unwrap();

        let all = db
            .list_subagents_for_session("parent-1", None)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_subagents_with_status_filter() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_subagent("sa-1", "parent-1", "task 1", None, 10, false, "[]", None)
            .await
            .unwrap();
        db.create_subagent("sa-2", "parent-1", "task 2", None, 10, false, "[]", None)
            .await
            .unwrap();

        // Mark one as running
        db.update_subagent_status("sa-1", "running", None, None)
            .await
            .unwrap();

        let pending = db
            .list_subagents_for_session("parent-1", Some("pending"))
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "sa-2");

        let running = db
            .list_subagents_for_session("parent-1", Some("running"))
            .await
            .unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, "sa-1");
    }

    #[tokio::test]
    async fn update_subagent_status_to_running() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_subagent("sa-1", "parent-1", "task", None, 10, false, "[]", None)
            .await
            .unwrap();

        db.update_subagent_status("sa-1", "running", None, None)
            .await
            .unwrap();

        let sa = db.get_subagent("sa-1").await.unwrap();
        assert_eq!(sa.status, "running");
        assert!(sa.started_at.is_some());
        assert!(sa.completed_at.is_none());
    }

    #[tokio::test]
    async fn update_subagent_status_to_completed() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_subagent("sa-1", "parent-1", "task", None, 10, false, "[]", None)
            .await
            .unwrap();

        db.update_subagent_status("sa-1", "running", None, None)
            .await
            .unwrap();
        db.update_subagent_status("sa-1", "completed", Some(0), Some("All tests passed"))
            .await
            .unwrap();

        let sa = db.get_subagent("sa-1").await.unwrap();
        assert_eq!(sa.status, "completed");
        assert_eq!(sa.exit_code, Some(0));
        assert_eq!(sa.result_summary.as_deref(), Some("All tests passed"));
        assert!(sa.completed_at.is_some());
    }

    #[tokio::test]
    async fn update_subagent_status_to_failed() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_subagent("sa-1", "parent-1", "task", None, 10, false, "[]", None)
            .await
            .unwrap();

        db.update_subagent_status("sa-1", "failed", Some(1), Some("Compilation error"))
            .await
            .unwrap();

        let sa = db.get_subagent("sa-1").await.unwrap();
        assert_eq!(sa.status, "failed");
        assert_eq!(sa.exit_code, Some(1));
        assert_eq!(sa.result_summary.as_deref(), Some("Compilation error"));
        assert!(sa.completed_at.is_some());
    }

    #[tokio::test]
    async fn update_subagent_status_to_cancelled() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_subagent("sa-1", "parent-1", "task", None, 10, false, "[]", None)
            .await
            .unwrap();

        db.update_subagent_status("sa-1", "cancelled", None, Some("User cancelled"))
            .await
            .unwrap();

        let sa = db.get_subagent("sa-1").await.unwrap();
        assert_eq!(sa.status, "cancelled");
        assert!(sa.completed_at.is_some());
    }

    #[tokio::test]
    async fn subagents_cascade_on_session_delete() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_subagent("sa-1", "parent-1", "task", None, 10, false, "[]", None)
            .await
            .unwrap();

        // Delete parent session -> subagents cascade
        db.delete_session("parent-1").await.unwrap();

        let result = db.get_subagent("sa-1").await;
        assert!(matches!(
            result,
            Err(crate::storage::DatabaseError::NotFound(_))
        ));
    }

    // =========================================================================
    // Orchestration tests
    // =========================================================================

    #[tokio::test]
    async fn create_and_get_orchestration() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        let orch = db
            .create_orchestration("orch-1", "parent-1", "parallel")
            .await
            .unwrap();

        assert_eq!(orch.id, "orch-1");
        assert_eq!(orch.parent_session_id, "parent-1");
        assert_eq!(orch.strategy, "parallel");
        assert_eq!(orch.status, "pending");
        assert!(orch.completed_at.is_none());
    }

    #[tokio::test]
    async fn get_nonexistent_orchestration_returns_not_found() {
        let db = Database::open_in_memory().await.unwrap();
        let result = db.get_orchestration("nonexistent").await;
        assert!(matches!(
            result,
            Err(crate::storage::DatabaseError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn update_orchestration_status_to_running() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_orchestration("orch-1", "parent-1", "dag")
            .await
            .unwrap();

        db.update_orchestration_status("orch-1", "running")
            .await
            .unwrap();

        let orch = db.get_orchestration("orch-1").await.unwrap();
        assert_eq!(orch.status, "running");
        assert!(orch.completed_at.is_none());
    }

    #[tokio::test]
    async fn update_orchestration_status_to_completed() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_orchestration("orch-1", "parent-1", "sequential")
            .await
            .unwrap();

        db.update_orchestration_status("orch-1", "completed")
            .await
            .unwrap();

        let orch = db.get_orchestration("orch-1").await.unwrap();
        assert_eq!(orch.status, "completed");
        assert!(orch.completed_at.is_some());
    }

    #[tokio::test]
    async fn update_orchestration_status_to_failed() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_orchestration("orch-1", "parent-1", "dag")
            .await
            .unwrap();

        db.update_orchestration_status("orch-1", "failed")
            .await
            .unwrap();

        let orch = db.get_orchestration("orch-1").await.unwrap();
        assert_eq!(orch.status, "failed");
        assert!(orch.completed_at.is_some());
    }

    #[tokio::test]
    async fn orchestrations_cascade_on_session_delete() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        db.create_orchestration("orch-1", "parent-1", "parallel")
            .await
            .unwrap();

        db.delete_session("parent-1").await.unwrap();

        let result = db.get_orchestration("orch-1").await;
        assert!(matches!(
            result,
            Err(crate::storage::DatabaseError::NotFound(_))
        ));
    }

    // =========================================================================
    // Orchestration step tests
    // =========================================================================

    #[tokio::test]
    async fn create_and_get_orchestration_step() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;
        db.create_orchestration("orch-1", "parent-1", "dag")
            .await
            .unwrap();

        let step = db
            .create_orchestration_step("step-1", "orch-1", 0, "Analyze codebase", "[]")
            .await
            .unwrap();

        assert_eq!(step.id, "step-1");
        assert_eq!(step.orchestration_id, "orch-1");
        assert_eq!(step.step_index, 0);
        assert_eq!(step.prompt, "Analyze codebase");
        assert_eq!(step.depends_on, "[]");
        assert_eq!(step.status, "pending");
        assert!(step.subagent_id.is_none());
    }

    #[tokio::test]
    async fn get_steps_for_orchestration_ordered() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;
        db.create_orchestration("orch-1", "parent-1", "sequential")
            .await
            .unwrap();

        db.create_orchestration_step("step-b", "orch-1", 1, "Step B", "[]")
            .await
            .unwrap();
        db.create_orchestration_step("step-a", "orch-1", 0, "Step A", "[]")
            .await
            .unwrap();
        db.create_orchestration_step("step-c", "orch-1", 2, "Step C", "[]")
            .await
            .unwrap();

        let steps = db.get_steps_for_orchestration("orch-1").await.unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].id, "step-a");
        assert_eq!(steps[1].id, "step-b");
        assert_eq!(steps[2].id, "step-c");
    }

    #[tokio::test]
    async fn update_step_status_with_subagent() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;
        db.create_orchestration("orch-1", "parent-1", "dag")
            .await
            .unwrap();
        db.create_subagent("sa-1", "parent-1", "task", None, 10, false, "[]", None)
            .await
            .unwrap();
        db.create_orchestration_step("step-1", "orch-1", 0, "Run task", "[]")
            .await
            .unwrap();

        db.update_step_status("step-1", "running", Some("sa-1"))
            .await
            .unwrap();

        let step = db.get_orchestration_step("step-1").await.unwrap();
        assert_eq!(step.status, "running");
        assert_eq!(step.subagent_id.as_deref(), Some("sa-1"));
    }

    #[tokio::test]
    async fn update_step_status_without_subagent() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;
        db.create_orchestration("orch-1", "parent-1", "dag")
            .await
            .unwrap();
        db.create_orchestration_step("step-1", "orch-1", 0, "Run task", "[]")
            .await
            .unwrap();

        db.update_step_status("step-1", "blocked", None)
            .await
            .unwrap();

        let step = db.get_orchestration_step("step-1").await.unwrap();
        assert_eq!(step.status, "blocked");
        assert!(step.subagent_id.is_none());
    }

    #[tokio::test]
    async fn steps_cascade_on_orchestration_delete() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;
        db.create_orchestration("orch-1", "parent-1", "parallel")
            .await
            .unwrap();
        db.create_orchestration_step("step-1", "orch-1", 0, "Task", "[]")
            .await
            .unwrap();

        // Delete session -> orchestration cascades -> steps cascade
        db.delete_session("parent-1").await.unwrap();

        let result = db.get_orchestration_step("step-1").await;
        assert!(matches!(
            result,
            Err(crate::storage::DatabaseError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn step_depends_on_stores_json() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;
        db.create_orchestration("orch-1", "parent-1", "dag")
            .await
            .unwrap();

        db.create_orchestration_step("step-a", "orch-1", 0, "First", "[]")
            .await
            .unwrap();
        db.create_orchestration_step("step-b", "orch-1", 1, "Second", r#"["step-a"]"#)
            .await
            .unwrap();

        let steps = db.get_steps_for_orchestration("orch-1").await.unwrap();
        assert_eq!(steps[0].depends_on, "[]");
        assert_eq!(steps[1].depends_on, r#"["step-a"]"#);
    }

    #[tokio::test]
    async fn step_subagent_set_null_on_subagent_delete() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        // Create a second session to be the parent of a separate subagent
        // so deleting sa-1 doesn't cascade from session deletion
        db.create_subagent("sa-1", "parent-1", "task", None, 10, false, "[]", None)
            .await
            .unwrap();
        db.create_orchestration("orch-1", "parent-1", "dag")
            .await
            .unwrap();
        db.create_orchestration_step("step-1", "orch-1", 0, "Task", "[]")
            .await
            .unwrap();

        // Link step to subagent
        db.update_step_status("step-1", "running", Some("sa-1"))
            .await
            .unwrap();

        let step = db.get_orchestration_step("step-1").await.unwrap();
        assert_eq!(step.subagent_id.as_deref(), Some("sa-1"));

        // Manually delete subagent to trigger ON DELETE SET NULL
        sqlx::query("DELETE FROM subagents WHERE id = ?")
            .bind("sa-1")
            .execute(db.pool())
            .await
            .unwrap();

        let step = db.get_orchestration_step("step-1").await.unwrap();
        assert!(step.subagent_id.is_none());
    }

    #[tokio::test]
    async fn list_subagents_empty_session() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;

        let all = db
            .list_subagents_for_session("parent-1", None)
            .await
            .unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn get_steps_for_empty_orchestration() {
        let db = Database::open_in_memory().await.unwrap();
        seed_parent_session(&db).await;
        db.create_orchestration("orch-1", "parent-1", "parallel")
            .await
            .unwrap();

        let steps = db.get_steps_for_orchestration("orch-1").await.unwrap();
        assert!(steps.is_empty());
    }

    #[tokio::test]
    async fn migration_applies_cleanly() {
        // Just opening an in-memory DB runs all migrations
        let db = Database::open_in_memory().await.unwrap();

        // Verify tables exist by inserting and querying
        seed_parent_session(&db).await;
        db.create_subagent("sa-test", "parent-1", "p", None, 5, false, "[]", None)
            .await
            .unwrap();
        db.create_orchestration("orch-test", "parent-1", "parallel")
            .await
            .unwrap();
        db.create_orchestration_step("step-test", "orch-test", 0, "p", "[]")
            .await
            .unwrap();

        // All tables functional
        assert_eq!(db.get_subagent("sa-test").await.unwrap().id, "sa-test");
        assert_eq!(
            db.get_orchestration("orch-test").await.unwrap().id,
            "orch-test"
        );
        assert_eq!(
            db.get_orchestration_step("step-test").await.unwrap().id,
            "step-test"
        );
    }
}
