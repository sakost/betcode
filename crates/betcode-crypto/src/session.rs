//! Crypto session for E2E encryption.
//!
//! Manages per-session symmetric encryption using ChaCha20-Poly1305 AEAD
//! with keys derived from X25519 ECDH + HKDF-SHA256.

use std::sync::atomic::{AtomicU32, Ordering};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

use crate::error::CryptoError;

/// HKDF info string for session key derivation.
const HKDF_INFO: &[u8] = b"betcode-e2e-session-v1";

/// HKDF salt for domain separation (recommended by RFC 5869).
const HKDF_SALT: &[u8] = b"betcode-e2e-hkdf-salt-v1";

/// Nonce size for ChaCha20-Poly1305.
pub const NONCE_SIZE: usize = 12;

/// Encrypted payload with metadata needed for decryption.
#[derive(Debug, Clone)]
pub struct EncryptedData {
    /// ChaCha20-Poly1305 ciphertext (includes 16-byte auth tag).
    pub ciphertext: Vec<u8>,
    /// 12-byte nonce used for this encryption.
    pub nonce: [u8; NONCE_SIZE],
}

/// A crypto session holding a derived symmetric key.
///
/// Created from an X25519 ECDH shared secret, with keys derived via HKDF-SHA256.
/// Provides ChaCha20-Poly1305 AEAD encryption/decryption.
pub struct CryptoSession {
    cipher: ChaCha20Poly1305,
    /// Random prefix for nonces (set once per session).
    nonce_prefix: [u8; 8],
    /// Monotonic counter for nonce uniqueness.
    nonce_counter: AtomicU32,
}

/// Derive a 32-byte key from a shared secret via HKDF-SHA256.
///
/// The caller is responsible for zeroizing the returned bytes.
fn hkdf_derive(shared_secret: &[u8; 32]) -> Result<[u8; 32], CryptoError> {
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), shared_secret);
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .map_err(|e| CryptoError::KeyDerivationFailed(e.to_string()))?;
    Ok(key)
}

impl Drop for CryptoSession {
    fn drop(&mut self) {
        self.nonce_prefix.zeroize();
    }
}

impl CryptoSession {
    /// Create a session from a raw 32-byte shared secret.
    ///
    /// The shared secret is passed through HKDF-SHA256 to derive the
    /// symmetric encryption key.
    pub fn from_shared_secret(shared_secret: &[u8; 32]) -> Result<Self, CryptoError> {
        let mut key_bytes = hkdf_derive(shared_secret)?;

        let key = Key::from_slice(&key_bytes);
        let cipher = ChaCha20Poly1305::new(key);
        key_bytes.zeroize();

        let mut nonce_prefix = [0u8; 8];
        OsRng.fill_bytes(&mut nonce_prefix);

        Ok(Self {
            cipher,
            nonce_prefix,
            nonce_counter: AtomicU32::new(0),
        })
    }

    /// Create a session from two X25519 keypairs (performs ECDH).
    ///
    /// `local_secret` is our static/ephemeral secret, `remote_public` is theirs.
    pub fn from_keypairs(
        local_secret: &StaticSecret,
        remote_public: &PublicKey,
    ) -> Result<Self, CryptoError> {
        let shared = local_secret.diffie_hellman(remote_public);
        Self::from_shared_secret(shared.as_bytes())
    }

    /// Encrypt plaintext data.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedData, CryptoError> {
        let nonce_bytes = self.next_nonce()?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

