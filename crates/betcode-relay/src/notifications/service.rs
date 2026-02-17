//! `NotificationService` gRPC implementation.
//!
//! Provides device token registration and unregistration for push
//! notifications. Device tokens are stored in the relay database.

use std::collections::HashMap;

use tonic::{Request, Response, Status};
use tracing::{info, instrument, warn};

use betcode_proto::v1::notification_service_server::NotificationService;
use betcode_proto::v1::{
    DevicePlatform, RegisterDeviceRequest, RegisterDeviceResponse, UnregisterDeviceRequest,
    UnregisterDeviceResponse,
};

use crate::storage::RelayDatabase;

use super::{FcmClient, NotificationError};

/// gRPC service for managing device token registrations.
pub struct NotificationServiceImpl {
    db: RelayDatabase,
    fcm: FcmClient,
}

impl NotificationServiceImpl {
    /// Create a new `NotificationServiceImpl`.
    pub const fn new(db: RelayDatabase, fcm: FcmClient) -> Self {
        Self { db, fcm }
    }

    /// Send a push notification to a device.
    ///
    /// Builds an FCM message from the given parameters and delegates to the
    /// underlying [`FcmClient`].
    ///
    /// # Errors
    ///
    /// Returns [`NotificationError`] if the FCM request fails or the API
    /// returns a non-success status.
    #[instrument(skip(self, data), fields(device_token))]
    pub async fn send_notification(
        &self,
        device_token: &str,
        title: &str,
        body: &str,
        data: Option<HashMap<String, String>>,
    ) -> Result<(), NotificationError> {
        let msg = FcmClient::build_message(device_token, title, body, data);
        self.fcm.send(&msg).await
    }
}

/// Convert a `DevicePlatform` enum value to a database string.
fn platform_to_str(platform: i32) -> Result<&'static str, Status> {
    match DevicePlatform::try_from(platform) {
        Ok(DevicePlatform::Android) => Ok("android"),
        Ok(DevicePlatform::Ios) => Ok("ios"),
        Ok(DevicePlatform::Unspecified) | Err(_) => {
            Err(Status::invalid_argument("Platform must be ANDROID or IOS"))
        }
    }
}

#[tonic::async_trait]
impl NotificationService for NotificationServiceImpl {
    #[instrument(skip(self, request), fields(rpc = "RegisterDevice"))]
    async fn register_device(
        &self,
        request: Request<RegisterDeviceRequest>,
    ) -> Result<Response<RegisterDeviceResponse>, Status> {
        let req = request.into_inner();

        if req.device_token.is_empty() {
            return Err(Status::invalid_argument("device_token is required"));
        }
        if req.user_id.is_empty() {
            return Err(Status::invalid_argument("user_id is required"));
        }

        let platform = platform_to_str(req.platform)?;

        let id = uuid::Uuid::new_v4().to_string();

        match self
            .db
            .upsert_device_token(&id, &req.user_id, &req.device_token, platform)
            .await
        {
            Ok(()) => {
                info!(
                    user_id = %req.user_id,
                    platform = platform,
                    "Device token registered"
                );
                Ok(Response::new(RegisterDeviceResponse { success: true }))
            }
            Err(e) => {
                warn!(error = %e, "Failed to register device token");
                Err(Status::internal(format!(
                    "Failed to register device token: {e}"
                )))
            }
        }
    }

