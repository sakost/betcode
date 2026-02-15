//! JWT token issuance and validation.

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use sha2::{Digest, Sha256};

use super::claims::Claims;

/// The well-known insecure default JWT secret that must never be used in production.
const INSECURE_DEFAULT_SECRET: &str = "dev-secret-change-me";

/// Minimum length in bytes for a JWT secret.
const MIN_SECRET_LENGTH: usize = 32;

/// Errors arising from JWT secret validation.
#[derive(Debug, thiserror::Error)]
pub enum JwtSecretError {
    /// The secret matches the well-known insecure default.
    #[error(
        "JWT secret is the insecure default 'dev-secret-change-me'. Set BETCODE_JWT_SECRET or --jwt-secret"
    )]
    InsecureDefault,
    /// The secret is shorter than the required minimum length.
    #[error("JWT secret must be at least {MIN_SECRET_LENGTH} bytes, got {0}")]
    TooShort(usize),
}

/// Validate that a JWT secret is safe for production use.
///
/// Rejects the well-known default `"dev-secret-change-me"` and any secret
/// shorter than 32 bytes.
pub fn validate_jwt_secret(secret: &str) -> Result<(), JwtSecretError> {
    if secret == INSECURE_DEFAULT_SECRET {
        return Err(JwtSecretError::InsecureDefault);
    }
    if secret.len() < MIN_SECRET_LENGTH {
        return Err(JwtSecretError::TooShort(secret.len()));
    }
    Ok(())
}

/// Manages JWT token creation and validation.
#[derive(Clone)]
pub struct JwtManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    access_ttl_secs: i64,
    refresh_ttl_secs: i64,
}

impl JwtManager {
    /// Create a new `JwtManager` with the given secret.
    pub fn new(secret: &[u8], access_ttl_secs: i64, refresh_ttl_secs: i64) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret),
            decoding_key: DecodingKey::from_secret(secret),
            access_ttl_secs,
            refresh_ttl_secs,
        }
    }

    /// Issue an access token for the given user.
    pub fn issue_access_token(
        &self,
        user_id: &str,
        username: &str,
    ) -> Result<(String, i64), jsonwebtoken::errors::Error> {
        let now = now_secs();
        let exp = now + self.access_ttl_secs;

        let claims = Claims {
            jti: uuid::Uuid::new_v4().to_string(),
            sub: user_id.to_string(),
            username: username.to_string(),
            iat: now,
            exp,
            token_type: "access".to_string(),
        };

        let token = jsonwebtoken::encode(&Header::default(), &claims, &self.encoding_key)?;
        Ok((token, self.access_ttl_secs))
    }

    /// Issue a refresh token for the given user.
    pub fn issue_refresh_token(
        &self,
        user_id: &str,
        username: &str,
    ) -> Result<(String, i64), jsonwebtoken::errors::Error> {
        let now = now_secs();
        let exp = now + self.refresh_ttl_secs;

        let claims = Claims {
            jti: uuid::Uuid::new_v4().to_string(),
            sub: user_id.to_string(),
            username: username.to_string(),
            iat: now,
            exp,
            token_type: "refresh".to_string(),
        };

        let token = jsonwebtoken::encode(&Header::default(), &claims, &self.encoding_key)?;
        Ok((token, exp))
    }

    /// Validate a token and return its claims.
    pub fn validate(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let data =
            jsonwebtoken::decode::<Claims>(token, &self.decoding_key, &Validation::default())?;
        Ok(data.claims)
    }

    /// Hash a token for storage (we don't store raw tokens).
    pub fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

fn now_secs() -> i64 {
    #[allow(clippy::cast_possible_wrap)]
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    secs
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_jwt() -> JwtManager {
        JwtManager::new(b"test-secret-key-for-testing", 3600, 86400)
    }

    #[test]
    fn issue_and_validate_access_token() {
        let jwt = test_jwt();
        let (token, ttl) = jwt.issue_access_token("user-1", "alice").unwrap();
        assert_eq!(ttl, 3600);

        let claims = jwt.validate(&token).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.username, "alice");
        assert!(claims.is_access());
    }

    #[test]
    fn issue_and_validate_refresh_token() {
        let jwt = test_jwt();
        let (token, _exp) = jwt.issue_refresh_token("user-1", "alice").unwrap();

        let claims = jwt.validate(&token).unwrap();
        assert!(claims.is_refresh());
        assert_eq!(claims.sub, "user-1");
    }

    #[test]
    fn invalid_token_fails_validation() {
        let jwt = test_jwt();
        assert!(jwt.validate("not-a-valid-token").is_err());
    }

    #[test]
    fn wrong_secret_fails_validation() {
        let jwt1 = test_jwt();
        let jwt2 = JwtManager::new(b"different-secret", 3600, 86400);

        let (token, _) = jwt1.issue_access_token("user-1", "alice").unwrap();
        assert!(jwt2.validate(&token).is_err());
    }

    #[test]
    fn token_hash_is_deterministic() {
        let h1 = JwtManager::hash_token("same-token");
        let h2 = JwtManager::hash_token("same-token");
        assert_eq!(h1, h2);

        let h3 = JwtManager::hash_token("different-token");
        assert_ne!(h1, h3);
    }

    #[test]
    fn validate_jwt_secret_rejects_default() {
        let err = validate_jwt_secret("dev-secret-change-me").unwrap_err();
        assert!(matches!(err, JwtSecretError::InsecureDefault));
    }

    #[test]
    fn validate_jwt_secret_rejects_short_secret() {
        let err = validate_jwt_secret("too-short").unwrap_err();
        assert!(matches!(err, JwtSecretError::TooShort(9)));
    }

    #[test]
    fn validate_jwt_secret_rejects_empty_secret() {
        let err = validate_jwt_secret("").unwrap_err();
        assert!(matches!(err, JwtSecretError::TooShort(0)));
    }

    #[test]
    fn validate_jwt_secret_accepts_valid_secret() {
        // 40 bytes - well above the 32-byte minimum
        let result = validate_jwt_secret("a]3kF9$mP2xL7nQ4wR8vT1yU6bJ0dH5gZ!cE@sA");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_jwt_secret_accepts_exactly_32_bytes() {
        let secret = "abcdefghijklmnopqrstuvwxyz012345"; // exactly 32 bytes
        assert_eq!(secret.len(), 32);
        let result = validate_jwt_secret(secret);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_jwt_secret_rejects_31_bytes() {
        let secret = "abcdefghijklmnopqrstuvwxyz01234"; // 31 bytes
        assert_eq!(secret.len(), 31);
        let err = validate_jwt_secret(secret).unwrap_err();
        assert!(matches!(err, JwtSecretError::TooShort(31)));
    }
}
