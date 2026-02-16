//! Client certificate generation for mTLS machine identity.
//!
//! Provides CA generation and client certificate signing for daemon
//! machine identity in mTLS connections to the relay.
//!
//! Requires the `certs` feature to be enabled.

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};

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
