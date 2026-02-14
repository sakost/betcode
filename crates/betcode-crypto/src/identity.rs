//! Identity keypair management.
//!
//! Each machine has a long-lived X25519 identity keypair used for
//! authentication and key exchange bootstrapping.

use std::path::Path;

use rand::rngs::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

use crate::error::CryptoError;

/// An X25519 identity keypair for a machine.
pub struct IdentityKeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl std::fmt::Debug for IdentityKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityKeyPair")
            .field("public", &hex::encode(self.public.as_bytes()))
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl IdentityKeyPair {
    /// Generate a new random identity keypair.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Reconstruct from raw 32-byte secret key bytes.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyLength {
                expected: 32,
                actual: bytes.len(),
            });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        let secret = StaticSecret::from(arr);
        let public = PublicKey::from(&secret);
        arr.zeroize();
        Ok(Self { secret, public })
    }

    /// Get the public key.
    pub const fn public_key(&self) -> &PublicKey {
        &self.public
    }

    /// Get the public key as raw bytes.
    pub fn public_bytes(&self) -> [u8; 32] {
        *self.public.as_bytes()
    }

    /// Get the secret key as raw bytes. Handle with care.
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    /// Get the secret key reference for ECDH.
    pub const fn secret(&self) -> &StaticSecret {
        &self.secret
    }

    /// Compute a human-readable hex fingerprint of the public key.
    ///
    /// Uses SHA-256 of the public key, formatted as colon-separated hex pairs.
    pub fn fingerprint(&self) -> String {
        fingerprint_of(self.public.as_bytes())
    }

    /// Save the secret key to a file with restrictive permissions.
    pub fn save_to_file(&self, path: &Path) -> Result<(), CryptoError> {
        let dir = path.parent().ok_or_else(|| {
            CryptoError::IoError(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path has no parent directory",
            ))
        })?;
        std::fs::create_dir_all(dir)?;

        let mut bytes = self.secret_bytes();
        std::fs::write(path, bytes)?;
        bytes.zeroize();

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    /// Load a keypair from a file containing the 32-byte secret key.
    ///
    /// Reads directly into a fixed-size array to avoid heap-allocated `Vec`
    /// whose prior allocations may leave key material in freed memory.
    ///
    /// On Unix, verifies file permissions are 0600 (owner-only) before reading.
    pub fn load_from_file(path: &Path) -> Result<Self, CryptoError> {
        use std::io::Read;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(path)?;
            let mode = metadata.permissions().mode() & 0o777;
            if mode != 0o600 {
                return Err(CryptoError::IoError(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("Identity key file has insecure permissions: {mode:o} (expected 600)"),
                )));
            }
        }

        let mut file = std::fs::File::open(path)?;
        let mut buf = [0u8; 32];
        file.read_exact(&mut buf)?;
        let result = Self::from_secret_bytes(&buf);
        buf.zeroize();
        result
    }

    /// Load from file, or generate a new keypair and save it.
    pub fn load_or_generate(path: &Path) -> Result<Self, CryptoError> {
        if path.exists() {
            Self::load_from_file(path)
        } else {
            let kp = Self::generate();
            kp.save_to_file(path)?;
            Ok(kp)
        }
    }
}

