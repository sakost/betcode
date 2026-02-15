//! Tests for `AuthService` gRPC implementation.

use std::sync::Arc;

use tonic::Request;

use betcode_proto::v1::auth_service_server::AuthService;
use betcode_proto::v1::{LoginRequest, RefreshTokenRequest, RegisterRequest, RevokeTokenRequest};

use super::auth_svc::AuthServiceImpl;
use crate::auth::jwt::JwtManager;
use crate::storage::RelayDatabase;

/// Default grace period used in tests (30 seconds).
const TEST_GRACE_PERIOD: i64 = 30;

async fn setup() -> (AuthServiceImpl, Arc<JwtManager>) {
    setup_with_grace(TEST_GRACE_PERIOD).await
}

async fn setup_with_grace(grace_period_secs: i64) -> (AuthServiceImpl, Arc<JwtManager>) {
    let db = RelayDatabase::open_in_memory().await.unwrap();
    let jwt = Arc::new(JwtManager::new(b"test-secret", 3600, 86400));
    let svc = AuthServiceImpl::new(db, Arc::clone(&jwt), grace_period_secs);
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

    // Old refresh token should still work within grace window (retry scenario)
    let retry_resp = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!retry_resp.access_token.is_empty());
    assert!(!retry_resp.refresh_token.is_empty());
}

#[tokio::test]
async fn refresh_token_grace_retry_returns_valid_tokens() {
    let (svc, _jwt) = setup().await;

    let reg = register_alice(&svc).await;

    // First rotation
    let first = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Grace-period retry with old token
    let retry = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!retry.access_token.is_empty());
    assert!(!retry.refresh_token.is_empty());
    // Retry should produce different tokens than the first rotation
    assert_ne!(retry.refresh_token, first.refresh_token);
    assert_ne!(retry.access_token, first.access_token);
}

#[tokio::test]
async fn refresh_token_grace_retry_revokes_orphaned_successor() {
    let (svc, _jwt) = setup().await;

    let reg = register_alice(&svc).await;

    // First rotation — produces successor T2
    let first = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Grace-period retry — revokes T2, produces successor T3
    let _retry = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    // T2 (from first rotation) should now be revoked
    let err = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: first.refresh_token,
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn refresh_token_hard_revoked_not_accepted() {
    let (svc, _jwt) = setup().await;

    let reg = register_alice(&svc).await;

    // Explicitly revoke (logout)
    svc.revoke_token(Request::new(RevokeTokenRequest {
        refresh_token: reg.refresh_token.clone(),
    }))
    .await
    .unwrap();

    // Even within grace window, hard-revoked token should fail
    let err = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn refresh_token_new_token_works_after_rotation() {
    let (svc, _jwt) = setup().await;

    let reg = register_alice(&svc).await;

    // First rotation
    let first = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    // Use the NEW refresh token to rotate again — normal chain continues
    let second = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: first.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!second.access_token.is_empty());
    assert!(!second.refresh_token.is_empty());
}

#[tokio::test]
async fn refresh_token_full_recovery_flow() {
    let (svc, _jwt) = setup().await;

    let reg = register_alice(&svc).await;

    // Step 1: Normal refresh
    let _first = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    // Step 2: "Lose response" — retry with old token (grace window)
    let recovery = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: reg.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    // Step 3: Use recovered tokens for next refresh — full cycle works
    let next = svc
        .refresh_token(Request::new(RefreshTokenRequest {
            refresh_token: recovery.refresh_token,
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(!next.access_token.is_empty());
    assert!(!next.refresh_token.is_empty());
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
