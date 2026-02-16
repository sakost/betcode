//! Certificate expiry monitoring and rotation for mTLS client certs.
//!
//! Provides a background task that checks cert expiry daily and initiates
//! renewal when the certificate is within 30 days of expiration. Also
//! exposes a function for immediate (forced) rotation.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{error, info, warn};

use betcode_crypto::certs::{
    CERTS_SUBDIR, CertMetadata, DEFAULT_VALIDITY_DAYS, read_metadata, write_metadata,
};

/// Number of days before expiry to trigger automatic renewal.
const RENEWAL_THRESHOLD_DAYS: u64 = 30;

/// Interval between expiry checks (24 hours).
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Result of a certificate rotation attempt.
#[derive(Debug)]
pub enum RotationResult {
    /// Certificate was successfully rotated.
    Rotated {
        /// Machine ID the new cert was issued for.
        machine_id: String,
    },
    /// No rotation needed (cert is not near expiry).
    NotNeeded,
    /// Rotation failed with the given error message.
    Failed(String),
    /// No certificate metadata found; cannot determine expiry.
    NoMetadata,
}

/// Resolve the default certs directory.
fn default_certs_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(CERTS_SUBDIR))
}

/// Perform certificate rotation for the given machine ID in the given certs directory.
///
/// Generates a new CA and client certificate, overwrites existing files,
/// and updates the metadata.
fn rotate_cert_in_dir(certs_dir: &Path, machine_id: &str) -> Result<(), String> {
    info!(
        machine_id = %machine_id,
        certs_dir = %certs_dir.display(),
        "Rotating client certificate"
    );

    let ca = betcode_crypto::certs::generate_ca("BetCode")
        .map_err(|e| format!("CA generation failed: {e}"))?;

    let bundle = betcode_crypto::certs::generate_client_cert(&ca, machine_id)
        .map_err(|e| format!("Client cert generation failed: {e}"))?;

    fs::write(certs_dir.join("client.pem"), &bundle.cert_pem)
        .map_err(|e| format!("Failed to write client cert: {e}"))?;
    fs::write(certs_dir.join("client-key.pem"), &bundle.key_pem)
        .map_err(|e| format!("Failed to write client key: {e}"))?;
    fs::write(certs_dir.join("ca.pem"), &bundle.ca_cert_pem)
        .map_err(|e| format!("Failed to write CA cert: {e}"))?;

    #[cfg(unix)]
    betcode_crypto::certs::restrict_key_permissions(&certs_dir.join("client-key.pem"))?;

    let metadata = CertMetadata::now(machine_id.to_string(), DEFAULT_VALIDITY_DAYS);
    write_metadata(certs_dir, &metadata)?;

    info!(
        machine_id = %machine_id,
        "Certificate rotation complete"
    );

    Ok(())
}

/// Check cert expiry and rotate if needed. Returns the rotation result.
pub fn check_and_rotate(certs_dir: &Path) -> RotationResult {
    let Some(metadata) = read_metadata(certs_dir) else {
        return RotationResult::NoMetadata;
    };

    if !metadata.expires_within_days(RENEWAL_THRESHOLD_DAYS) {
        return RotationResult::NotNeeded;
    }

    info!(
        machine_id = %metadata.machine_id,
        "Certificate expires within {} days, initiating rotation",
        RENEWAL_THRESHOLD_DAYS
    );

    match rotate_cert_in_dir(certs_dir, &metadata.machine_id) {
        Ok(()) => RotationResult::Rotated {
            machine_id: metadata.machine_id,
        },
        Err(e) => RotationResult::Failed(e),
    }
}

/// Force immediate certificate rotation regardless of expiry.
///
/// If no metadata file exists, uses `fallback_machine_id` as the machine ID.
pub fn force_rotate(certs_dir: &Path, fallback_machine_id: &str) -> RotationResult {
    let machine_id =
        read_metadata(certs_dir).map_or_else(|| fallback_machine_id.to_string(), |m| m.machine_id);

    match rotate_cert_in_dir(certs_dir, &machine_id) {
        Ok(()) => RotationResult::Rotated { machine_id },
        Err(e) => RotationResult::Failed(e),
    }
}

