//! Key exchange protocol logic.
//!
//! Implements relay-mediated X25519 key exchange between CLI and daemon.
//! Each side generates an ephemeral keypair per session, performs ECDH,
//! and derives a symmetric session key via HKDF-SHA256.

use std::sync::Arc;

use rand::rngs::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::CryptoError;
use crate::identity::{fingerprint_of, IdentityKeyPair};
use crate::session::CryptoSession;

/// State of a key exchange in progress.
pub struct KeyExchangeState {
    /// Our ephemeral secret for this session.
    ephemeral_secret: StaticSecret,
    /// Our ephemeral public key to send to the peer.
    ephemeral_public: PublicKey,
    /// Our identity keypair (for signing/verification in future).
    identity: Option<Arc<IdentityKeyPair>>,
}

impl Default for KeyExchangeState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyExchangeState {
    /// Start a new key exchange by generating an ephemeral keypair.
    pub fn new() -> Self {
        Self::with_identity_opt(None)
    }

    /// Start a new key exchange with an identity keypair.
    ///
    /// Accepts `Arc<IdentityKeyPair>` to avoid reconstructing the keypair
    /// from raw secret bytes, which would unnecessarily expose key material.
    pub fn with_identity(identity: Arc<IdentityKeyPair>) -> Self {
        Self::with_identity_opt(Some(identity))
    }

    fn with_identity_opt(identity: Option<Arc<IdentityKeyPair>>) -> Self {
        let ephemeral_secret = StaticSecret::random_from_rng(OsRng);
        let ephemeral_public = PublicKey::from(&ephemeral_secret);
        Self {
            ephemeral_secret,
            ephemeral_public,
            identity,
        }
    }

    /// Get our ephemeral public key bytes to send to the peer.
    pub fn public_bytes(&self) -> [u8; 32] {
        *self.ephemeral_public.as_bytes()
    }

    /// Get the fingerprint of our identity public key (if set).
    pub fn identity_fingerprint(&self) -> Option<String> {
        self.identity.as_ref().map(|id| id.fingerprint())
    }

    /// Complete the key exchange with the peer's ephemeral public key.
    ///
    /// Performs X25519 ECDH and derives a CryptoSession with HKDF.
    pub fn complete(self, peer_public_bytes: &[u8]) -> Result<CryptoSession, CryptoError> {
        let arr: [u8; 32] =
            peer_public_bytes
                .try_into()
                .map_err(|_| CryptoError::InvalidKeyLength {
                    expected: 32,
                    actual: peer_public_bytes.len(),
                })?;
        let peer_public = PublicKey::from(arr);

        CryptoSession::from_keypairs(&self.ephemeral_secret, &peer_public)
    }
}

/// Perform a complete key exchange and return matching sessions for both sides.
///
/// This is a convenience function mainly useful for testing. In production,
/// each side creates a `KeyExchangeState`, sends its public bytes, and
/// calls `complete()` with the peer's public bytes.
#[cfg(any(test, feature = "test-utils"))]
pub fn perform_key_exchange() -> Result<(CryptoSession, CryptoSession), CryptoError> {
    let client_state = KeyExchangeState::new();
    let server_state = KeyExchangeState::new();

    let client_pub = client_state.public_bytes();
    let server_pub = server_state.public_bytes();

    let client_session = client_state.complete(&server_pub)?;
    let server_session = server_state.complete(&client_pub)?;

    Ok((client_session, server_session))
}

/// Verify that a remote public key matches an expected fingerprint.
///
/// Uses constant-time comparison to prevent timing side-channel attacks.
pub fn verify_fingerprint(pubkey_bytes: &[u8; 32], expected_fingerprint: &str) -> bool {
    let actual = fingerprint_of(pubkey_bytes);
    constant_time_str_eq(&actual, expected_fingerprint)
}

