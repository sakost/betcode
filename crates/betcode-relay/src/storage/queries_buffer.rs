//! Buffer and certificate queries for BetCode relay server.

use super::db::{DatabaseError, RelayDatabase};
use super::models::*;
use betcode_core::db::unix_timestamp;

/// Parameters for buffering a message.
pub struct BufferMessageParams<'a> {
    pub machine_id: &'a str,
    pub request_id: &'a str,
    pub method: &'a str,
    pub payload: &'a [u8],
    pub metadata: &'a str,
    pub priority: i64,
    pub ttl_secs: i64,
}

/// Parameters for creating a certificate.
pub struct CertificateParams<'a> {
    pub id: &'a str,
    pub machine_id: Option<&'a str>,
    pub subject_cn: &'a str,
    pub serial_number: &'a str,
    pub not_before: i64,
    pub not_after: i64,
    pub pem_cert: &'a str,
}

impl RelayDatabase {
    // =========================================================================
    // Message buffer queries
    // =========================================================================

    /// Buffer a request for an offline machine.
    pub async fn buffer_message(
        &self,
        params: &BufferMessageParams<'_>,
    ) -> Result<i64, DatabaseError> {
        let now = unix_timestamp();
        let expires_at = now + params.ttl_secs;

        let result = sqlx::query(
            "INSERT INTO message_buffer (machine_id, request_id, method, payload, metadata, priority, expires_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(params.machine_id)
        .bind(params.request_id)
        .bind(params.method)
        .bind(params.payload)
        .bind(params.metadata)
        .bind(params.priority)
        .bind(expires_at)
        .bind(now)
        .execute(self.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Fetch buffered messages for a machine (priority DESC, created_at ASC).
    ///
    /// Messages are NOT deleted by this call. Use `delete_buffered_message` to
    /// remove each message after it has been successfully delivered.
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

        Ok(messages)
    }

    /// Delete a single buffered message by ID after successful delivery.
    pub async fn delete_buffered_message(&self, id: i64) -> Result<bool, DatabaseError> {
        let result = sqlx::query("DELETE FROM message_buffer WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
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
    pub async fn create_certificate(
        &self,
        params: &CertificateParams<'_>,
    ) -> Result<Certificate, DatabaseError> {
        let now = unix_timestamp();

        sqlx::query(
            "INSERT INTO certificates (id, machine_id, subject_cn, serial_number, not_before, not_after, pem_cert, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(params.id)
        .bind(params.machine_id)
        .bind(params.subject_cn)
        .bind(params.serial_number)
        .bind(params.not_before)
        .bind(params.not_after)
        .bind(params.pem_cert)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_certificate(params.id).await
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
