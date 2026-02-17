//! Client certificate provisioning for mTLS daemon identity.
//!
//! During setup, after the user authenticates with the relay via JWT,
//! this module generates a client certificate and saves it to the
//! daemon's config directory for mTLS connections.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub use betcode_crypto::certs::{CERTS_SUBDIR, CertMetadata, DEFAULT_VALIDITY_DAYS};
use betcode_crypto::certs::{METADATA_FILENAME, read_metadata as crypto_read_metadata};

/// Default client certificate filename.
const CLIENT_CERT_FILENAME: &str = "client.pem";
/// Default client key filename.
const CLIENT_KEY_FILENAME: &str = "client-key.pem";
/// Default CA certificate filename.
const CA_CERT_FILENAME: &str = "ca.pem";

/// Paths to the provisioned certificate files.
#[derive(Debug, Clone)]
pub struct CertPaths {
    /// Path to the client certificate PEM file.
    pub client_cert: PathBuf,
    /// Path to the client private key PEM file.
    pub client_key: PathBuf,
    /// Path to the CA certificate PEM file.
    pub ca_cert: PathBuf,
}

/// Resolve the default certificate directory (`$HOME/.betcode/certs/`).
pub fn default_certs_dir() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(CERTS_SUBDIR))
}

/// Provision a client certificate for the given machine ID.
///
/// Generates a self-signed CA and a client certificate signed by that CA,
/// then writes all PEM files and a metadata JSON to the specified directory.
///
/// Returns the paths to the written files.
pub fn provision_client_cert(certs_dir: &Path, machine_id: &str) -> Result<CertPaths> {
    fs::create_dir_all(certs_dir)
        .with_context(|| format!("Failed to create certs directory: {}", certs_dir.display()))?;

    tracing::info!(
        machine_id = %machine_id,
        certs_dir = %certs_dir.display(),
        "Generating client certificate for mTLS"
    );

    let ca = betcode_crypto::certs::generate_ca("BetCode")
        .map_err(|e| anyhow::anyhow!("CA generation failed: {e}"))?;

    let bundle = betcode_crypto::certs::generate_client_cert(&ca, machine_id)
        .map_err(|e| anyhow::anyhow!("Client cert generation failed: {e}"))?;

    let paths = CertPaths {
        client_cert: certs_dir.join(CLIENT_CERT_FILENAME),
        client_key: certs_dir.join(CLIENT_KEY_FILENAME),
        ca_cert: certs_dir.join(CA_CERT_FILENAME),
    };

    write_pem_file(&paths.client_cert, &bundle.cert_pem, "client certificate")?;
    write_pem_file(&paths.client_key, &bundle.key_pem, "client key")?;
    write_pem_file(&paths.ca_cert, &bundle.ca_cert_pem, "CA certificate")?;

    // Write metadata for expiry tracking
    let metadata = CertMetadata::now(machine_id.to_string(), DEFAULT_VALIDITY_DAYS);
    write_metadata(certs_dir, &metadata)?;

    // Restrict key file permissions on unix
    #[cfg(unix)]
    restrict_key_permissions(&paths.client_key)?;

    tracing::info!(
        client_cert = %paths.client_cert.display(),
        client_key = %paths.client_key.display(),
        ca_cert = %paths.ca_cert.display(),
        "Client certificate provisioned"
    );

    Ok(paths)
}

/// Write a PEM file with a human-readable description for error context.
fn write_pem_file(path: &Path, content: &str, description: &str) -> Result<()> {
    fs::write(path, content)
        .with_context(|| format!("Failed to write {description}: {}", path.display()))
}

/// Write certificate metadata JSON file.
fn write_metadata(certs_dir: &Path, metadata: &CertMetadata) -> Result<()> {
    let path = certs_dir.join(METADATA_FILENAME);
    let json = serde_json::to_string_pretty(metadata)
        .context("Failed to serialize cert metadata to JSON")?;
    fs::write(&path, json)
        .with_context(|| format!("Failed to write cert metadata: {}", path.display()))
}

/// Read certificate metadata from the certs directory.
///
/// Returns `None` if the file does not exist or cannot be parsed.
pub fn read_metadata(certs_dir: &Path) -> Option<CertMetadata> {
    crypto_read_metadata(certs_dir)
}

/// Restrict file permissions to owner-only read/write (0600) on unix.
#[cfg(unix)]
fn restrict_key_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms)
        .with_context(|| format!("Failed to set permissions on {}", path.display()))
}

