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
    pub fn public_key(&self) -> &PublicKey {
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
    pub fn secret(&self) -> &StaticSecret {
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
    pub fn load_from_file(path: &Path) -> Result<Self, CryptoError> {
        use std::io::Read;
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
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

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
            _ => panic!("wrong error: {:?}", err),
        }
    }

    #[test]
    fn save_and_load_identity_key() {
        let dir = std::env::temp_dir().join(format!("betcode-test-{}", rand::random::<u64>()));
        let path = dir.join("identity.key");

        let kp = IdentityKeyPair::generate();
        kp.save_to_file(&path).unwrap();

        let loaded = IdentityKeyPair::load_from_file(&path).unwrap();
        assert_eq!(loaded.public_bytes(), kp.public_bytes());
        assert_eq!(loaded.secret_bytes(), kp.secret_bytes());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_nonexistent_generates_new() {
        let dir = std::env::temp_dir().join(format!("betcode-test-{}", rand::random::<u64>()));
        let path = dir.join("identity.key");

        let kp = IdentityKeyPair::load_or_generate(&path).unwrap();
        assert!(path.exists());

        // Loading again returns the same key
        let kp2 = IdentityKeyPair::load_or_generate(&path).unwrap();
        assert_eq!(kp.public_bytes(), kp2.public_bytes());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn file_permissions_are_restrictive() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("betcode-test-{}", rand::random::<u64>()));
        let path = dir.join("identity.key");

        let kp = IdentityKeyPair::generate();
        kp.save_to_file(&path).unwrap();

        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);

        std::fs::remove_dir_all(&dir).ok();
    }
}