    #[instrument(skip(self, request), fields(rpc = "UnregisterDevice"))]
    async fn unregister_device(
        &self,
        request: Request<UnregisterDeviceRequest>,
    ) -> Result<Response<UnregisterDeviceResponse>, Status> {
        let req = request.into_inner();

        if req.device_token.is_empty() {
            return Err(Status::invalid_argument("device_token is required"));
        }

        match self.db.delete_device_token(&req.device_token).await {
            Ok(removed) => {
                if removed {
                    info!(device_token_prefix = %&req.device_token[..req.device_token.len().min(8)], "Device token unregistered");
                } else {
                    info!("Device token not found (already removed)");
                }
                Ok(Response::new(UnregisterDeviceResponse { success: removed }))
            }
            Err(e) => {
                warn!(error = %e, "Failed to unregister device token");
                Err(Status::internal(format!(
                    "Failed to unregister device token: {e}"
                )))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::notifications::fcm::ServiceAccountCredentials;

    fn register_req(
        token: &str,
        platform: DevicePlatform,
        user_id: &str,
    ) -> Request<RegisterDeviceRequest> {
        Request::new(RegisterDeviceRequest {
            device_token: token.to_string(),
            platform: platform as i32,
            user_id: user_id.to_string(),
        })
    }

    async fn test_service() -> NotificationServiceImpl {
        let db = RelayDatabase::open_in_memory().await.unwrap();
        let creds = ServiceAccountCredentials {
            project_id: "test-project".to_string(),
            client_email: "test@test.iam.gserviceaccount.com".to_string(),
            private_key: "test-key".to_string(),
        };
        let fcm = FcmClient::for_testing(creds);
        NotificationServiceImpl::new(db, fcm)
    }

    #[tokio::test]
    async fn register_device_success() {
        let svc = test_service().await;
        let resp = svc
            .register_device(register_req(
                "token-abc-123",
                DevicePlatform::Android,
                "user-1",
            ))
            .await
            .unwrap();
        assert!(resp.into_inner().success);
    }

    // jscpd:ignore-start -- validation tests are intentionally repetitive
    #[tokio::test]
    async fn register_device_empty_token_fails() {
        let svc = test_service().await;
        let req = Request::new(RegisterDeviceRequest {
            device_token: String::new(),
            platform: DevicePlatform::Android as i32,
            user_id: "user-1".to_string(),
        });

        let err = svc.register_device(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("device_token"));
    }

    #[tokio::test]
    async fn register_device_empty_user_id_fails() {
        let svc = test_service().await;
        let req = Request::new(RegisterDeviceRequest {
            device_token: "token-xyz".to_string(),
            platform: DevicePlatform::Android as i32,
            user_id: String::new(),
        });

        let err = svc.register_device(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("user_id"));
    }

    #[tokio::test]
    async fn register_device_unspecified_platform_fails() {
        let svc = test_service().await;
        let req = Request::new(RegisterDeviceRequest {
            device_token: "token-xyz".to_string(),
            platform: DevicePlatform::Unspecified as i32,
            user_id: "user-1".to_string(),
        });

        let err = svc.register_device(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("Platform"));
    }
    // jscpd:ignore-end

    #[tokio::test]
    async fn register_device_ios_platform_success() {
        let svc = test_service().await;
        let resp = svc
            .register_device(register_req("ios-token-456", DevicePlatform::Ios, "user-2"))
            .await
            .unwrap();
        assert!(resp.into_inner().success);
    }

    #[tokio::test]
    async fn unregister_device_success() {
        let svc = test_service().await;

        // First register
        svc.register_device(register_req(
            "token-to-remove",
            DevicePlatform::Android,
            "user-1",
        ))
        .await
        .unwrap();

        // Then unregister
        let req = Request::new(UnregisterDeviceRequest {
            device_token: "token-to-remove".to_string(),
        });
        let resp = svc.unregister_device(req).await.unwrap();
        assert!(resp.into_inner().success);
    }

    #[tokio::test]
    async fn unregister_device_not_found() {
        let svc = test_service().await;
        let req = Request::new(UnregisterDeviceRequest {
            device_token: "nonexistent-token".to_string(),
        });

        let resp = svc.unregister_device(req).await.unwrap();
        assert!(!resp.into_inner().success);
    }

    #[tokio::test]
    async fn unregister_device_empty_token_fails() {
        let svc = test_service().await;
        let req = Request::new(UnregisterDeviceRequest {
            device_token: String::new(),
        });

        let err = svc.unregister_device(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("device_token"));
    }

    #[tokio::test]
    async fn send_notification_builds_and_delegates() {
        // The actual HTTP call will fail because there is no real FCM server,
        // but we can verify the method is callable and returns a request error
        // (not a panic or compile error).
        let svc = test_service().await;
        let result = svc
            .send_notification("device-tok", "Hello", "World", None)
            .await;

        // We expect an error because no real FCM endpoint is reachable in tests.
        assert!(result.is_err(), "expected an error from unreachable FCM");
    }

    #[tokio::test]
    async fn send_notification_with_data_builds_and_delegates() {
        let svc = test_service().await;
        let mut data = std::collections::HashMap::new();
        data.insert("key".to_string(), "value".to_string());

        let result = svc
            .send_notification("device-tok", "Title", "Body", Some(data))
            .await;

        assert!(result.is_err(), "expected an error from unreachable FCM");
    }

    #[tokio::test]
    async fn register_device_upsert_same_token() {
        let svc = test_service().await;

        // Register with user-1
        svc.register_device(register_req(
            "shared-token",
            DevicePlatform::Android,
            "user-1",
        ))
        .await
        .unwrap();

        // Re-register same token with user-2 (should upsert)
        let resp = svc
            .register_device(register_req("shared-token", DevicePlatform::Ios, "user-2"))
            .await
            .unwrap();
        assert!(resp.into_inner().success);

        // Unregister should succeed (token still exists)
        let req = Request::new(UnregisterDeviceRequest {
            device_token: "shared-token".to_string(),
        });
        let resp = svc.unregister_device(req).await.unwrap();
        assert!(resp.into_inner().success);

        // Second unregister should report not found
        let req = Request::new(UnregisterDeviceRequest {
            device_token: "shared-token".to_string(),
        });
        let resp = svc.unregister_device(req).await.unwrap();
        assert!(!resp.into_inner().success);
    }
}
