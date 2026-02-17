//! Device token queries for push notification support.
//!
//! These queries are only used when the `push-notifications` feature is enabled,
//! but the table is always created by the migration so the queries themselves
//! compile unconditionally.

use betcode_core::db::unix_timestamp;

use super::db::{DatabaseError, RelayDatabase};
use super::models::DeviceToken;

impl RelayDatabase {
    // =========================================================================
    // Device token queries
    // =========================================================================

    /// Register or update a device token.
    ///
    /// If a record with the same `device_token` already exists, its `user_id`,
    /// `platform`, and `created_at` are updated (upsert).
    pub async fn upsert_device_token(
        &self,
        id: &str,
        user_id: &str,
        device_token: &str,
        platform: &str,
    ) -> Result<(), DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO device_tokens (id, user_id, device_token, platform, created_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(device_token) DO UPDATE SET user_id = ?, platform = ?, created_at = ?",
        )
        .bind(id)
        .bind(user_id)
        .bind(device_token)
        .bind(platform)
        .bind(now)
        .bind(user_id)
        .bind(platform)
        .bind(now)
        .execute(self.pool())
        .await?;

        Ok(())
    }

    /// Delete a device token by its token string.
    ///
    /// Returns `true` if a row was deleted, `false` if the token was not found.
    pub async fn delete_device_token(&self, device_token: &str) -> Result<bool, DatabaseError> {
        let result = sqlx::query("DELETE FROM device_tokens WHERE device_token = ?")
            .bind(device_token)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get all device tokens for a given user.
    pub async fn get_device_tokens_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<DeviceToken>, DatabaseError> {
        let tokens = sqlx::query_as::<_, DeviceToken>(
            "SELECT * FROM device_tokens WHERE user_id = ? ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;

        Ok(tokens)
    }

    /// Get a device token by its token string.
    pub async fn get_device_token(
        &self,
        device_token: &str,
    ) -> Result<Option<DeviceToken>, DatabaseError> {
        let token =
            sqlx::query_as::<_, DeviceToken>("SELECT * FROM device_tokens WHERE device_token = ?")
                .bind(device_token)
                .fetch_optional(self.pool())
                .await?;

        Ok(token)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn test_db() -> RelayDatabase {
        RelayDatabase::open_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn upsert_and_get_device_token() {
        let db = test_db().await;

        db.upsert_device_token("dt-1", "user-1", "token-abc", "android")
            .await
            .unwrap();

        let token = db.get_device_token("token-abc").await.unwrap().unwrap();
        assert_eq!(token.user_id, "user-1");
        assert_eq!(token.device_token, "token-abc");
        assert_eq!(token.platform, "android");
    }

    #[tokio::test]
    async fn upsert_updates_existing_token() {
        let db = test_db().await;

        db.upsert_device_token("dt-1", "user-1", "token-abc", "android")
            .await
            .unwrap();

        // Upsert same device_token with different user and platform
        db.upsert_device_token("dt-2", "user-2", "token-abc", "ios")
            .await
            .unwrap();

        let token = db.get_device_token("token-abc").await.unwrap().unwrap();
        assert_eq!(token.user_id, "user-2");
        assert_eq!(token.platform, "ios");
    }

    #[tokio::test]
    async fn delete_device_token_existing() {
        let db = test_db().await;

        db.upsert_device_token("dt-1", "user-1", "token-abc", "android")
            .await
            .unwrap();

        let removed = db.delete_device_token("token-abc").await.unwrap();
        assert!(removed);

        let token = db.get_device_token("token-abc").await.unwrap();
        assert!(token.is_none());
    }

    #[tokio::test]
    async fn delete_device_token_nonexistent() {
        let db = test_db().await;

        let removed = db.delete_device_token("nonexistent").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn get_device_tokens_for_user() {
        let db = test_db().await;

        db.upsert_device_token("dt-1", "user-1", "token-a", "android")
            .await
            .unwrap();
        db.upsert_device_token("dt-2", "user-1", "token-b", "ios")
            .await
            .unwrap();
        db.upsert_device_token("dt-3", "user-2", "token-c", "android")
            .await
            .unwrap();

        let tokens = db.get_device_tokens_for_user("user-1").await.unwrap();
        assert_eq!(tokens.len(), 2);
        assert!(tokens.iter().all(|t| t.user_id == "user-1"));

        let tokens_u2 = db.get_device_tokens_for_user("user-2").await.unwrap();
        assert_eq!(tokens_u2.len(), 1);
    }

    #[tokio::test]
    async fn get_device_tokens_for_user_empty() {
        let db = test_db().await;

        let tokens = db.get_device_tokens_for_user("nobody").await.unwrap();
        assert!(tokens.is_empty());
    }

    #[tokio::test]
    async fn get_device_token_not_found() {
        let db = test_db().await;

        let token = db.get_device_token("missing").await.unwrap();
        assert!(token.is_none());
    }
}