/// Compute a colon-separated hex fingerprint from raw public key bytes.
pub fn fingerprint_of(pubkey_bytes: &[u8; 32]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(pubkey_bytes);
    hash.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A temporary test directory that is cleaned up on drop.
    struct TestDir {
        dir: std::path::PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("betcode-test-{}", rand::random::<u64>()));
            Self { dir }
        }

        fn key_path(&self) -> std::path::PathBuf {
            self.dir.join("identity.key")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    /// Write `data` to `path`, creating parent dirs and setting 0o600 permissions on Unix.
    fn write_test_key_file(path: &Path, data: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, data).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    /// Generate a keypair and save it to a temporary directory.
    /// Returns the `TestDir` (for lifetime/cleanup), the key path, and the keypair.
    fn generate_and_save_keypair() -> (TestDir, std::path::PathBuf, IdentityKeyPair) {
        let test_dir = TestDir::new();
        let path = test_dir.key_path();
        let kp = IdentityKeyPair::generate();
        kp.save_to_file(&path).unwrap();
        (test_dir, path, kp)
    }

    #[test]
    fn generate_identity_keypair_produces_32_byte_keys() {
        let kp = IdentityKeyPair::generate();
        assert_eq!(kp.public_bytes().len(), 32);
        assert_eq!(kp.secret_bytes().len(), 32);
    }

    #[test]
    fn identity_keypair_roundtrip_serialize_deserialize() {
        let kp = IdentityKeyPair::generate();
        let secret = kp.secret_bytes();
        let public = kp.public_bytes();

        let kp2 = IdentityKeyPair::from_secret_bytes(&secret).unwrap();
        assert_eq!(kp2.public_bytes(), public);
        assert_eq!(kp2.secret_bytes(), secret);
    }

    #[test]
    fn fingerprint_is_human_readable_hex() {
        let kp = IdentityKeyPair::generate();
        let fp = kp.fingerprint();

        // SHA-256 = 32 bytes = 32 hex pairs + 31 colons = 95 chars
        assert_eq!(fp.len(), 95);
        assert!(fp.contains(':'));

        // Each segment is 2 hex chars
        for segment in fp.split(':') {
            assert_eq!(segment.len(), 2);
            assert!(segment.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn two_keypairs_are_distinct() {
        let kp1 = IdentityKeyPair::generate();
        let kp2 = IdentityKeyPair::generate();
        assert_ne!(kp1.public_bytes(), kp2.public_bytes());
        assert_ne!(kp1.secret_bytes(), kp2.secret_bytes());
    }

    #[test]
    fn from_secret_bytes_rejects_wrong_length() {
        let err = IdentityKeyPair::from_secret_bytes(&[0u8; 16]).unwrap_err();
        match err {
            CryptoError::InvalidKeyLength {
                expected: 32,
                actual: 16,
            } => {}
            _ => panic!("wrong error: {err:?}"),
        }
    }

    #[test]
    fn save_and_load_identity_key() {
        let (_test_dir, path, kp) = generate_and_save_keypair();

        let loaded = IdentityKeyPair::load_from_file(&path).unwrap();
        assert_eq!(loaded.public_bytes(), kp.public_bytes());
        assert_eq!(loaded.secret_bytes(), kp.secret_bytes());
    }

    #[test]
    fn load_nonexistent_generates_new() {
        let test_dir = TestDir::new();
        let path = test_dir.key_path();

        let kp = IdentityKeyPair::load_or_generate(&path).unwrap();
        assert!(path.exists());

        // Loading again returns the same key
        let kp2 = IdentityKeyPair::load_or_generate(&path).unwrap();
        assert_eq!(kp.public_bytes(), kp2.public_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn file_permissions_are_restrictive() {
        use std::os::unix::fs::PermissionsExt;

        let (_test_dir, path, _kp) = generate_and_save_keypair();

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[test]
    fn load_wrong_byte_count_file() {
        let test_dir = TestDir::new();
        let path = test_dir.key_path();

        // Write 16 bytes instead of 32 — read_exact should fail
        write_test_key_file(&path, &[0u8; 16]);
        let result = IdentityKeyPair::load_from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        let result = IdentityKeyPair::load_from_file(Path::new("/nonexistent/identity.key"));
        assert!(result.is_err());
    }

    #[test]
    fn load_truncated_file_fails_gracefully() {
        let test_dir = TestDir::new();
        let path = test_dir.key_path();

        // Simulate partial write (20 bytes instead of 32)
        write_test_key_file(&path, &[0u8; 20]);

        let result = IdentityKeyPair::load_from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn load_file_with_trailing_garbage_still_works() {
        use std::io::Write;

        let (_test_dir, path, kp) = generate_and_save_keypair();

        // Append garbage after valid 32 bytes
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"GARBAGE_DATA").unwrap();

        // read_exact only reads first 32 bytes — should still work
        let loaded = IdentityKeyPair::load_from_file(&path).unwrap();
        assert_eq!(loaded.public_bytes(), kp.public_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_world_readable_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (_test_dir, path, _kp) = generate_and_save_keypair();

        // Make world-readable
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let result = IdentityKeyPair::load_from_file(&path);
        assert!(result.is_err());

        // Restore for cleanup
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn debug_impl_redacts_secret() {
        let kp = IdentityKeyPair::generate();
        let debug_output = format!("{kp:?}");
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output should redact secret key"
        );
        assert!(
            !debug_output.contains(&hex::encode(kp.secret_bytes())),
            "Debug output must not contain raw secret bytes"
        );
    }

    #[test]
    fn public_key_accessor_returns_correct_key() {
        let kp = IdentityKeyPair::generate();
        assert_eq!(kp.public_key().as_bytes(), &kp.public_bytes());
    }

    #[test]
    fn secret_accessor_matches_secret_bytes() {
        let kp = IdentityKeyPair::generate();
        assert_eq!(kp.secret().to_bytes(), kp.secret_bytes());
    }
}
