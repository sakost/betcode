//! FCM HTTP v1 API client.
//!
//! Constructs and sends push notification requests to the Firebase Cloud
//! Messaging HTTP v1 API endpoint.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::NotificationError;

/// FCM HTTP v1 API endpoint template.
/// The `{project_id}` placeholder is replaced with the actual project ID.
const FCM_API_URL_TEMPLATE: &str =
    "https://fcm.googleapis.com/v1/projects/{project_id}/messages:send";

/// Service account credentials loaded from a Google Cloud JSON key file.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAccountCredentials {
    /// The Google Cloud project ID.
    pub project_id: String,

    /// The service account email (used for constructing JWTs for auth, but
    /// for this implementation we pass the credential file path and use
    /// a bearer token approach).
    #[serde(default)]
    pub client_email: String,

    /// The private key in PEM format.
    #[serde(default)]
    pub private_key: String,
}

/// FCM notification message payload.
#[derive(Debug, Serialize)]
pub struct FcmMessage {
    /// The wrapper message object required by the FCM v1 API.
    pub message: FcmMessageBody,
}

/// The inner message body sent to FCM.
#[derive(Debug, Serialize)]
pub struct FcmMessageBody {
    /// The device registration token to send the notification to.
    pub token: String,

    /// The notification payload (title + body).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification: Option<FcmNotification>,

    /// Optional data payload (key-value string pairs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<std::collections::HashMap<String, String>>,
}

/// FCM notification display payload.
#[derive(Debug, Serialize)]
pub struct FcmNotification {
    /// The notification title.
    pub title: String,

    /// The notification body text.
    pub body: String,
}

/// Environment variable name for the FCM access token.
const FCM_ACCESS_TOKEN_ENV: &str = "BETCODE_FCM_ACCESS_TOKEN";

/// Client for the FCM HTTP v1 API.
///
/// Holds the HTTP client, service account credentials, and the resolved API
/// endpoint URL.
#[derive(Debug)]
pub struct FcmClient {
    /// The reqwest HTTP client.
    http: reqwest::Client,

    /// Service account credentials.
    credentials: ServiceAccountCredentials,

    /// The fully-resolved FCM API URL for this project.
    api_url: String,

    /// Bearer token for FCM API authentication, read from the
    /// `BETCODE_FCM_ACCESS_TOKEN` environment variable at construction time.
    /// When `None`, falls back to `credentials.private_key`.
    access_token: Option<String>,
}

/// Read the FCM access token from the environment.
///
/// Logs a warning if the variable is not set, since the fallback to
/// `credentials.private_key` is unlikely to work with the real FCM API.
fn read_access_token_from_env() -> Option<String> {
    let token = std::env::var(FCM_ACCESS_TOKEN_ENV).ok();
    if token.is_none() {
        warn!(
            "Environment variable {FCM_ACCESS_TOKEN_ENV} is not set; \
             falling back to credentials.private_key for FCM auth"
        );
    }
    token
}

