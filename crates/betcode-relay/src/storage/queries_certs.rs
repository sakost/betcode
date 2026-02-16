//! Additional certificate queries for mTLS validation.
//!
//! Core certificate CRUD lives in `queries_buffer.rs`; this module adds
//! serial-number-based lookups needed for mTLS peer certificate validation.

use super::db::{DatabaseError, RelayDatabase};
use super::models::Certificate;

impl RelayDatabase {
    /// Look up a certificate by its serial number.
    pub async fn get_certificate_by_serial(
        &self,
        serial_number: &str,
    ) -> Result<Option<Certificate>, DatabaseError> {
        let cert =
            sqlx::query_as::<_, Certificate>("SELECT * FROM certificates WHERE serial_number = ?")
                .bind(serial_number)
                .fetch_optional(self.pool())
                .await?;

        Ok(cert)
    }

    /// Check whether a certificate serial is revoked.
    ///
    /// Returns `false` if the serial is not found (unknown certs are not
    /// considered revoked -- chain validation is handled by the TLS layer).
    pub async fn is_certificate_revoked(&self, serial_number: &str) -> Result<bool, DatabaseError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT revoked FROM certificates WHERE serial_number = ?")
                .bind(serial_number)
                .fetch_optional(self.pool())
                .await?;

        Ok(row.is_some_and(|(revoked,)| revoked != 0))
    }

    /// Revoke a certificate by its serial number.
    pub async fn revoke_certificate_by_serial(
        &self,
        serial_number: &str,
    ) -> Result<bool, DatabaseError> {
        let result = sqlx::query("UPDATE certificates SET revoked = 1 WHERE serial_number = ?")
            .bind(serial_number)
            .execute(self.pool())
            .await?;

        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_pass_by_value
)]
mod tests {
    use crate::storage::{CertificateParams, RelayDatabase};

    async fn test_db() -> RelayDatabase {
        let db = RelayDatabase::open_in_memory().await.unwrap();
        // Set up user + machine so FK constraints pass
        db.create_user("u1", "alice", "alice@test.com", "hash")
            .await
            .unwrap();
        db.create_machine("machine-1", "laptop", "u1", "{}")
            .await
            .unwrap();
        db.create_machine("machine-2", "desktop", "u1", "{}")
            .await
            .unwrap();
        db.create_machine("machine-3", "server", "u1", "{}")
            .await
            .unwrap();
        db
    }

    fn sample_cert_params<'a>(
        id: &'a str,
        serial: &'a str,
        machine_id: &'a str,
    ) -> CertificateParams<'a> {
        CertificateParams {
            id,
            machine_id: Some(machine_id),
            subject_cn: machine_id,
            serial_number: serial,
            not_before: 1000,
            not_after: 2000,
            pem_cert: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----",
        }
    }

    #[tokio::test]
    async fn get_certificate_by_serial_found() {
        let db = test_db().await;
        let params = sample_cert_params("cert-1", "AABB01", "machine-1");
        db.create_certificate(&params).await.unwrap();

        let found = db.get_certificate_by_serial("AABB01").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "cert-1");
    }

    #[tokio::test]
    async fn get_certificate_by_serial_not_found() {
        let db = test_db().await;
        let missing = db.get_certificate_by_serial("NONEXISTENT").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn is_certificate_revoked_false_for_active() {
        let db = test_db().await;
        let params = sample_cert_params("cert-2", "CC0022", "machine-2");
        db.create_certificate(&params).await.unwrap();

        assert!(!db.is_certificate_revoked("CC0022").await.unwrap());
    }

    #[tokio::test]
    async fn is_certificate_revoked_true_after_revocation() {
        let db = test_db().await;
        let params = sample_cert_params("cert-3", "DD0033", "machine-3");
        db.create_certificate(&params).await.unwrap();

        let revoked = db.revoke_certificate_by_serial("DD0033").await.unwrap();
        assert!(revoked);
        assert!(db.is_certificate_revoked("DD0033").await.unwrap());
    }

    #[tokio::test]
    async fn is_certificate_revoked_false_for_unknown_serial() {
        let db = test_db().await;
        assert!(!db.is_certificate_revoked("UNKNOWN").await.unwrap());
    }

    #[tokio::test]
    async fn revoke_by_serial_returns_false_for_unknown() {
        let db = test_db().await;
        assert!(!db.revoke_certificate_by_serial("UNKNOWN").await.unwrap());
    }
}
