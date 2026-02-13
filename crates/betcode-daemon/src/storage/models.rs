//! Database models for `BetCode` daemon.

use serde::{Deserialize, Serialize};

/// Session record from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub claude_session_id: Option<String>,
    pub worktree_id: Option<String>,
    pub status: String,
    pub model: String,
    pub working_directory: String,
    pub input_lock_client: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
    pub last_message_preview: Option<String>,
    pub compaction_sequence: i64,
}

/// Message record from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: i64,
    pub session_id: String,
    pub sequence: i64,
    pub message_type: String,
    pub payload: String,
    pub created_at: i64,
}

/// Worktree record from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Worktree {
    pub id: String,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub repo_id: String,
    pub setup_script: Option<String>,
    pub created_at: i64,
    pub last_active: i64,
}

/// Git repository record from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GitRepoRow {
    pub id: String,
    pub name: String,
    pub repo_path: String,
    pub worktree_mode: String,
    pub local_subfolder: String,
    pub custom_path: Option<String>,
    pub setup_script: Option<String>,
    pub auto_gitignore: i64,
    pub created_at: i64,
    pub last_active: i64,
}

/// Permission grant record from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PermissionGrant {
    pub id: i64,
    pub session_id: String,
    pub tool_name: String,
    pub pattern: Option<String>,
    pub action: String,
    pub created_at: i64,
}

/// Connected client record from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConnectedClient {
    pub client_id: String,
    pub session_id: Option<String>,
    pub client_type: String,
    pub has_input_lock: i64,
    pub connected_at: i64,
    pub last_heartbeat: i64,
}

/// Todo item record from the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Todo {
    pub id: i64,
    pub session_id: String,
    pub subject: String,
    pub description: Option<String>,
    pub active_form: String,
    pub status: String,
    pub sequence: i64,
    pub updated_at: i64,
}

/// Session status enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Idle,
    Active,
    Completed,
    Error,
}

impl SessionStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
