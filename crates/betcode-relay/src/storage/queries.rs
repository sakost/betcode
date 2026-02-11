//! Database queries for `BetCode` relay server.

use betcode_core::db::unix_timestamp;

use super::db::{DatabaseError, RelayDatabase};
use super::models::{Machine, Token, User};

impl RelayDatabase {
    // =========================================================================
    // User queries
    // =========================================================================

    /// Create a new user.
    pub async fn create_user(
        &self,
        id: &str,
        username: &str,
        email: &str,
        password_hash: &str,
    ) -> Result<User, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(username)
        .bind(email)
        .bind(password_hash)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_user(id).await
    }

    /// Get a user by ID.
    pub async fn get_user(&self, id: &str) -> Result<User, DatabaseError> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("User {id}")))
    }

    /// Get a user by username.
    pub async fn get_user_by_username(&self, username: &str) -> Result<User, DatabaseError> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("User with username {username}")))
    }

    // =========================================================================
    // Token queries
    // =========================================================================

    /// Store a refresh token.
    pub async fn create_token(
        &self,
        id: &str,
        user_id: &str,
        token_hash: &str,
        expires_at: i64,
    ) -> Result<Token, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO tokens (id, user_id, token_hash, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_token(id).await
    }

    /// Get a token by ID.
    pub async fn get_token(&self, id: &str) -> Result<Token, DatabaseError> {
        sqlx::query_as::<_, Token>("SELECT * FROM tokens WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Token {id}")))
    }

    /// Find a valid (non-revoked, non-expired) token by hash.
    pub async fn get_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Token>, DatabaseError> {
        let now = unix_timestamp();

        let token = sqlx::query_as::<_, Token>(
            "SELECT * FROM tokens WHERE token_hash = ? AND revoked = 0 AND expires_at > ?",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(self.pool())
        .await?;

        Ok(token)
    }

    /// Revoke a token by ID.
    pub async fn revoke_token(&self, id: &str) -> Result<bool, DatabaseError> {
        let result = sqlx::query("UPDATE tokens SET revoked = 1 WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Revoke all tokens for a user.
    pub async fn revoke_user_tokens(&self, user_id: &str) -> Result<u64, DatabaseError> {
        let result = sqlx::query("UPDATE tokens SET revoked = 1 WHERE user_id = ?")
            .bind(user_id)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected())
    }

    // =========================================================================
    // Machine queries
    // =========================================================================

    /// Register a machine.
    pub async fn create_machine(
        &self,
        id: &str,
        name: &str,
        owner_id: &str,
        metadata: &str,
    ) -> Result<Machine, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO machines (id, name, owner_id, registered_at, last_seen, metadata) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(owner_id)
        .bind(now)
        .bind(now)
        .bind(metadata)
        .execute(self.pool())
        .await?;

        self.get_machine(id).await
    }

    /// Get a machine by ID.
    pub async fn get_machine(&self, id: &str) -> Result<Machine, DatabaseError> {
        sqlx::query_as::<_, Machine>("SELECT * FROM machines WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Machine {id}")))
    }

    /// List machines for an owner.
    pub async fn list_machines(
        &self,
        owner_id: &str,
        status_filter: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Machine>, DatabaseError> {
        let machines = if let Some(status) = status_filter {
            sqlx::query_as::<_, Machine>(
                "SELECT * FROM machines WHERE owner_id = ? AND status = ? ORDER BY last_seen DESC LIMIT ? OFFSET ?",
            )
            .bind(owner_id)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool())
            .await?
        } else {
            sqlx::query_as::<_, Machine>(
                "SELECT * FROM machines WHERE owner_id = ? ORDER BY last_seen DESC LIMIT ? OFFSET ?",
            )
            .bind(owner_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(self.pool())
            .await?
        };

        Ok(machines)
    }

    /// Update machine status.
    pub async fn update_machine_status(&self, id: &str, status: &str) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE machines SET status = ?, last_seen = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Update machine `last_seen` timestamp.
    pub async fn touch_machine(&self, id: &str) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query("UPDATE machines SET last_seen = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(())
    }

    /// Remove a machine.
    pub async fn remove_machine(&self, id: &str) -> Result<bool, DatabaseError> {
        let result = sqlx::query("DELETE FROM machines WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Update a machine's identity public key.
    pub async fn update_machine_identity_pubkey(
        &self,
        id: &str,
        pubkey: &[u8],
    ) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE machines SET identity_pubkey = ? WHERE id = ?")
            .bind(pubkey)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Get a machine's identity public key.
    pub async fn get_machine_identity_pubkey(
        &self,
        id: &str,
    ) -> Result<Option<Vec<u8>>, DatabaseError> {
        let machine = self.get_machine(id).await?;
        Ok(machine.identity_pubkey)
    }

    /// Count machines for an owner.
    pub async fn count_machines(
        &self,
        owner_id: &str,
        status_filter: Option<&str>,
    ) -> Result<i64, DatabaseError> {
        let row: (i64,) = if let Some(status) = status_filter {
            sqlx::query_as("SELECT COUNT(*) FROM machines WHERE owner_id = ? AND status = ?")
                .bind(owner_id)
                .bind(status)
                .fetch_one(self.pool())
                .await?
        } else {
            sqlx::query_as("SELECT COUNT(*) FROM machines WHERE owner_id = ?")
                .bind(owner_id)
                .fetch_one(self.pool())
                .await?
        };

        Ok(row.0)
    }
}
