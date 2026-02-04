//! Authentication module for BetCode relay.
//!
//! Provides JWT token management and password hashing.

pub mod claims;
pub mod jwt;
pub mod password;

pub use claims::Claims;
pub use jwt::JwtManager;