        Ok(EncryptedData {
            ciphertext,
            nonce: nonce_bytes,
        })
    }

    /// Decrypt ciphertext using the provided nonce.
    pub fn decrypt(&self, ciphertext: &[u8], nonce_bytes: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if nonce_bytes.len() != NONCE_SIZE {
            return Err(CryptoError::InvalidNonceLength {
                expected: NONCE_SIZE,
                actual: nonce_bytes.len(),
            });
        }
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
    }

    /// Generate the next unique nonce.
    ///
    /// Layout: [4-byte counter (big-endian)] [8-byte random prefix]
    ///
    /// Uses compare-and-swap to prevent counter wrapping under concurrent access.
    /// Returns `NonceExhausted` if the counter has reached `u32::MAX`,
    /// meaning the session must be rekeyed.
    ///
    /// `Ordering::Relaxed` is sufficient here because we only need the counter
    /// to produce unique values — no other memory operations depend on the
    /// counter's synchronization. The `cipher` and `nonce_prefix` fields are
    /// set once at construction and never modified.
    fn next_nonce(&self) -> Result<[u8; NONCE_SIZE], CryptoError> {
        loop {
            let current = self.nonce_counter.load(Ordering::Relaxed);
            if current == u32::MAX {
                return Err(CryptoError::NonceExhausted);
            }
            if let Ok(prev) = self.nonce_counter.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                let mut nonce = [0u8; NONCE_SIZE];
                nonce[..4].copy_from_slice(&prev.to_be_bytes());
                nonce[4..].copy_from_slice(&self.nonce_prefix);
                return Ok(nonce);
            }
        }
    }

    /// Get the current nonce counter value (for testing).
    #[cfg(any(test, feature = "test-utils"))]
    pub fn nonce_counter(&self) -> u32 {
        self.nonce_counter.load(Ordering::Relaxed)
    }
}

/// Derive a 32-byte session key from an X25519 ECDH shared secret.
///
/// **Note:** The caller is responsible for zeroizing the returned key bytes
/// when they are no longer needed.
#[cfg(any(test, feature = "test-utils"))]
pub fn derive_session_key(shared_secret: &[u8; 32]) -> Result<[u8; 32], CryptoError> {
    hkdf_derive(shared_secret)
}

/// Perform X25519 ECDH and return the raw shared secret.
#[cfg(any(test, feature = "test-utils"))]
pub fn ecdh(local_secret: &StaticSecret, remote_public: &PublicKey) -> [u8; 32] {
    *local_secret.diffie_hellman(remote_public).as_bytes()
}

