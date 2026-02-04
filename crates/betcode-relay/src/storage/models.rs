//! Data models for BetCode relay storage.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Token {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub expires_at: i64,
    pub revoked: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub status: String,
    pub registered_at: i64,
    pub last_seen: i64,
    pub metadata: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BufferedMessage {
    pub id: i64,
    pub machine_id: String,
    pub request_id: String,
    pub method: String,
    pub payload: Vec<u8>,
    pub metadata: String,
    pub priority: i64,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Certificate {
    pub id: String,
    pub machine_id: Option<String>,
    pub subject_cn: String,
    pub serial_number: String,
    pub not_before: i64,
    pub not_after: i64,
    pub pem_cert: String,
    pub revoked: i64,
    pub created_at: i64,
}
