//! `BetCode` E2E Encryption Library
//!
//! Provides cryptographic primitives for end-to-end encryption between
//! CLI/mobile clients and daemons, with the relay unable to see
//! sensitive content.
//!
//! ## Crypto primitives
//!
//! - **Identity**: X25519 static keypair per machine
//! - **Session**: X25519 ephemeral ECDH per session → HKDF-SHA256 → symmetric key
//! - **Encryption**: ChaCha20-Poly1305 AEAD, 12-byte nonce (4-byte counter + 8-byte random prefix)

pub mod error;
pub mod exchange;
pub mod fingerprint_store;
pub mod fingerprint_visual;
pub mod identity;
pub mod session;

pub use error::CryptoError;
#[cfg(any(test, feature = "test-utils"))]
pub use exchange::perform_key_exchange;
pub use exchange::{KeyExchangeState, constant_time_str_eq, verify_fingerprint};
pub use fingerprint_store::{FingerprintCheck, FingerprintStore};
pub use fingerprint_visual::{
    compare_fingerprints, fingerprint_randomart, format_fingerprint_display,
};
pub use identity::{IdentityKeyPair, fingerprint_of};
#[cfg(any(test, feature = "test-utils"))]
pub use session::test_session_pair;
pub use session::{CryptoSession, EncryptedData, NONCE_SIZE};
