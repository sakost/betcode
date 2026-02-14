//! Tests for `AuthService` gRPC implementation.

use std::sync::Arc;

use tonic::Request;

use betcode_proto::v1::auth_service_server::AuthService;
use betcode_proto::v1::{LoginRequest, RefreshTokenRequest, RegisterRequest, RevokeTokenRequest};

use super::auth_svc::AuthServiceImpl;
use crate::auth::jwt::JwtManager;
use crate::storage::RelayDatabase;

async fn setup() -> (AuthServiceImpl, Arc<JwtManager>) {
    let db = RelayDatabase::open_in_memory().await.unwrap();
    let jwt = Arc::new(JwtManager::new(b"test-secret", 3600, 86400));
    let svc = AuthServiceImpl::new(db, Arc::clone(&jwt));
    (svc, jwt)
}

/// Standard "alice" registration request used by most tests.
fn alice_register() -> RegisterRequest {
    RegisterRequest {
        username: "alice".into(),
        password: "password123".into(),
        email: "alice@example.com".into(),
    }
}

/// Register alice and return the registration response.
async fn register_alice(svc: &AuthServiceImpl) -> betcode_proto::v1::RegisterResponse {
    svc.register(Request::new(alice_register()))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn register_and_login() {
    let (svc, _jwt) = setup().await;

    let resp = register_alice(&svc).await;

    assert!(!resp.user_id.is_empty());
    assert!(!resp.access_token.is_empty());
    assert!(!resp.refresh_token.is_empty());
    assert_eq!(resp.expires_in_secs, 3600);

    let login_resp = svc
        .login(Request::new(LoginRequest {
            username: "alice".into(),
            password: "password123".into(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(login_resp.user_id, resp.user_id);
    assert!(!login_resp.access_token.is_empty());
}

#[tokio::test]
async fn login_wrong_password() {
    let (svc, _jwt) = setup().await;

    register_alice(&svc).await;

    let err = svc
        .login(Request::new(LoginRequest {
            username: "alice".into(),
            password: "wrongpassword".into(),
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn refresh_token_rotation() {
    let (svc, _jwt) = setup().await;

    let reg = register_alice(&svc).await;

    let refresh_resp = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!refresh_resp.access_token.is_empty());
    assert_ne!(refresh_resp.refresh_token, reg.refresh_token);

    // Old refresh token should be revoked
    let err = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn revoke_token_works() {
    let (svc, _jwt) = setup().await;

    let reg = register_alice(&svc).await;

    let resp = svc
        .revoke_token(Request::new(RevokeTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(resp.revoked);
}

#[tokio::test]
async fn register_duplicate_username() {
    let (svc, _jwt) = setup().await;

    register_alice(&svc).await;

    let err = svc
        .register(Request::new(RegisterRequest {
            username: "alice".into(),
            password: "password456".into(),
            email: "alice2@example.com".into(),
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::AlreadyExists);
}
