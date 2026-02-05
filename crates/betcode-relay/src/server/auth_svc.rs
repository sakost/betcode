//! AuthService gRPC implementation.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::{info, instrument, warn};

use betcode_proto::v1::auth_service_server::AuthService;
use betcode_proto::v1::{
    LoginRequest, LoginResponse, RefreshTokenRequest, RefreshTokenResponse, RegisterRequest,
    RegisterResponse, RevokeTokenRequest, RevokeTokenResponse,
};

use crate::auth::jwt::JwtManager;
use crate::auth::password;
use crate::storage::RelayDatabase;

pub struct AuthServiceImpl {
    db: RelayDatabase,
    jwt: Arc<JwtManager>,
}

impl AuthServiceImpl {
    pub fn new(db: RelayDatabase, jwt: Arc<JwtManager>) -> Self {
        Self { db, jwt }
    }
}

#[tonic::async_trait]
impl AuthService for AuthServiceImpl {
    #[instrument(skip(self, request), fields(rpc = "Login"))]
    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();

        let user = self
            .db
            .get_user_by_username(&req.username)
            .await
            .map_err(|_| Status::unauthenticated("Invalid credentials"))?;

        let valid = password::verify_password(&req.password, &user.password_hash)
            .map_err(|_| Status::internal("Password verification failed"))?;

        if !valid {
            warn!(username = %req.username, "Failed login attempt");
            return Err(Status::unauthenticated("Invalid credentials"));
        }

        let (access_token, expires_in) = self
            .jwt
            .issue_access_token(&user.id, &user.username)
            .map_err(|e| Status::internal(format!("Token creation failed: {}", e)))?;

        let (refresh_token, refresh_exp) =
            self.jwt
                .issue_refresh_token(&user.id, &user.username)
                .map_err(|e| Status::internal(format!("Token creation failed: {}", e)))?;

        let token_id = uuid::Uuid::new_v4().to_string();
        let token_hash = JwtManager::hash_token(&refresh_token);
        self.db
            .create_token(&token_id, &user.id, &token_hash, refresh_exp)
            .await
            .map_err(|e| Status::internal(format!("Token storage failed: {}", e)))?;

        info!(user_id = %user.id, username = %user.username, "User logged in");

        Ok(Response::new(LoginResponse {
            access_token,
            refresh_token,
            expires_in_secs: expires_in,
            user_id: user.id,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "Register"))]
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let req = request.into_inner();

        if req.username.len() < 3 {
            return Err(Status::invalid_argument(
                "Username must be at least 3 characters",
            ));
        }
        if req.password.len() < 8 {
            return Err(Status::invalid_argument(
                "Password must be at least 8 characters",
            ));
        }

        if self.db.get_user_by_username(&req.username).await.is_ok() {
            return Err(Status::already_exists("Username already taken"));
        }

        let hash = password::hash_password(&req.password)
            .map_err(|e| Status::internal(format!("Password hashing failed: {}", e)))?;

        let user_id = uuid::Uuid::new_v4().to_string();
        self.db
            .create_user(&user_id, &req.username, &req.email, &hash)
            .await
            .map_err(|e| Status::internal(format!("User creation failed: {}", e)))?;

        let (access_token, expires_in) = self
            .jwt
            .issue_access_token(&user_id, &req.username)
            .map_err(|e| Status::internal(format!("Token creation failed: {}", e)))?;

        let (refresh_token, refresh_exp) = self
            .jwt
            .issue_refresh_token(&user_id, &req.username)
            .map_err(|e| Status::internal(format!("Token creation failed: {}", e)))?;

        let token_id = uuid::Uuid::new_v4().to_string();
        let token_hash = JwtManager::hash_token(&refresh_token);
        self.db
            .create_token(&token_id, &user_id, &token_hash, refresh_exp)
            .await
            .map_err(|e| Status::internal(format!("Token storage failed: {}", e)))?;

        info!(user_id = %user_id, username = %req.username, "User registered");

        Ok(Response::new(RegisterResponse {
            user_id,
            access_token,
            refresh_token,
            expires_in_secs: expires_in,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "RefreshToken"))]
    async fn refresh_token(
        &self,
        request: Request<RefreshTokenRequest>,
    ) -> Result<Response<RefreshTokenResponse>, Status> {
        let req = request.into_inner();

        let claims = self
            .jwt
            .validate(&req.refresh_token)
            .map_err(|_| Status::unauthenticated("Invalid refresh token"))?;

        if !claims.is_refresh() {
            return Err(Status::invalid_argument("Not a refresh token"));
        }

        let token_hash = JwtManager::hash_token(&req.refresh_token);
        let stored = self
            .db
            .get_token_by_hash(&token_hash)
            .await
            .map_err(|e| Status::internal(format!("Token lookup failed: {}", e)))?
            .ok_or_else(|| Status::unauthenticated("Refresh token revoked or expired"))?;

        // Revoke old refresh token (rotation)
        self.db
            .revoke_token(&stored.id)
            .await
            .map_err(|e| Status::internal(format!("Token revocation failed: {}", e)))?;

        let (access_token, expires_in) = self
            .jwt
            .issue_access_token(&claims.sub, &claims.username)
            .map_err(|e| Status::internal(format!("Token creation failed: {}", e)))?;

        let (refresh_token, refresh_exp) = self
            .jwt
            .issue_refresh_token(&claims.sub, &claims.username)
            .map_err(|e| Status::internal(format!("Token creation failed: {}", e)))?;

        let new_token_id = uuid::Uuid::new_v4().to_string();
        let new_hash = JwtManager::hash_token(&refresh_token);
        self.db
            .create_token(&new_token_id, &claims.sub, &new_hash, refresh_exp)
            .await
            .map_err(|e| Status::internal(format!("Token storage failed: {}", e)))?;

        Ok(Response::new(RefreshTokenResponse {
            access_token,
            refresh_token,
            expires_in_secs: expires_in,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "RevokeToken"))]
    async fn revoke_token(
        &self,
        request: Request<RevokeTokenRequest>,
    ) -> Result<Response<RevokeTokenResponse>, Status> {
        let req = request.into_inner();

        let token_hash = JwtManager::hash_token(&req.refresh_token);
        let stored = self
            .db
            .get_token_by_hash(&token_hash)
            .await
            .map_err(|e| Status::internal(format!("Token lookup failed: {}", e)))?;

        let revoked = if let Some(token) = stored {
            self.db
                .revoke_token(&token.id)
                .await
                .map_err(|e| Status::internal(format!("Revocation failed: {}", e)))?
        } else {
            false
        };

        Ok(Response::new(RevokeTokenResponse { revoked }))
    }
}
