//! Buffer and certificate queries for BetCode relay server.

use super::db::{DatabaseError, RelayDatabase};
use super::models::*;
use super::queries::unix_timestamp;

impl RelayDatabase {
    // =========================================================================
    // Message buffer queries
    // =========================================================================

    /// Buffer a request for an offline machine.
    #[allow(clippy::too_many_arguments)]
    pub async fn buffer_message(
        &self,
        machine_id: &str,
        request_id: &str,
        method: &str,
        payload: &[u8],
        metadata: &str,
        priority: i64,
        ttl_secs: i64,
    ) -> Result<i64, DatabaseError> {
        let now = unix_timestamp();
        let expires_at = now + ttl_secs;

        let result = sqlx::query(
            "INSERT INTO message_buffer (machine_id, request_id, method, payload, metadata, priority, expires_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(machine_id)
        .bind(request_id)
        .bind(method)
        .bind(payload)
        .bind(metadata)
        .bind(priority)
        .bind(expires_at)
        .bind(now)
        .execute(self.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Drain buffered messages for a machine (priority DESC, created_at ASC).
    pub async fn drain_buffer(
        &self,
        machine_id: &str,
    ) -> Result<Vec<BufferedMessage>, DatabaseError> {
        let messages = sqlx::query_as::<_, BufferedMessage>(
            "SELECT * FROM message_buffer WHERE machine_id = ? AND expires_at > ? ORDER BY priority DESC, created_at ASC",
        )
        .bind(machine_id)
        .bind(unix_timestamp())
        .fetch_all(self.pool())
        .await?;

        sqlx::query("DELETE FROM message_buffer WHERE machine_id = ?")
            .bind(machine_id)
            .execute(self.pool())
            .await?;

        Ok(messages)
    }

    /// Remove expired buffered messages.
    pub async fn cleanup_expired_buffer(&self) -> Result<u64, DatabaseError> {
        let now = unix_timestamp();

        let result = sqlx::query("DELETE FROM message_buffer WHERE expires_at <= ?")
            .bind(now)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected())
    }

    /// Count buffered messages for a machine.
    pub async fn count_buffered_messages(&self, machine_id: &str) -> Result<i64, DatabaseError> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM message_buffer WHERE machine_id = ?")
                .bind(machine_id)
                .fetch_one(self.pool())
                .await?;

        Ok(row.0)
    }

    // =========================================================================
    // Certificate queries
    // =========================================================================

    /// Store a certificate record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_certificate(
        &self,
        id: &str,
        machine_id: Option<&str>,
        subject_cn: &str,
        serial_number: &str,
        not_before: i64,
        not_after: i64,
        pem_cert: &str,
    ) -> Result<Certificate, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO certificates (id, machine_id, subject_cn, serial_number, not_before, not_after, pem_cert, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(machine_id)
        .bind(subject_cn)
        .bind(serial_number)
        .bind(not_before)
        .bind(not_after)
        .bind(pem_cert)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_certificate(id).await
    }

    /// Get a certificate by ID.
    pub async fn get_certificate(&self, id: &str) -> Result<Certificate, DatabaseError> {
        sqlx::query_as::<_, Certificate>("SELECT * FROM certificates WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Certificate {}", id)))
    }

    /// Get active certificates for a machine.
    pub async fn get_machine_certificates(
        &self,
        machine_id: &str,
    ) -> Result<Vec<Certificate>, DatabaseError> {
        let certs = sqlx::query_as::<_, Certificate>(
            "SELECT * FROM certificates WHERE machine_id = ? AND revoked = 0 ORDER BY created_at DESC",
        )
        .bind(machine_id)
        .fetch_all(self.pool())
        .await?;

        Ok(certs)
    }

    /// Revoke a certificate.
    pub async fn revoke_certificate(&self, id: &str) -> Result<bool, DatabaseError> {
        let result = sqlx::query("UPDATE certificates SET revoked = 1 WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
    }
}