impl FcmClient {
    /// Create a new FCM client by loading service account credentials from a
    /// JSON file.
    ///
    /// # Errors
    ///
    /// Returns `NotificationError::Credentials` if the file cannot be read or
    /// parsed.
    pub fn from_credentials_file(path: &Path) -> Result<Self, NotificationError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            NotificationError::Credentials(format!(
                "Failed to read credentials file {}: {e}",
                path.display()
            ))
        })?;

        let credentials: ServiceAccountCredentials =
            serde_json::from_str(&content).map_err(|e| {
                NotificationError::Credentials(format!("Failed to parse credentials JSON: {e}"))
            })?;

        let api_url = FCM_API_URL_TEMPLATE.replace("{project_id}", &credentials.project_id);
        let access_token = read_access_token_from_env();

        debug!(
            project_id = %credentials.project_id,
            has_env_token = access_token.is_some(),
            "FCM client initialized"
        );

        Ok(Self {
            http: reqwest::Client::new(),
            credentials,
            api_url,
            access_token,
        })
    }

    /// Create an FCM client from pre-parsed credentials and a pre-built HTTP
    /// client.
    pub fn from_credentials(credentials: ServiceAccountCredentials, http: reqwest::Client) -> Self {
        let api_url = FCM_API_URL_TEMPLATE.replace("{project_id}", &credentials.project_id);
        let access_token = read_access_token_from_env();

        Self {
            http,
            credentials,
            api_url,
            access_token,
        }
    }

    /// Create an FCM client for testing purposes only.
    ///
    /// Installs the `ring` crypto provider (via dev-dependency on `rustls`)
    /// so that `reqwest::Client` can be constructed in the test environment
    /// where `rustls-no-provider` is the workspace default.
    #[cfg(test)]
    #[allow(clippy::expect_used)]
    pub(crate) fn for_testing(credentials: ServiceAccountCredentials) -> Self {
        let api_url = FCM_API_URL_TEMPLATE.replace("{project_id}", &credentials.project_id);

        // Install ring as the default crypto provider (no-op if already installed).
        let _ = rustls::crypto::ring::default_provider().install_default();

        let http = reqwest::Client::builder()
            .build()
            .expect("failed to build test HTTP client");

        Self {
            http,
            credentials,
            api_url,
            access_token: Some("test-access-token".to_string()),
        }
    }

    /// Build an [`FcmMessage`] for the given device token with a notification
    /// payload.
    pub fn build_message(
        device_token: &str,
        title: &str,
        body: &str,
        data: Option<std::collections::HashMap<String, String>>,
    ) -> FcmMessage {
        FcmMessage {
            message: FcmMessageBody {
                token: device_token.to_string(),
                notification: Some(FcmNotification {
                    title: title.to_string(),
                    body: body.to_string(),
                }),
                data,
            },
        }
    }

    /// Send a push notification via the FCM HTTP v1 API.
    ///
    /// # Errors
    ///
    /// Returns `NotificationError::Request` if the HTTP request fails, or
    /// `NotificationError::ApiError` if FCM returns a non-2xx status code.
    pub async fn send(&self, message: &FcmMessage) -> Result<(), NotificationError> {
        let response = self
            .http
            .post(&self.api_url)
            .header("Authorization", self.auth_header())
            .json(message)
            .send()
            .await
            .map_err(|e| NotificationError::Request(e.to_string()))?;

        let status = response.status();
        if status.is_success() {
            debug!("FCM notification sent successfully");
            Ok(())
        } else {
            let status_code = status.as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            warn!(status = status_code, body = %body, "FCM API returned error");
            Err(NotificationError::ApiError {
                status: status_code,
                body,
            })
        }
    }

    /// Construct the Authorization header value.
    ///
    /// Uses the access token read from `BETCODE_FCM_ACCESS_TOKEN` at
    /// construction time. If that variable was not set, falls back to
    /// `credentials.private_key` for backward compatibility.
    fn auth_header(&self) -> String {
        let token = self
            .access_token
            .as_deref()
            .unwrap_or(&self.credentials.private_key);
        format!("Bearer {token}")
    }

    /// Returns the project ID from the loaded credentials.
    pub fn project_id(&self) -> &str {
        &self.credentials.project_id
    }

    /// Returns the resolved FCM API URL.
    pub fn api_url(&self) -> &str {
        &self.api_url
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_credentials() -> ServiceAccountCredentials {
        ServiceAccountCredentials {
            project_id: "test-project-123".to_string(),
            client_email: "test@test-project-123.iam.gserviceaccount.com".to_string(),
            private_key: "test-private-key".to_string(),
        }
    }

    #[test]
    fn build_message_with_notification() {
        let msg = FcmClient::build_message("device-token-abc", "Hello", "World", None);

        assert_eq!(msg.message.token, "device-token-abc");
        let notif = msg.message.notification.as_ref().unwrap();
        assert_eq!(notif.title, "Hello");
        assert_eq!(notif.body, "World");
        assert!(msg.message.data.is_none());
    }

    #[test]
    fn build_message_with_data() {
        let mut data = std::collections::HashMap::new();
        data.insert("key1".to_string(), "value1".to_string());
        data.insert("key2".to_string(), "value2".to_string());

        let msg = FcmClient::build_message("token-xyz", "Title", "Body", Some(data));

        assert_eq!(msg.message.token, "token-xyz");
        let d = msg.message.data.as_ref().unwrap();
        assert_eq!(d.get("key1").unwrap(), "value1");
        assert_eq!(d.get("key2").unwrap(), "value2");
    }

    #[test]
    fn from_credentials_sets_api_url() {
        let creds = test_credentials();
        let client = FcmClient::for_testing(creds);

        assert_eq!(client.project_id(), "test-project-123");
        assert_eq!(
            client.api_url(),
            "https://fcm.googleapis.com/v1/projects/test-project-123/messages:send"
        );
    }

    #[test]
    fn message_serializes_to_json() {
        let msg = FcmClient::build_message("tok", "T", "B", None);
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["message"]["token"], "tok");
        assert_eq!(json["message"]["notification"]["title"], "T");
        assert_eq!(json["message"]["notification"]["body"], "B");
        // data should be absent (skip_serializing_if = None)
        assert!(json["message"].get("data").is_none());
    }

    #[test]
    fn from_credentials_file_missing_returns_error() {
        let result = FcmClient::from_credentials_file(Path::new("/nonexistent/path.json"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, NotificationError::Credentials(_)),
            "expected Credentials error, got: {err}"
        );
    }

    #[test]
    fn auth_header_uses_access_token_when_set() {
        let client = FcmClient::for_testing(test_credentials());
        // for_testing sets access_token to Some("test-access-token")
        assert_eq!(client.auth_header(), "Bearer test-access-token");
    }

    #[test]
    fn auth_header_falls_back_to_private_key() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder().build().expect("http client");
        let creds = test_credentials();
        let client = FcmClient {
            http,
            credentials: creds,
            api_url: "https://example.com".to_string(),
            access_token: None,
        };
        assert_eq!(client.auth_header(), "Bearer test-private-key");
    }
}
