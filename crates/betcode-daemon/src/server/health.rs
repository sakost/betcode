//! Health service gRPC implementations.
//!
//! Implements both the standard gRPC Health service (compatible with grpc.health.v1)
//! and the BetCode-specific health details service.

use std::pin::Pin;
use std::sync::Arc;

use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use betcode_proto::v1::{
    ComponentHealth, HealthCheckRequest, HealthCheckResponse, HealthDetailsRequest,
    HealthDetailsResponse, ServingStatus, bet_code_health_server::BetCodeHealth,
    health_server::Health,
};

use crate::storage::Database;
use crate::subprocess::SubprocessManager;

/// Health service implementation.
#[derive(Clone)]
pub struct HealthServiceImpl {
    db: Database,
    subprocess_manager: Arc<SubprocessManager>,
}

impl HealthServiceImpl {
    /// Create a new health service.
    pub const fn new(db: Database, subprocess_manager: Arc<SubprocessManager>) -> Self {
        Self {
            db,
            subprocess_manager,
        }
    }

    /// Check if the database is healthy by running a simple query.
    async fn check_db_health(&self) -> ComponentHealth {
        let status = match self.db.list_sessions(None, 1, 0).await {
            Ok(_) => ServingStatus::Serving,
            Err(_) => ServingStatus::NotServing,
        };

        ComponentHealth {
            name: "database".to_string(),
            status: status.into(),
            message: match ServingStatus::try_from(status as i32).unwrap_or(ServingStatus::Unknown)
            {
                ServingStatus::Serving => "SQLite database operational".to_string(),
                _ => "Database query failed".to_string(),
            },
            last_check: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        }
    }

    /// Check subprocess manager health.
    async fn check_subprocess_health(&self) -> ComponentHealth {
        let active = self.subprocess_manager.active_count().await;
        let capacity = self.subprocess_manager.capacity();

        ComponentHealth {
            name: "subprocess_pool".to_string(),
            status: ServingStatus::Serving.into(),
            message: format!("{active}/{capacity} subprocess slots in use"),
            last_check: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        }
    }
}

#[tonic::async_trait]
impl Health for HealthServiceImpl {
    type WatchStream =
        Pin<Box<dyn Stream<Item = Result<HealthCheckResponse, Status>> + Send + 'static>>;

    async fn check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let service = request.into_inner().service;

        // Empty service name means overall health
        let status = if service.is_empty() || service == "betcode.v1.AgentService" {
            // Quick DB probe to confirm we're serving
            match self.db.list_sessions(None, 1, 0).await {
                Ok(_) => ServingStatus::Serving,
                Err(_) => ServingStatus::NotServing,
            }
        } else {
            ServingStatus::ServiceUnknown
        };

        Ok(Response::new(HealthCheckResponse {
            status: status.into(),
        }))
    }

    async fn watch(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let service = request.into_inner().service;
        let db = self.db.clone();

        let stream = async_stream::stream! {
            loop {
                let status = if service.is_empty() || service == "betcode.v1.AgentService" {
                    match db.list_sessions(None, 1, 0).await {
                        Ok(_) => ServingStatus::Serving,
                        Err(_) => ServingStatus::NotServing,
                    }
                } else {
                    ServingStatus::ServiceUnknown
                };

                yield Ok(HealthCheckResponse {
                    status: status.into(),
                });

                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }
}

#[tonic::async_trait]
impl BetCodeHealth for HealthServiceImpl {
    async fn get_health_details(
        &self,
        _request: Request<HealthDetailsRequest>,
    ) -> Result<Response<HealthDetailsResponse>, Status> {
        let db_health = self.check_db_health().await;
        let subprocess_health = self.check_subprocess_health().await;

        let components = vec![db_health, subprocess_health];

        // Overall status: SERVING if all components are SERVING
        let overall = if components
            .iter()
            .all(|c| c.status == ServingStatus::Serving as i32)
        {
            ServingStatus::Serving
        } else {
            ServingStatus::NotServing
        };

        let degraded = components
            .iter()
            .any(|c| c.status != ServingStatus::Serving as i32);
        let degraded_reason = if degraded {
            components
                .iter()
                .filter(|c| c.status != ServingStatus::Serving as i32)
                .map(|c| format!("{}: {}", c.name, c.message))
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            String::new()
        };

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());

        Ok(Response::new(HealthDetailsResponse {
            overall_status: overall.into(),
            components,
            metadata,
            degraded,
            degraded_reason,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_health_service() -> HealthServiceImpl {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        HealthServiceImpl::new(db, subprocess_mgr)
    }

    #[tokio::test]
    async fn health_check_returns_serving() {
        let svc = test_health_service().await;
        let resp = svc
            .check(Request::new(HealthCheckRequest {
                service: String::new(),
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().status, ServingStatus::Serving as i32);
    }

    #[tokio::test]
    async fn health_check_named_service() {
        let svc = test_health_service().await;
        let resp = svc
            .check(Request::new(HealthCheckRequest {
                service: "betcode.v1.AgentService".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().status, ServingStatus::Serving as i32);
    }

    #[tokio::test]
    async fn health_check_unknown_service() {
        let svc = test_health_service().await;
        let resp = svc
            .check(Request::new(HealthCheckRequest {
                service: "nonexistent.Service".to_string(),
            }))
            .await
            .unwrap();
        assert_eq!(
            resp.into_inner().status,
            ServingStatus::ServiceUnknown as i32
        );
    }

    #[tokio::test]
    async fn health_details_all_components_healthy() {
        let svc = test_health_service().await;
        let resp = svc
            .get_health_details(Request::new(HealthDetailsRequest {}))
            .await
            .unwrap();
        let details = resp.into_inner();

        assert_eq!(details.overall_status, ServingStatus::Serving as i32);
        assert!(!details.degraded);
        assert!(details.degraded_reason.is_empty());
        assert_eq!(details.components.len(), 2);

        // Check component names
        let names: Vec<&str> = details.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"database"));
        assert!(names.contains(&"subprocess_pool"));

        // All should be serving
        for comp in &details.components {
            assert_eq!(comp.status, ServingStatus::Serving as i32);
        }
    }

    #[tokio::test]
    async fn health_details_includes_version() {
        let svc = test_health_service().await;
        let resp = svc
            .get_health_details(Request::new(HealthDetailsRequest {}))
            .await
            .unwrap();
        let details = resp.into_inner();
        assert!(details.metadata.contains_key("version"));
    }

    #[tokio::test]
    async fn health_details_subprocess_pool_message() {
        let svc = test_health_service().await;
        let resp = svc
            .get_health_details(Request::new(HealthDetailsRequest {}))
            .await
            .unwrap();
        let details = resp.into_inner();
        let pool = details
            .components
            .iter()
            .find(|c| c.name == "subprocess_pool")
            .unwrap();
        assert!(pool.message.contains("0/5"));
    }
}
