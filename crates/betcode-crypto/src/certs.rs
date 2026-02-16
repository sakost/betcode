//! Client certificate generation for mTLS machine identity.
//!
//! Provides CA generation and client certificate signing for daemon
//! machine identity in mTLS connections to the relay.
//!
//! Requires the `certs` feature to be enabled.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};

/// Default cert directory name under the user's home directory.
pub const CERTS_SUBDIR: &str = ".betcode/certs";
/// Metadata filename for cert generation tracking.
pub const METADATA_FILENAME: &str = "cert-metadata.json";
/// Default validity period for generated certificates (365 days).
pub const DEFAULT_VALIDITY_DAYS: u64 = 365;

/// Certificate metadata stored alongside provisioned certificates.
///
/// Shared across `betcode-cli`, `betcode-daemon`, and `betcode-setup`
/// for consistent cert expiry tracking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CertMetadata {
    pub machine_id: String,
    pub generated_at_secs: u64,
    pub validity_days: u64,
}

impl CertMetadata {
    /// Create a new `CertMetadata` with the current timestamp.
    pub fn now(machine_id: String, validity_days: u64) -> Self {
        Self {
            machine_id,
            generated_at_secs: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            validity_days,
        }
    }

    /// Returns `true` if the certificate expires within the given number of days.
    pub fn expires_within_days(&self, days: u64) -> bool {
        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let expires_at_secs = self.generated_at_secs + self.validity_days * 86400;
        let threshold_secs = days * 86400;
        now_secs.saturating_add(threshold_secs) >= expires_at_secs
    }
}

/// Read certificate metadata from the certs directory.
///
/// Returns `None` if the file does not exist or cannot be parsed.
pub fn read_metadata(certs_dir: &Path) -> Option<CertMetadata> {
    let path = certs_dir.join(METADATA_FILENAME);
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Write certificate metadata JSON file.
///
/// Returns the stringified error on failure (suitable for both
/// `anyhow` and plain `String` error contexts).
pub fn write_metadata(certs_dir: &Path, metadata: &CertMetadata) -> Result<(), String> {
    let path = certs_dir.join(METADATA_FILENAME);
    let json = serde_json::to_string_pretty(metadata)
        .map_err(|e| format!("Failed to serialize cert metadata: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

/// Restrict a file's permissions to owner-only read/write (0600) on unix.
#[cfg(unix)]
pub fn restrict_key_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms)
        .map_err(|e| format!("Failed to set permissions on {}: {e}", path.display()))
}

/// PEM-encoded CA material for signing client certificates.
pub struct CaBundle {
    /// CA certificate parameters (needed for signing).
    pub params: CertificateParams,
    /// CA key pair.
    pub key_pair: KeyPair,
    /// PEM-encoded CA certificate.
    pub ca_cert_pem: String,
}

/// PEM-encoded client certificate bundle.
pub struct ClientCertBundle {
    /// PEM-encoded client certificate.
    pub cert_pem: String,
    /// PEM-encoded client private key.
    pub key_pem: String,
    /// PEM-encoded CA certificate (for the client to trust-anchor the relay).
    pub ca_cert_pem: String,
}

/// Certificate generation errors.
#[derive(Debug, thiserror::Error)]
pub enum CertError {
    /// An error occurred during certificate generation or signing.
    #[error("Certificate generation error: {0}")]
    Generation(String),
}

/// Generate a self-signed CA suitable for signing client certificates.
pub fn generate_ca(org_name: &str) -> Result<CaBundle, CertError> {
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, format!("{org_name} CA"));
    params
        .distinguished_name
        .push(DnType::OrganizationName, org_name);
    params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    params.key_usages.push(KeyUsagePurpose::CrlSign);

    let key_pair = KeyPair::generate().map_err(|e| CertError::Generation(e.to_string()))?;
    let ca_cert = params
        .self_signed(&key_pair)
        .map_err(|e| CertError::Generation(e.to_string()))?;

    Ok(CaBundle {
        ca_cert_pem: ca_cert.pem(),
        params,
        key_pair,
    })
}

/// Generate a client certificate signed by the given CA.
///
/// The certificate has `ExtendedKeyUsagePurpose::ClientAuth` and the
/// machine ID as the Common Name (CN), which the relay can extract
/// for machine identity verification.
pub fn generate_client_cert(
    ca: &CaBundle,
    machine_id: &str,
) -> Result<ClientCertBundle, CertError> {
    let issuer = Issuer::from_params(&ca.params, &ca.key_pair);

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, machine_id);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);

    let client_key = KeyPair::generate().map_err(|e| CertError::Generation(e.to_string()))?;
    let client_cert = params
        .signed_by(&client_key, &issuer)
        .map_err(|e| CertError::Generation(e.to_string()))?;

    Ok(ClientCertBundle {
        cert_pem: client_cert.pem(),
        key_pem: client_key.serialize_pem(),
        ca_cert_pem: ca.ca_cert_pem.clone(),
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

    #[test]
    fn generate_ca_produces_valid_pem() {
        let ca = generate_ca("BetCode Test").unwrap();
        assert!(ca.ca_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.ca_cert_pem.contains("END CERTIFICATE"));
    }

    #[test]
    fn generate_client_cert_has_correct_pem() {
        let ca = generate_ca("BetCode Test").unwrap();
        let bundle = generate_client_cert(&ca, "machine-001").unwrap();

        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(bundle.ca_cert_pem.contains("BEGIN CERTIFICATE"));
        // Client cert should differ from CA cert
        assert_ne!(bundle.cert_pem, bundle.ca_cert_pem);
    }

    #[test]
    fn multiple_clients_get_different_certs() {
        let ca = generate_ca("BetCode Test").unwrap();
        let c1 = generate_client_cert(&ca, "machine-a").unwrap();
        let c2 = generate_client_cert(&ca, "machine-b").unwrap();

        assert_ne!(c1.cert_pem, c2.cert_pem);
        assert_ne!(c1.key_pem, c2.key_pem);
        // Both share the same CA
        assert_eq!(c1.ca_cert_pem, c2.ca_cert_pem);
    }
}