/// Spawn a background task that checks certificate expiry daily
/// and rotates if within the renewal threshold.
pub fn spawn_cert_monitor(
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(certs_dir) = default_certs_dir() else {
            warn!("Cannot determine home directory; cert expiry monitoring disabled");
            return;
        };

        if !certs_dir.exists() {
            info!(
                certs_dir = %certs_dir.display(),
                "No certs directory found; cert expiry monitoring disabled"
            );
            return;
        }

        info!("Certificate expiry monitor started (checking every 24h)");

        let mut timer = tokio::time::interval(CHECK_INTERVAL);
        // Skip the first immediate tick — let the daemon finish starting up
        timer.tick().await;

        loop {
            tokio::select! {
                _ = timer.tick() => {
                    match check_and_rotate(&certs_dir) {
                        RotationResult::Rotated { machine_id } => {
                            info!(
                                machine_id = %machine_id,
                                "Certificate automatically rotated (was near expiry)"
                            );
                        }
                        RotationResult::NotNeeded => {
                            info!("Certificate expiry check: OK (not near expiry)");
                        }
                        RotationResult::Failed(e) => {
                            error!(
                                error = %e,
                                "Automatic certificate rotation failed"
                            );
                        }
                        RotationResult::NoMetadata => {
                            warn!("No cert metadata found; skipping expiry check");
                        }
                    }
                }
                _ = shutdown.changed() => {
                    info!("Certificate expiry monitor shutting down");
                    return;
                }
            }
        }
    })
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_pass_by_value
)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn make_test_metadata(generated_days_ago: u64, validity_days: u64) -> CertMetadata {
        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        CertMetadata {
            machine_id: "test-machine".to_string(),
            generated_at_secs: now_secs.saturating_sub(generated_days_ago * 86400),
            validity_days,
        }
    }

    fn write_test_metadata(certs_dir: &Path, meta: &CertMetadata) {
        fs::create_dir_all(certs_dir).unwrap();
        let json = serde_json::to_string_pretty(meta).unwrap();
        fs::write(
            certs_dir.join(betcode_crypto::certs::METADATA_FILENAME),
            json,
        )
        .unwrap();
    }

    #[test]
    fn check_and_rotate_with_no_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let result = check_and_rotate(dir.path());
        assert!(matches!(result, RotationResult::NoMetadata));
    }

    #[test]
    fn check_and_rotate_not_needed() {
        let dir = tempfile::tempdir().unwrap();
        // Generated today, valid for 365 days — should NOT rotate
        let meta = make_test_metadata(0, 365);
        write_test_metadata(dir.path(), &meta);

        let result = check_and_rotate(dir.path());
        assert!(matches!(result, RotationResult::NotNeeded));
    }

    #[test]
    fn check_and_rotate_triggers_rotation() {
        let dir = tempfile::tempdir().unwrap();
        // Generated 340 days ago, valid for 365 days — only 25 days left, should rotate
        let meta = make_test_metadata(340, 365);
        write_test_metadata(dir.path(), &meta);

        let result = check_and_rotate(dir.path());
        assert!(
            matches!(result, RotationResult::Rotated { .. }),
            "expected Rotated, got {result:?}"
        );

        // Verify new cert files were written
        assert!(dir.path().join("client.pem").exists());
        assert!(dir.path().join("client-key.pem").exists());
        assert!(dir.path().join("ca.pem").exists());

        // Verify metadata was updated
        let new_meta = read_metadata(dir.path()).unwrap();
        assert!(new_meta.generated_at_secs > meta.generated_at_secs);
    }

    #[test]
    fn force_rotate_creates_new_certs() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path();

        let result = force_rotate(certs_dir, "force-test-machine");
        assert!(
            matches!(result, RotationResult::Rotated { ref machine_id } if machine_id == "force-test-machine"),
            "expected Rotated with correct machine_id, got {result:?}"
        );

        assert!(certs_dir.join("client.pem").exists());
        assert!(certs_dir.join("client-key.pem").exists());
        assert!(certs_dir.join("ca.pem").exists());

        let meta = read_metadata(certs_dir).unwrap();
        assert_eq!(meta.machine_id, "force-test-machine");
    }

    #[test]
    fn force_rotate_uses_existing_machine_id() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path();

        // Pre-populate metadata with a specific machine ID
        let meta = make_test_metadata(0, 365);
        write_test_metadata(certs_dir, &meta);

        let result = force_rotate(certs_dir, "fallback-id");
        assert!(
            matches!(result, RotationResult::Rotated { ref machine_id } if machine_id == "test-machine"),
            "should use existing machine_id from metadata, not fallback"
        );
    }

    #[test]
    fn metadata_expires_within_days_boundary() {
        // Exactly at the boundary: generated 335 days ago, valid for 365 days
        // That means 30 days left. expires_within_days(30) should be true
        // (because now + 30d >= generated + 365d)
        let meta = make_test_metadata(335, 365);
        assert!(meta.expires_within_days(30));

        // 334 days ago: 31 days left. expires_within_days(30) should be false
        let meta = make_test_metadata(334, 365);
        assert!(!meta.expires_within_days(30));
    }

    #[cfg(unix)]
    #[test]
    fn rotated_key_has_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        force_rotate(dir.path(), "perm-machine");

        let key_meta = fs::metadata(dir.path().join("client-key.pem")).unwrap();
        let mode = key_meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "rotated key should be owner-only");
    }
}
