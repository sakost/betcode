//! Database queries for `BetCode` daemon.

use betcode_core::db::unix_timestamp;

use super::db::{Database, DatabaseError};
use super::models::{Session, SessionStatus, Message, PermissionGrant, Worktree};

impl Database {
    // =========================================================================
    // Session queries
    // =========================================================================

    /// Create a new session.
    pub async fn create_session(
        &self,
        id: &str,
        model: &str,
        working_directory: &str,
    ) -> Result<Session, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            r"
            INSERT INTO sessions (id, model, working_directory, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ",
        )
        .bind(id)
        .bind(model)
        .bind(working_directory)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_session(id).await
    }

    /// Get a session by ID.
    pub async fn get_session(&self, id: &str) -> Result<Session, DatabaseError> {
        sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Session {id}")))
    }

    /// Update session status.
    pub async fn update_session_status(
        &self,
        id: &str,
        status: SessionStatus,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE sessions SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Update the Claude session ID (from system.init).
    pub async fn update_claude_session_id(
        &self,
        id: &str,
        claude_session_id: &str,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE sessions SET claude_session_id = ?, updated_at = ? WHERE id = ?")
            .bind(claude_session_id)
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Update session usage stats.
    pub async fn update_session_usage(
        &self,
        id: &str,
        input_tokens: i64,
        output_tokens: i64,
        cost_usd: f64,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "UPDATE sessions SET total_input_tokens = total_input_tokens + ?, total_output_tokens = total_output_tokens + ?, total_cost_usd = total_cost_usd + ?, updated_at = ? WHERE id = ?",
        )
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cost_usd)
        .bind(now)
        .bind(id)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// List sessions, optionally filtered.
    pub async fn list_sessions(
        &self,
        working_directory: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Session>, DatabaseError> {
        let sessions = if let Some(wd) = working_directory {
            sqlx::query_as::<_, Session>(
                "SELECT * FROM sessions WHERE working_directory = ? ORDER BY updated_at DESC LIMIT ? OFFSET ?",
            )
            .bind(wd)
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool())
            .await?
        } else {
            sqlx::query_as::<_, Session>(
                "SELECT * FROM sessions ORDER BY updated_at DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool())
            .await?
        };

        Ok(sessions)
    }

    // =========================================================================
    // Message queries
    // =========================================================================

    /// Insert a message.
    pub async fn insert_message(
        &self,
        session_id: &str,
        sequence: i64,
        message_type: &str,
        payload: &str,
    ) -> Result<i64, DatabaseError> {
        let now = unix_timestamp();

        let result = sqlx::query(
            "INSERT INTO messages (session_id, sequence, message_type, payload, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(sequence)
        .bind(message_type)
        .bind(payload)
        .bind(now)
        .execute(self.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Get messages for replay starting from a sequence.
    pub async fn get_messages_from_sequence(
        &self,
        session_id: &str,
        from_sequence: i64,
    ) -> Result<Vec<Message>, DatabaseError> {
        let messages = sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE session_id = ? AND sequence > ? ORDER BY sequence ASC",
        )
        .bind(session_id)
        .bind(from_sequence)
        .fetch_all(self.pool())
        .await?;

        Ok(messages)
    }

    // =========================================================================
    // Permission queries
    // =========================================================================

    /// Insert a permission grant.
    pub async fn insert_permission_grant(
        &self,
        session_id: &str,
        tool_name: &str,
        pattern: Option<&str>,
        action: &str,
    ) -> Result<i64, DatabaseError> {
        let now = unix_timestamp();

        let result = sqlx::query(
            "INSERT INTO permission_grants (session_id, tool_name, pattern, action, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(tool_name)
        .bind(pattern)
        .bind(action)
        .bind(now)
        .execute(self.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Get permission grant for a tool.
    pub async fn get_permission_grant(
        &self,
        session_id: &str,
        tool_name: &str,
    ) -> Result<Option<PermissionGrant>, DatabaseError> {
        let grant = sqlx::query_as::<_, PermissionGrant>(
            "SELECT * FROM permission_grants WHERE session_id = ? AND tool_name = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(tool_name)
        .fetch_optional(self.pool())
        .await?;

        Ok(grant)
    }

    // =========================================================================
    // Worktree queries
    // =========================================================================

    /// Create a new worktree record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_worktree(
        &self,
        id: &str,
        name: &str,
        path: &str,
        branch: &str,
        repo_path: &str,
        setup_script: Option<&str>,
    ) -> Result<Worktree, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO worktrees (id, name, path, branch, repo_path, setup_script, created_at, last_active) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(path)
        .bind(branch)
        .bind(repo_path)
        .bind(setup_script)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_worktree(id).await
    }

    /// Get a worktree by ID.
    pub async fn get_worktree(&self, id: &str) -> Result<Worktree, DatabaseError> {
        sqlx::query_as::<_, Worktree>("SELECT * FROM worktrees WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Worktree {id}")))
    }

    /// List worktrees, optionally filtered by repository path.
    pub async fn list_worktrees(
        &self,
        repo_path: Option<&str>,
    ) -> Result<Vec<Worktree>, DatabaseError> {
        let worktrees = if let Some(rp) = repo_path {
            sqlx::query_as::<_, Worktree>(
                "SELECT * FROM worktrees WHERE repo_path = ? ORDER BY last_active DESC",
            )
            .bind(rp)
            .fetch_all(self.pool())
            .await?
        } else {
            sqlx::query_as::<_, Worktree>("SELECT * FROM worktrees ORDER BY last_active DESC")
                .fetch_all(self.pool())
                .await?
        };

        Ok(worktrees)
    }

    /// Remove a worktree record. Sessions bound to this worktree have their
    /// `worktree_id` set to NULL (handled by application logic, not FK cascade
    /// since `worktree_id` is not a formal FK in the schema).
    pub async fn remove_worktree(&self, id: &str) -> Result<bool, DatabaseError> {
        // Clear worktree_id on sessions that reference this worktree
        sqlx::query("UPDATE sessions SET worktree_id = NULL WHERE worktree_id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        let result = sqlx::query("DELETE FROM worktrees WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Update the `last_active` timestamp on a worktree.
    pub async fn touch_worktree(&self, id: &str) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE worktrees SET last_active = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Bind a session to a worktree.
    pub async fn bind_session_to_worktree(
        &self,
        session_id: &str,
        worktree_id: &str,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE sessions SET worktree_id = ?, updated_at = ? WHERE id = ?")
            .bind(worktree_id)
            .bind(now)
            .bind(session_id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Get sessions bound to a worktree.
    pub async fn get_worktree_sessions(
        &self,
        worktree_id: &str,
    ) -> Result<Vec<Session>, DatabaseError> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT * FROM sessions WHERE worktree_id = ? ORDER BY updated_at DESC",
        )
        .bind(worktree_id)
        .fetch_all(self.pool())
        .await?;

        Ok(sessions)
    }

    // =========================================================================
    // Compaction queries
    // =========================================================================

    /// Count messages for a session.
    pub async fn count_messages(&self, session_id: &str) -> Result<i64, DatabaseError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(self.pool())
            .await?;
        Ok(row.0)
    }

    /// Delete messages at or below a sequence threshold.
    /// Returns number of deleted messages.
    pub async fn delete_messages_before_sequence(
        &self,
        session_id: &str,
        sequence: i64,
    ) -> Result<u64, DatabaseError> {
        let result = sqlx::query("DELETE FROM messages WHERE session_id = ? AND sequence <= ?")
            .bind(session_id)
            .bind(sequence)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }

    /// Update the compaction sequence marker on a session.
    pub async fn update_compaction_sequence(
        &self,
        session_id: &str,
        sequence: i64,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();
        sqlx::query("UPDATE sessions SET compaction_sequence = ?, updated_at = ? WHERE id = ?")
            .bind(sequence)
            .bind(now)
            .bind(session_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Get the maximum sequence number for a session's messages.
    pub async fn max_message_sequence(&self, session_id: &str) -> Result<i64, DatabaseError> {
        let row: (Option<i64>,) =
            sqlx::query_as("SELECT MAX(sequence) FROM messages WHERE session_id = ?")
                .bind(session_id)
                .fetch_one(self.pool())
                .await?;
        Ok(row.0.unwrap_or(0))
    }

    // =========================================================================
    // Input lock queries
    // =========================================================================

    /// Acquire input lock for a client on a session.
    /// Returns the previous lock holder (if any).
    ///
    /// Uses a transaction to atomically read the previous holder and set the new one,
    /// preventing race conditions between concurrent lock requests.
    pub async fn acquire_input_lock(
        &self,
        session_id: &str,
        client_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let mut tx = self.pool().begin().await?;

        let previous: Option<String> =
            sqlx::query_scalar("SELECT input_lock_client FROM sessions WHERE id = ?")
                .bind(session_id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();

        let now = unix_timestamp();
        sqlx::query("UPDATE sessions SET input_lock_client = ?, updated_at = ? WHERE id = ?")
            .bind(client_id)
            .bind(now)
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        // Update client records
        if let Some(ref prev) = previous {
            sqlx::query("UPDATE connected_clients SET has_input_lock = 0 WHERE client_id = ?")
                .bind(prev)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE connected_clients SET has_input_lock = 1 WHERE client_id = ?")
            .bind(client_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(previous)
    }

    /// Release input lock for a session.
    pub async fn release_input_lock(&self, session_id: &str) -> Result<(), DatabaseError> {
        let session = self.get_session(session_id).await?;

        if let Some(ref holder) = session.input_lock_client {
            sqlx::query("UPDATE connected_clients SET has_input_lock = 0 WHERE client_id = ?")
                .bind(holder)
                .execute(self.pool())
                .await?;
        }

        let now = unix_timestamp();
        sqlx::query("UPDATE sessions SET input_lock_client = NULL, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(session_id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    // =========================================================================
    // Client queries
    // =========================================================================

    /// Register a connected client.
    pub async fn register_client(
        &self,
        client_id: &str,
        client_type: &str,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT OR REPLACE INTO connected_clients (client_id, client_type, connected_at, last_heartbeat) VALUES (?, ?, ?, ?)",
        )
        .bind(client_id)
        .bind(client_type)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Update client heartbeat.
    pub async fn update_client_heartbeat(&self, client_id: &str) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE connected_clients SET last_heartbeat = ? WHERE client_id = ?")
            .bind(now)
            .bind(client_id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Remove stale clients.
    pub async fn remove_stale_clients(&self, max_age_secs: i64) -> Result<u64, DatabaseError> {
        let cutoff = unix_timestamp() - max_age_secs;

        let result = sqlx::query("DELETE FROM connected_clients WHERE last_heartbeat < ?")
            .bind(cutoff)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected())
    }

    /// Unregister a client.
    pub async fn unregister_client(&self, client_id: &str) -> Result<(), DatabaseError> {
        sqlx::query("DELETE FROM connected_clients WHERE client_id = ?")
            .bind(client_id)
            .execute(self.pool())
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_get_session() {
        let db = Database::open_in_memory().await.unwrap();

        let session = db
            .create_session("test-123", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();

        assert_eq!(session.id, "test-123");
        assert_eq!(session.model, "claude-sonnet-4");
        assert_eq!(session.status, "idle");
    }

    #[tokio::test]
    async fn create_and_get_worktree() {
        let db = Database::open_in_memory().await.unwrap();

        let wt = db
            .create_worktree(
                "wt-1",
                "feature-x",
                "/repo/wt-1",
                "feature-x",
                "/repo",
                None,
            )
            .await
            .unwrap();

        assert_eq!(wt.id, "wt-1");
        assert_eq!(wt.name, "feature-x");
        assert_eq!(wt.branch, "feature-x");
        assert_eq!(wt.repo_path, "/repo");
        assert!(wt.setup_script.is_none());
    }

    #[tokio::test]
    async fn worktree_with_setup_script() {
        let db = Database::open_in_memory().await.unwrap();

        let wt = db
            .create_worktree(
                "wt-1",
                "feature-x",
                "/repo/wt-1",
                "feature-x",
                "/repo",
                Some("npm install"),
            )
            .await
            .unwrap();

        assert_eq!(wt.setup_script.as_deref(), Some("npm install"));
    }

    #[tokio::test]
    async fn list_worktrees_by_repo() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_worktree("wt-1", "a", "/repo-a/wt-1", "a", "/repo-a", None)
            .await
            .unwrap();
        db.create_worktree("wt-2", "b", "/repo-b/wt-2", "b", "/repo-b", None)
            .await
            .unwrap();
        db.create_worktree("wt-3", "c", "/repo-a/wt-3", "c", "/repo-a", None)
            .await
            .unwrap();

        assert_eq!(db.list_worktrees(Some("/repo-a")).await.unwrap().len(), 2);
        assert_eq!(db.list_worktrees(Some("/repo-b")).await.unwrap().len(), 1);
        assert_eq!(db.list_worktrees(None).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn remove_worktree_clears_session_binding() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_worktree("wt-1", "feat", "/repo/wt-1", "feat", "/repo", None)
            .await
            .unwrap();
        db.create_session("s1", "claude-sonnet-4", "/repo/wt-1")
            .await
            .unwrap();
        db.bind_session_to_worktree("s1", "wt-1").await.unwrap();

        // Verify binding
        let s = db.get_session("s1").await.unwrap();
        assert_eq!(s.worktree_id.as_deref(), Some("wt-1"));

        // Remove worktree
        assert!(db.remove_worktree("wt-1").await.unwrap());

        // Session should have worktree_id cleared
        let s = db.get_session("s1").await.unwrap();
        assert!(s.worktree_id.is_none());

        // Worktree should be gone
        assert!(db.get_worktree("wt-1").await.is_err());
    }

    #[tokio::test]
    async fn remove_nonexistent_worktree_returns_false() {
        let db = Database::open_in_memory().await.unwrap();
        assert!(!db.remove_worktree("nope").await.unwrap());
    }

    #[tokio::test]
    async fn bind_session_to_worktree_and_query() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_worktree("wt-1", "feat", "/repo/wt-1", "feat", "/repo", None)
            .await
            .unwrap();
        db.create_session("s1", "claude-sonnet-4", "/repo/wt-1")
            .await
            .unwrap();
        db.create_session("s2", "claude-sonnet-4", "/repo/wt-1")
            .await
            .unwrap();

        db.bind_session_to_worktree("s1", "wt-1").await.unwrap();
        db.bind_session_to_worktree("s2", "wt-1").await.unwrap();

        let sessions = db.get_worktree_sessions("wt-1").await.unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn insert_and_get_messages() {
        let db = Database::open_in_memory().await.unwrap();

        db.create_session("test-123", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();

        db.insert_message("test-123", 1, "system", r#"{"type":"system"}"#)
            .await
            .unwrap();

        let messages = db.get_messages_from_sequence("test-123", 0).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sequence, 1);
    }

    #[tokio::test]
    async fn count_and_compact_messages() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_session("s1", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();

        for i in 1..=10 {
            db.insert_message("s1", i, "stream_event", "payload")
                .await
                .unwrap();
        }

        assert_eq!(db.count_messages("s1").await.unwrap(), 10);
        assert_eq!(db.max_message_sequence("s1").await.unwrap(), 10);

        // Delete messages 1-5
        let deleted = db.delete_messages_before_sequence("s1", 5).await.unwrap();
        assert_eq!(deleted, 5);
        assert_eq!(db.count_messages("s1").await.unwrap(), 5);

        // Remaining messages start at sequence 6
        let msgs = db.get_messages_from_sequence("s1", 0).await.unwrap();
        assert_eq!(msgs[0].sequence, 6);

        // Update compaction marker
        db.update_compaction_sequence("s1", 5).await.unwrap();
        let s = db.get_session("s1").await.unwrap();
        assert_eq!(s.compaction_sequence, 5);
    }

    #[tokio::test]
    async fn acquire_and_release_input_lock() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_session("s1", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();
        db.register_client("c1", "cli").await.unwrap();
        db.register_client("c2", "cli").await.unwrap();

        // First acquire: no previous holder
        let prev = db.acquire_input_lock("s1", "c1").await.unwrap();
        assert!(prev.is_none());

        let s = db.get_session("s1").await.unwrap();
        assert_eq!(s.input_lock_client.as_deref(), Some("c1"));

        // Second acquire by different client: previous holder returned
        let prev = db.acquire_input_lock("s1", "c2").await.unwrap();
        assert_eq!(prev.as_deref(), Some("c1"));

        let s = db.get_session("s1").await.unwrap();
        assert_eq!(s.input_lock_client.as_deref(), Some("c2"));

        // Release
        db.release_input_lock("s1").await.unwrap();
        let s = db.get_session("s1").await.unwrap();
        assert!(s.input_lock_client.is_none());
    }

    #[tokio::test]
    async fn max_sequence_empty_session() {
        let db = Database::open_in_memory().await.unwrap();
        db.create_session("s1", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();
        assert_eq!(db.max_message_sequence("s1").await.unwrap(), 0);
    }
}
