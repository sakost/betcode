//! Push notification support (FCM).
//!
//! This module is only compiled when the `push-notifications` Cargo feature is
//! enabled. It provides:
//! - [`FcmClient`] for sending push notifications via the FCM HTTP v1 API
//! - [`NotificationServiceImpl`] gRPC service for device token registration
//! - Database queries for persisting device tokens

pub mod fcm;
pub mod service;

pub use fcm::FcmClient;
pub use service::NotificationServiceImpl;

/// Errors that can occur in the notification subsystem.
#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    /// Failed to read or parse the FCM service account credentials file.
    #[error("FCM credentials error: {0}")]
    Credentials(String),

    /// HTTP request to FCM API failed.
    #[error("FCM request error: {0}")]
    Request(String),

    /// FCM API returned a non-success status code.
    #[error("FCM API error (status {status}): {body}")]
    ApiError {
        /// HTTP status code returned by FCM.
        status: u16,
        /// Response body from FCM.
        body: String,
    },

    /// Database operation failed.
    #[error("Database error: {0}")]
    Database(String),
}

impl From<betcode_core::db::DatabaseError> for NotificationError {
    fn from(e: betcode_core::db::DatabaseError) -> Self {
        Self::Database(e.to_string())
    }
}