/// Create a matched pair of `CryptoSessions` for testing.
///
/// Returns (`client_session`, `server_session`) that can encrypt/decrypt each other's data.
#[cfg(any(test, feature = "test-utils"))]
pub fn test_session_pair() -> Result<(CryptoSession, CryptoSession), CryptoError> {
    let client_secret = StaticSecret::random_from_rng(OsRng);
    let client_public = PublicKey::from(&client_secret);

    let server_secret = StaticSecret::random_from_rng(OsRng);
    let server_public = PublicKey::from(&server_secret);

    let client_session = CryptoSession::from_keypairs(&client_secret, &server_public)?;
    let server_session = CryptoSession::from_keypairs(&server_secret, &client_public)?;

    Ok((client_session, server_session))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Generate two X25519 keypairs for testing.
    fn generate_keypair_pair() -> (StaticSecret, PublicKey, StaticSecret, PublicKey) {
        let a_secret = StaticSecret::random_from_rng(OsRng);
        let a_public = PublicKey::from(&a_secret);
        let b_secret = StaticSecret::random_from_rng(OsRng);
        let b_public = PublicKey::from(&b_secret);
        (a_secret, a_public, b_secret, b_public)
    }

    #[test]
    fn ecdh_shared_secret_is_symmetric() {
        let (a_secret, _a_public, b_secret, b_public) = generate_keypair_pair();
        let a_public = PublicKey::from(&a_secret);

        let shared_ab = ecdh(&a_secret, &b_public);
        let shared_ba = ecdh(&b_secret, &a_public);

        assert_eq!(shared_ab, shared_ba);
    }

    #[test]
    fn derive_session_key_produces_32_bytes() {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        let shared = ecdh(&secret, &public);
        let key = derive_session_key(&shared).unwrap();
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn different_ephemeral_keys_produce_different_session_keys() {
        let a_secret = StaticSecret::random_from_rng(OsRng);
        let b_secret = StaticSecret::random_from_rng(OsRng);
        let target_secret = StaticSecret::random_from_rng(OsRng);
        let target_public = PublicKey::from(&target_secret);

        let key_a = derive_session_key(&ecdh(&a_secret, &target_public)).unwrap();
        let key_b = derive_session_key(&ecdh(&b_secret, &target_public)).unwrap();

        assert_ne!(key_a, key_b);
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let (client, server) = test_session_pair().unwrap();
        let plaintext = b"Hello, encrypted world!";

        let encrypted = client.encrypt(plaintext).unwrap();
        let decrypted = server
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_empty_data() {
        let (client, server) = test_session_pair().unwrap();

        let encrypted = client.encrypt(b"").unwrap();
        let decrypted = server
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();

        assert!(decrypted.is_empty());
    }

    #[test]
    fn encrypt_large_data() {
        let (client, server) = test_session_pair().unwrap();
        let plaintext = vec![0xABu8; 1024 * 1024]; // 1MB

        let encrypted = client.encrypt(&plaintext).unwrap();
        let decrypted = server
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let (client, _server) = test_session_pair().unwrap();
        let (_, wrong_server) = test_session_pair().unwrap();

        let encrypted = client.encrypt(b"secret data").unwrap();
        let result = wrong_server.decrypt(&encrypted.ciphertext, &encrypted.nonce);

        assert!(result.is_err());
        assert!(matches!(result, Err(CryptoError::DecryptionFailed(_))));
    }

    #[test]
    fn decrypt_with_tampered_ciphertext_fails() {
        let (client, server) = test_session_pair().unwrap();

        let mut encrypted = client.encrypt(b"secret data").unwrap();
        if let Some(byte) = encrypted.ciphertext.first_mut() {
            *byte ^= 0xFF; // Flip bits
        }

        let result = server.decrypt(&encrypted.ciphertext, &encrypted.nonce);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_with_wrong_nonce_fails() {
        let (client, server) = test_session_pair().unwrap();

        let encrypted = client.encrypt(b"secret data").unwrap();
        let wrong_nonce = [0u8; NONCE_SIZE];

        let result = server.decrypt(&encrypted.ciphertext, &wrong_nonce);
        assert!(result.is_err());
    }

    #[test]
    fn nonce_counter_increments() {
        let (client, _server) = test_session_pair().unwrap();

        assert_eq!(client.nonce_counter(), 0);
        client.encrypt(b"msg1").unwrap();
        assert_eq!(client.nonce_counter(), 1);
        client.encrypt(b"msg2").unwrap();
        assert_eq!(client.nonce_counter(), 2);
    }

    #[test]
    fn session_from_keypairs_encrypts_and_decrypts() {
        let (a_secret, a_public, b_secret, b_public) = generate_keypair_pair();

        let session_a = CryptoSession::from_keypairs(&a_secret, &b_public).unwrap();
        let session_b = CryptoSession::from_keypairs(&b_secret, &a_public).unwrap();

        let encrypted = session_a.encrypt(b"test payload").unwrap();
        let decrypted = session_b
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();
        assert_eq!(decrypted, b"test payload");
    }

    #[test]
    fn session_nonce_never_repeats() {
        let (session, _) = test_session_pair().unwrap();
        let mut nonces = std::collections::HashSet::new();

        for _ in 0..1000 {
            let encrypted = session.encrypt(b"x").unwrap();
            assert!(nonces.insert(encrypted.nonce), "nonce collision detected");
        }
    }

    #[test]
    fn session_encrypt_produces_valid_encrypted_payload() {
        let (client, server) = test_session_pair().unwrap();

        let encrypted = client.encrypt(b"payload data").unwrap();

        // Ciphertext should be longer than plaintext (16-byte AEAD tag)
        assert!(encrypted.ciphertext.len() > 12);
        assert_eq!(encrypted.nonce.len(), NONCE_SIZE);

        // Must be decryptable
        let decrypted = server
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();
        assert_eq!(decrypted, b"payload data");
    }

    #[test]
    fn decrypt_with_invalid_nonce_length_returns_error() {
        let (_, server) = test_session_pair().unwrap();
        let result = server.decrypt(b"ciphertext", &[0u8; 8]); // Wrong nonce length
        assert!(matches!(
            result,
            Err(CryptoError::InvalidNonceLength { .. })
        ));
    }

    #[test]
    fn concurrent_nonce_generation_produces_unique_nonces() {
        use std::sync::Arc;
        use std::thread;

        let (session, _) = test_session_pair().unwrap();
        let session = Arc::new(session);
        let num_threads = 8;
        let per_thread = 500;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let s = Arc::clone(&session);
                thread::spawn(move || {
                    let mut nonces = Vec::with_capacity(per_thread);
                    for _ in 0..per_thread {
                        let enc = s.encrypt(b"x").unwrap();
                        nonces.push(enc.nonce);
                    }
                    nonces
                })
            })
            .collect();

        let mut all_nonces = std::collections::HashSet::new();
        for h in handles {
            for nonce in h.join().unwrap() {
                assert!(
                    all_nonces.insert(nonce),
                    "nonce collision in concurrent test"
                );
            }
        }
        assert_eq!(all_nonces.len(), num_threads * per_thread);
        assert_eq!(session.nonce_counter() as usize, num_threads * per_thread);
    }

    #[test]
    fn nonce_exhaustion_returns_error() {
        let (client, _server) = test_session_pair().unwrap();
        // Set the counter to u32::MAX so the next encrypt triggers exhaustion
        client.nonce_counter.store(u32::MAX, Ordering::Relaxed);
        let result = client.encrypt(b"should fail");
        assert!(
            matches!(result, Err(CryptoError::NonceExhausted)),
            "expected NonceExhausted, got {result:?}"
        );
    }

    #[test]
    fn nonce_exhaustion_concurrent_near_limit() {
        use std::sync::Arc;
        use std::thread;

        let (session, _) = test_session_pair().unwrap();
        let session = Arc::new(session);
        // Set counter close to limit
        session
            .nonce_counter
            .store(u32::MAX - 100, Ordering::Relaxed);

        let handles: Vec<_> = (0..200)
            .map(|_| {
                let s = Arc::clone(&session);
                thread::spawn(move || s.encrypt(b"x"))
            })
            .collect();

        let mut success = 0u32;
        let mut exhausted = 0u32;
        for h in handles {
            match h.join().unwrap() {
                Ok(_) => success += 1,
                Err(CryptoError::NonceExhausted) => exhausted += 1,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        // Exactly 100 should succeed (counters MAX-100 through MAX-1)
        assert_eq!(success, 100);
        assert_eq!(exhausted, 100);
    }

    #[test]
    fn decrypt_empty_ciphertext() {
        let (_, server) = test_session_pair().unwrap();
        // Empty ciphertext with valid-length nonce should fail (missing auth tag)
        let result = server.decrypt(&[], &[0u8; NONCE_SIZE]);
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_with_invalid_nonce_length_zero() {
        let (_, server) = test_session_pair().unwrap();
        let result = server.decrypt(b"data", &[]);
        assert!(matches!(
            result,
            Err(CryptoError::InvalidNonceLength {
                expected: NONCE_SIZE,
                actual: 0
            })
        ));
    }

    #[test]
    fn from_shared_secret_produces_working_session() {
        let secret = [42u8; 32];
        let session1 = CryptoSession::from_shared_secret(&secret).unwrap();
        let session2 = CryptoSession::from_shared_secret(&secret).unwrap();

        // Both sessions derived from same secret should decrypt each other
        let encrypted = session1.encrypt(b"test").unwrap();
        let decrypted = session2
            .decrypt(&encrypted.ciphertext, &encrypted.nonce)
            .unwrap();
        assert_eq!(decrypted, b"test");
    }
}