/// Check whether client certificate files already exist in the given directory.
pub fn certs_exist(certs_dir: &Path) -> bool {
    certs_dir.join(CLIENT_CERT_FILENAME).exists()
        && certs_dir.join(CLIENT_KEY_FILENAME).exists()
        && certs_dir.join(CA_CERT_FILENAME).exists()
}

/// Return the standard cert paths for a given directory, without checking existence.
pub fn cert_paths_in(certs_dir: &Path) -> CertPaths {
    CertPaths {
        client_cert: certs_dir.join(CLIENT_CERT_FILENAME),
        client_key: certs_dir.join(CLIENT_KEY_FILENAME),
        ca_cert: certs_dir.join(CA_CERT_FILENAME),
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn provision_creates_cert_files() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path().join("certs");

        let paths = provision_client_cert(&certs_dir, "test-machine-42").unwrap();

        assert!(paths.client_cert.exists(), "client cert should exist");
        assert!(paths.client_key.exists(), "client key should exist");
        assert!(paths.ca_cert.exists(), "CA cert should exist");

        let cert_pem = fs::read_to_string(&paths.client_cert).unwrap();
        let key_pem = fs::read_to_string(&paths.client_key).unwrap();
        let ca_pem = fs::read_to_string(&paths.ca_cert).unwrap();

        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(ca_pem.contains("BEGIN CERTIFICATE"));
        // Client cert and CA cert should differ
        assert_ne!(cert_pem, ca_pem);
    }

    #[test]
    fn provision_writes_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path().join("certs");

        provision_client_cert(&certs_dir, "meta-test-machine").unwrap();

        let metadata = read_metadata(&certs_dir);
        assert!(metadata.is_some(), "metadata should be written");

        let meta = metadata.unwrap();
        assert_eq!(meta.machine_id, "meta-test-machine");
        assert_eq!(meta.validity_days, DEFAULT_VALIDITY_DAYS);
        assert!(meta.generated_at_secs > 0);
    }

    #[test]
    fn certs_exist_returns_false_for_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!certs_exist(dir.path()));
    }

    #[test]
    fn certs_exist_returns_true_after_provision() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path().join("certs");

        provision_client_cert(&certs_dir, "machine-x").unwrap();
        assert!(certs_exist(&certs_dir));
    }

    #[test]
    fn cert_paths_in_returns_expected_paths() {
        let dir = Path::new("/tmp/test-certs");
        let paths = cert_paths_in(dir);

        assert_eq!(paths.client_cert, dir.join("client.pem"));
        assert_eq!(paths.client_key, dir.join("client-key.pem"));
        assert_eq!(paths.ca_cert, dir.join("ca.pem"));
    }

    #[cfg(unix)]
    #[test]
    fn key_file_has_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path().join("certs");

        let paths = provision_client_cert(&certs_dir, "perm-test").unwrap();

        let meta = fs::metadata(&paths.client_key).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "key file should be owner-only (0600)");
    }

    #[test]
    fn default_certs_dir_is_under_home() {
        // This may fail in CI without HOME, but that's ok
        if let Ok(dir) = default_certs_dir() {
            let dir_str = dir.to_string_lossy();
            assert!(
                dir_str.contains(".betcode/certs"),
                "expected .betcode/certs in path, got: {dir_str}"
            );
        }
    }

    #[test]
    fn metadata_expires_within_days_future_cert() {
        let meta = CertMetadata {
            machine_id: "test".to_string(),
            generated_at_secs: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            validity_days: 365,
        };
        // A freshly generated cert should NOT expire within 30 days
        assert!(!meta.expires_within_days(30));
        // But it SHOULD expire within 366 days (past the 365-day validity)
        assert!(meta.expires_within_days(366));
    }

    #[test]
    fn metadata_expires_within_days_expired_cert() {
        let meta = CertMetadata {
            machine_id: "test".to_string(),
            // Generated 400 days ago
            generated_at_secs: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .saturating_sub(400 * 86400),
            validity_days: 365,
        };
        // Already expired, so it should expire within 0 days
        assert!(meta.expires_within_days(0));
        assert!(meta.expires_within_days(30));
    }

    #[test]
    fn read_metadata_returns_none_for_missing_dir() {
        let dir = Path::new("/nonexistent/path/to/certs");
        assert!(read_metadata(dir).is_none());
    }
}
