//! Database queries for BetCode daemon.

use std::time::{SystemTime, UNIX_EPOCH};

use super::db::{Database, DatabaseError};
use super::models::*;

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
            r#"
            INSERT INTO sessions (id, model, working_directory, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
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
            .ok_or_else(|| DatabaseError::NotFound(format!("Session {}", id)))
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

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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
}
