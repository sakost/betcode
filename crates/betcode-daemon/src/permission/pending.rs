//! Pending permission request manager.
//!
//! Tracks permission requests that are awaiting client response.
//! Supports tiered timeouts: short for connected, long for disconnected.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for pending permission manager.
#[derive(Debug, Clone)]
pub struct PendingConfig {
    /// Timeout for connected clients.
    pub connected_timeout: Duration,
    /// Timeout for disconnected clients (grants preserved).
    pub disconnected_timeout: Duration,
    /// Cleanup interval for expired requests.
    pub cleanup_interval: Duration,
}

impl Default for PendingConfig {
    fn default() -> Self {
        Self {
            connected_timeout: Duration::from_secs(60),
            disconnected_timeout: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

/// A pending permission request.
#[derive(Debug, Clone)]
pub struct PendingRequest {
    /// Unique request ID.
    pub request_id: String,
    /// Session this request belongs to.
    pub session_id: String,
    /// Tool name being requested.
    pub tool_name: String,
    /// Tool description.
    pub description: String,
    /// Tool input (as JSON string).
    pub input_json: String,
    /// Client that should respond (if any).
    pub target_client: Option<String>,
    /// When the request was created.
    pub created_at: Instant,
    /// When the request expires.
    pub expires_at: Instant,
    /// Whether the client is connected.
    pub client_connected: bool,
}

impl PendingRequest {
    /// Check if the request has expired.
    pub fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }

    /// Refresh expiry based on client connection status.
    pub fn refresh_expiry(&mut self, connected: bool, config: &PendingConfig) {
        self.client_connected = connected;
        let timeout = if connected {
            config.connected_timeout
        } else {
            config.disconnected_timeout
        };
        self.expires_at = Instant::now() + timeout;
    }
}

/// Parameters for creating a new pending request.
pub struct PendingRequestParams {
    /// Unique request ID.
    pub request_id: String,
    /// Session this request belongs to.
    pub session_id: String,
    /// Tool name being requested.
    pub tool_name: String,
    /// Tool description.
    pub description: String,
    /// Tool input (as JSON string).
    pub input_json: String,
    /// Client that should respond (if any).
    pub target_client: Option<String>,
    /// Whether the client is connected.
    pub client_connected: bool,
}

/// Manager for pending permission requests.
pub struct PendingManager {
    /// Pending requests keyed by request_id.
    requests: Arc<RwLock<HashMap<String, PendingRequest>>>,
    /// Configuration.
    config: PendingConfig,
}

impl PendingManager {
    /// Create a new pending manager.
    pub fn new(config: PendingConfig) -> Self {
        Self {
            requests: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(PendingConfig::default())
    }

    /// Add a pending request.
    pub async fn add(&self, request: PendingRequest) {
        let request_id = request.request_id.clone();
        self.requests
            .write()
            .await
            .insert(request_id.clone(), request);
        debug!(request_id, "Added pending permission request");
    }

    /// Create and add a new pending request.
    pub async fn create(&self, params: PendingRequestParams) -> PendingRequest {
        let timeout = if params.client_connected {
            self.config.connected_timeout
        } else {
            self.config.disconnected_timeout
        };

        let request = PendingRequest {
            request_id: params.request_id,
            session_id: params.session_id,
            tool_name: params.tool_name,
            description: params.description,
            input_json: params.input_json,
            target_client: params.target_client,
            created_at: Instant::now(),
            expires_at: Instant::now() + timeout,
            client_connected: params.client_connected,
        };

        self.add(request.clone()).await;
        request
    }

    /// Get a pending request by ID.
    pub async fn get(&self, request_id: &str) -> Option<PendingRequest> {
        self.requests.read().await.get(request_id).cloned()
    }

    /// Remove and return a pending request.
    pub async fn take(&self, request_id: &str) -> Option<PendingRequest> {
        let request = self.requests.write().await.remove(request_id);
        if request.is_some() {
            debug!(request_id, "Removed pending permission request");
        }
        request
    }

    /// Check if a request exists.
    pub async fn contains(&self, request_id: &str) -> bool {
        self.requests.read().await.contains_key(request_id)
    }

    /// Get all pending requests for a session.
    pub async fn get_for_session(&self, session_id: &str) -> Vec<PendingRequest> {
        self.requests
            .read()
            .await
            .values()
            .filter(|r| r.session_id == session_id)
            .cloned()
            .collect()
    }

    /// Update client connection status for all requests targeting a client.
    pub async fn update_client_status(&self, client_id: &str, connected: bool) {
        let mut requests = self.requests.write().await;
        for request in requests.values_mut() {
            if request.target_client.as_deref() == Some(client_id) {
                request.refresh_expiry(connected, &self.config);
            }
        }
        info!(client_id, connected, "Updated pending request timeouts");
    }

    /// Clean up expired requests.
    pub async fn cleanup_expired(&self) -> Vec<String> {
        let mut requests = self.requests.write().await;
        let expired: Vec<String> = requests
            .iter()
            .filter(|(_, r)| r.is_expired())
            .map(|(id, _)| id.clone())
            .collect();

        for id in &expired {
            requests.remove(id);
            warn!(request_id = %id, "Permission request expired");
        }

        expired
    }

    /// Get count of pending requests.
    pub async fn count(&self) -> usize {
        self.requests.read().await.len()
    }

    /// Get the configuration.
    pub fn config(&self) -> &PendingConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_pending_request() {
        let manager = PendingManager::with_defaults();

        let request = manager
            .create(PendingRequestParams {
                request_id: "req-1".to_string(),
                session_id: "session-1".to_string(),
                tool_name: "Bash".to_string(),
                description: "Run ls command".to_string(),
                input_json: r#"{"command": "ls"}"#.to_string(),
                target_client: Some("client-1".to_string()),
                client_connected: true,
            })
            .await;

        assert_eq!(request.request_id, "req-1");
        assert!(request.client_connected);
        assert!(!request.is_expired());
    }

    #[tokio::test]
    async fn take_removes_request() {
        let manager = PendingManager::with_defaults();

        manager
            .create(PendingRequestParams {
                request_id: "req-1".to_string(),
                session_id: "session-1".to_string(),
                tool_name: "Bash".to_string(),
                description: "desc".to_string(),
                input_json: "{}".to_string(),
                target_client: None,
                client_connected: true,
            })
            .await;

        assert!(manager.contains("req-1").await);
        let taken = manager.take("req-1").await;
        assert!(taken.is_some());
        assert!(!manager.contains("req-1").await);
    }

    #[tokio::test]
    async fn get_for_session() {
        let manager = PendingManager::with_defaults();

        manager
            .create(PendingRequestParams {
                request_id: "req-1".to_string(),
                session_id: "session-1".to_string(),
                tool_name: "Bash".to_string(),
                description: String::new(),
                input_json: "{}".to_string(),
                target_client: None,
                client_connected: true,
            })
            .await;
        manager
            .create(PendingRequestParams {
                request_id: "req-2".to_string(),
                session_id: "session-1".to_string(),
                tool_name: "Write".to_string(),
                description: String::new(),
                input_json: "{}".to_string(),
                target_client: None,
                client_connected: true,
            })
            .await;
        manager
            .create(PendingRequestParams {
                request_id: "req-3".to_string(),
                session_id: "session-2".to_string(),
                tool_name: "Edit".to_string(),
                description: String::new(),
                input_json: "{}".to_string(),
                target_client: None,
                client_connected: true,
            })
            .await;

        let session1_requests = manager.get_for_session("session-1").await;
        assert_eq!(session1_requests.len(), 2);
    }

    #[tokio::test]
    async fn expired_request_cleanup() {
        let config = PendingConfig {
            connected_timeout: Duration::from_millis(1),
            ..Default::default()
        };
        let manager = PendingManager::new(config);

        manager
            .create(PendingRequestParams {
                request_id: "req-1".to_string(),
                session_id: "session-1".to_string(),
                tool_name: "Bash".to_string(),
                description: String::new(),
                input_json: "{}".to_string(),
                target_client: None,
                client_connected: true,
            })
            .await;

        // Wait for expiry
        tokio::time::sleep(Duration::from_millis(10)).await;

        let expired = manager.cleanup_expired().await;
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], "req-1");
        assert_eq!(manager.count().await, 0);
    }
}
