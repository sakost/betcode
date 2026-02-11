//! JWT claims structure for `BetCode` relay auth.

use serde::{Deserialize, Serialize};

/// JWT claims embedded in access tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// JWT ID (unique per token).
    pub jti: String,
    /// Subject (user ID).
    pub sub: String,
    /// Username.
    pub username: String,
    /// Issued at (unix timestamp).
    pub iat: i64,
    /// Expiration (unix timestamp).
    pub exp: i64,
    /// Token type: "access" or "refresh".
    pub token_type: String,
}

impl Claims {
    pub fn is_access(&self) -> bool {
        self.token_type == "access"
    }

    pub fn is_refresh(&self) -> bool {
        self.token_type == "refresh"
    }
}
