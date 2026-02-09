//! Crypto error types.

/// Errors from cryptographic operations.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Invalid key length: expected {expected}, got {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },

    #[error("Invalid nonce length: expected {expected}, got {actual}")]
    InvalidNonceLength { expected: usize, actual: usize },

    #[error("Key derivation failed: {0}")]
    KeyDerivationFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Nonce counter exhausted — session must be rekeyed")]
    NonceExhausted,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
