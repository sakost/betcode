//! Development certificate generation using rcgen.
//!
//! Generates self-signed CA, server, and client certificates for local
//! development and testing. NOT suitable for production use.

use std::path::Path;

use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair};
use tracing::info;

/// Generated certificate bundle (PEM-encoded).
pub struct CertBundle {
    /// CA certificate PEM.
    pub ca_cert_pem: String,
    /// Server certificate PEM.
    pub server_cert_pem: String,
    /// Server private key PEM.
    pub server_key_pem: String,
}

/// Generate a server certificate signed by the given CA.
fn generate_server_cert(
    ca: &betcode_crypto::certs::CaBundle,
    server_names: &[&str],
) -> Result<(String, String), CertError> {
    let issuer = Issuer::from_params(&ca.params, &ca.key_pair);

    let mut params = CertificateParams::new(
        server_names
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
    )
    .map_err(|e| CertError::Generation(e.to_string()))?;

    params
        .distinguished_name
        .push(DnType::CommonName, "BetCode Relay Server");
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);

    let server_key = KeyPair::generate().map_err(|e| CertError::Generation(e.to_string()))?;
    let server_cert = params
        .signed_by(&server_key, &issuer)
        .map_err(|e| CertError::Generation(e.to_string()))?;

    Ok((server_cert.pem(), server_key.serialize_pem()))
}

/// Generate a full dev certificate bundle (CA + server).
pub fn generate_dev_bundle(server_names: &[&str]) -> Result<CertBundle, CertError> {
    let ca = betcode_crypto::certs::generate_ca("BetCode Dev")
        .map_err(|e| CertError::Generation(e.to_string()))?;
    let (server_cert_pem, server_key_pem) = generate_server_cert(&ca, server_names)?;

    Ok(CertBundle {
        ca_cert_pem: ca.ca_cert_pem,
        server_cert_pem,
        server_key_pem,
    })
}

/// Write a dev certificate bundle to disk.
pub fn write_dev_certs(dir: &Path, bundle: &CertBundle) -> Result<(), CertError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| CertError::Io(format!("Failed to create cert dir: {e}")))?;

    let ca_path = dir.join("ca.pem");
    let cert_path = dir.join("server.pem");
    let key_path = dir.join("server-key.pem");

    std::fs::write(&ca_path, &bundle.ca_cert_pem)
        .map_err(|e| CertError::Io(format!("Failed to write CA cert: {e}")))?;
    std::fs::write(&cert_path, &bundle.server_cert_pem)
        .map_err(|e| CertError::Io(format!("Failed to write server cert: {e}")))?;
    std::fs::write(&key_path, &bundle.server_key_pem)
        .map_err(|e| CertError::Io(format!("Failed to write server key: {e}")))?;

    info!(
        ca = %ca_path.display(),
        cert = %cert_path.display(),
        key = %key_path.display(),
        "Dev certificates written"
    );

    Ok(())
}

/// Certificate generation errors.
#[derive(Debug, thiserror::Error)]
pub enum CertError {
    #[error("Certificate generation error: {0}")]
    Generation(String),

    #[error("I/O error: {0}")]
    Io(String),
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn generate_dev_bundle_produces_valid_pem() {
        let bundle = generate_dev_bundle(&["localhost"]).unwrap();
        assert!(bundle.ca_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.ca_cert_pem.contains("END CERTIFICATE"));
    }

    #[test]
    fn generate_server_cert_signed_by_ca() {
        let ca = betcode_crypto::certs::generate_ca("BetCode Dev Test").unwrap();
        let (cert_pem, key_pem) = generate_server_cert(&ca, &["localhost", "127.0.0.1"]).unwrap();

        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn generate_dev_bundle_all_present() {
        let bundle = generate_dev_bundle(&["localhost"]).unwrap();
        assert!(bundle.ca_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.server_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.server_key_pem.contains("BEGIN PRIVATE KEY"));
        // CA and server certs should be different
        assert_ne!(bundle.ca_cert_pem, bundle.server_cert_pem);
    }

    #[test]
    fn write_dev_certs_creates_files() {
        let dir = std::env::temp_dir().join("betcode-test-certs");
        let _ = std::fs::remove_dir_all(&dir);

        let bundle = generate_dev_bundle(&["localhost"]).unwrap();
        write_dev_certs(&dir, &bundle).unwrap();

        assert!(dir.join("ca.pem").exists());
        assert!(dir.join("server.pem").exists());
        assert!(dir.join("server-key.pem").exists());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
