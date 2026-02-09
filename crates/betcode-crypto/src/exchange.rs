//! Key exchange protocol logic.
//!
//! Implements relay-mediated X25519 key exchange between CLI and daemon.
//! Each side generates an ephemeral keypair per session, performs ECDH,
//! and derives a symmetric session key via HKDF-SHA256.

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
    identity: Option<IdentityKeyPair>,
}

impl Default for KeyExchangeState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyExchangeState {
    /// Start a new key exchange by generating an ephemeral keypair.
    pub fn new() -> Self {
        let ephemeral_secret = StaticSecret::random_from_rng(OsRng);
        let ephemeral_public = PublicKey::from(&ephemeral_secret);
        Self {
            ephemeral_secret,
            ephemeral_public,
            identity: None,
        }
    }

    /// Start a new key exchange with an identity keypair.
    pub fn with_identity(identity: IdentityKeyPair) -> Self {
        let ephemeral_secret = StaticSecret::random_from_rng(OsRng);
        let ephemeral_public = PublicKey::from(&ephemeral_secret);
        Self {
            ephemeral_secret,
            ephemeral_public,
            identity: Some(identity),
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
        if peer_public_bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyLength {
                expected: 32,
                actual: peer_public_bytes.len(),
            });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(peer_public_bytes);
        let peer_public = PublicKey::from(arr);

        CryptoSession::from_keypairs(&self.ephemeral_secret, &peer_public)
    }
}

/// Perform a complete key exchange and return matching sessions for both sides.
///
/// This is a convenience function mainly useful for testing. In production,
/// each side creates a `KeyExchangeState`, sends its public bytes, and
/// calls `complete()` with the peer's public bytes.
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
pub fn verify_fingerprint(pubkey_bytes: &[u8; 32], expected_fingerprint: &str) -> bool {
    fingerprint_of(pubkey_bytes) == expected_fingerprint
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
        let identity = IdentityKeyPair::generate();
        let expected_fp = identity.fingerprint();
        let state = KeyExchangeState::with_identity(identity);

        assert_eq!(state.identity_fingerprint().unwrap(), expected_fp);
        assert_eq!(state.public_bytes().len(), 32);
    }

    #[test]
    fn complete_rejects_invalid_key_length() {
        let state = KeyExchangeState::new();
        let result = state.complete(&[0u8; 16]);
        assert!(matches!(
            result,
            Err(CryptoError::InvalidKeyLength { expected: 32, actual: 16 })
        ));
    }
}
