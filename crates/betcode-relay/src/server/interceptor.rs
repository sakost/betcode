//! JWT validation interceptor for gRPC requests.

use std::sync::Arc;

use tonic::{Request, Status};

use crate::auth::claims::Claims;
use crate::auth::jwt::JwtManager;

/// Extract and validate JWT from the authorization metadata header.
pub fn jwt_interceptor(
    jwt: Arc<JwtManager>,
) -> impl Fn(Request<()>) -> Result<Request<()>, Status> + Clone {
    move |mut req: Request<()>| {
        let token = req
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| Status::unauthenticated("Missing authorization header"))?;

        let claims = jwt
            .validate(token)
            .map_err(|_| Status::unauthenticated("Invalid token"))?;

        if !claims.is_access() {
            return Err(Status::unauthenticated("Not an access token"));
        }

        req.extensions_mut().insert(claims);
        Ok(req)
    }
}

/// Extract claims from a request that has passed through the interceptor.
#[allow(clippy::result_large_err)]
pub fn extract_claims<T>(req: &Request<T>) -> Result<&Claims, Status> {
    req.extensions()
        .get::<Claims>()
        .ok_or_else(|| Status::internal("Claims not found in request extensions"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::metadata::MetadataValue;

    fn test_jwt() -> Arc<JwtManager> {
        Arc::new(JwtManager::new(b"test-secret", 3600, 86400))
    }

    #[test]
    fn valid_access_token_passes() {
        let jwt = test_jwt();
        let (token, _) = jwt.issue_access_token("u1", "alice").unwrap();

        let mut req = Request::new(());
        req.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {}", token)).unwrap(),
        );

        let interceptor = jwt_interceptor(jwt);
        let result = interceptor(req);
        assert!(result.is_ok());

        let req = result.unwrap();
        let claims = extract_claims(&req).unwrap();
        assert_eq!(claims.sub, "u1");
    }

    #[test]
    fn missing_header_fails() {
        let jwt = test_jwt();
        let req = Request::new(());

        let interceptor = jwt_interceptor(jwt);
        let err = interceptor(req).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn refresh_token_rejected() {
        let jwt = test_jwt();
        let (token, _) = jwt.issue_refresh_token("u1", "alice").unwrap();

        let mut req = Request::new(());
        req.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {}", token)).unwrap(),
        );

        let interceptor = jwt_interceptor(jwt);
        let err = interceptor(req).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
}