/// Constant-time string equality comparison.
///
/// Compares byte-by-byte using `subtle::ConstantTimeEq` to prevent
/// timing side-channel leakage of which character differs.
///
/// **Known limitation:** The length check returns early if lengths differ,
/// leaking whether the lengths match. This is acceptable for our use case
/// because all SHA-256 fingerprints are fixed-length (95 characters).
/// If this function is reused for variable-length secrets, the length
/// check must be made constant-time (e.g., by padding to a fixed length).
pub fn constant_time_str_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_exchange_produces_matching_sessions() {
        let (client, server) = perform_key_exchange().unwrap();

        let plaintext = b"test message for key exchange verification";
        let encrypted = client.encrypt(plaintext).unwrap();
        let decrypted = server
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();
        assert_eq!(decrypted, plaintext);

        // Also test reverse direction
        let encrypted2 = server.encrypt(b"reply").unwrap();
        let decrypted2 = client
            .decrypt(&encrypted2.ciphertext, &encrypted2.nonce)
            .unwrap();
        assert_eq!(decrypted2, b"reply");
    }

    #[test]
    fn key_exchange_forward_secrecy_different_sessions() {
        let (client1, server1) = perform_key_exchange().unwrap();
        let (client2, _server2) = perform_key_exchange().unwrap();

        // Encrypt with session 1's client
        let encrypted = client1.encrypt(b"secret").unwrap();

        // Session 1's server can decrypt
        let decrypted = server1
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();
        assert_eq!(decrypted, b"secret");

        // Session 2's client cannot decrypt session 1's data
        let result = client2.decrypt(&encrypted.ciphertext, &encrypted.nonce);
        assert!(result.is_err());
    }

    #[test]
    fn fingerprint_matches_pubkey() {
        let kp = IdentityKeyPair::generate();
        let expected = kp.fingerprint();
        assert!(verify_fingerprint(&kp.public_bytes(), &expected));

        // Different key should not match
        let kp2 = IdentityKeyPair::generate();
        assert!(!verify_fingerprint(&kp2.public_bytes(), &expected));
    }

    #[test]
    fn key_exchange_state_with_identity() {
        let identity = Arc::new(IdentityKeyPair::generate());
        let expected_fp = identity.fingerprint();
        let state = KeyExchangeState::with_identity(identity);

        assert_eq!(state.identity_fingerprint().unwrap(), expected_fp);
        assert_eq!(state.public_bytes().len(), 32);
    }

    #[test]
    fn constant_time_str_eq_equal_strings() {
        assert!(constant_time_str_eq("hello", "hello"));
        assert!(constant_time_str_eq("", ""));
    }

    #[test]
    fn constant_time_str_eq_different_lengths() {
        assert!(!constant_time_str_eq("short", "longer_string"));
        assert!(!constant_time_str_eq("abc", "ab"));
        assert!(!constant_time_str_eq("", "x"));
    }

    #[test]
    fn constant_time_str_eq_same_length_different_content() {
        assert!(!constant_time_str_eq("aaaa", "aaab"));
        assert!(!constant_time_str_eq("abcd", "abce"));
    }

    #[test]
    fn constant_time_str_eq_fingerprint_format() {
        let fp = "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99";
        assert!(constant_time_str_eq(fp, fp));
        let fp_diff = "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:9a";
        assert!(!constant_time_str_eq(fp, fp_diff));
    }

    #[test]
    fn complete_rejects_invalid_key_length() {
        let state = KeyExchangeState::new();
        let result = state.complete(&[0u8; 16]);
        assert!(matches!(
            result,
            Err(CryptoError::InvalidKeyLength {
                expected: 32,
                actual: 16
            })
        ));
    }

    #[test]
    fn ephemeral_keys_are_unique_across_exchanges() {
        let state1 = KeyExchangeState::new();
        let state2 = KeyExchangeState::new();
        let state3 = KeyExchangeState::new();

        let pub1 = state1.public_bytes();
        let pub2 = state2.public_bytes();
        let pub3 = state3.public_bytes();

        assert_ne!(pub1, pub2);
        assert_ne!(pub2, pub3);
        assert_ne!(pub1, pub3);
    }

    #[test]
    fn complete_with_empty_key_returns_error() {
        let state = KeyExchangeState::new();
        let result = state.complete(&[]);
        assert!(matches!(
            result,
            Err(CryptoError::InvalidKeyLength {
                expected: 32,
                actual: 0
            })
        ));
    }

    #[test]
    fn key_exchange_state_default_has_no_identity() {
        let state = KeyExchangeState::default();
        assert!(state.identity_fingerprint().is_none());
        assert_eq!(state.public_bytes().len(), 32);
    }
}
