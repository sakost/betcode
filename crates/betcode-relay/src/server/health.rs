//! `betcode.v1.Health` service for the relay.
//!
//! Provides a simple health check that the app uses to verify TCP connectivity
//! after resuming from background. The relay already exposes the standard
//! `grpc.health.v1.Health` for load balancers; this covers the custom
//! `betcode.v1.Health` path that the Flutter client calls.

use std::pin::Pin;

use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use betcode_proto::v1::{
    HealthCheckRequest, HealthCheckResponse, ServingStatus, health_server::Health,
};

/// Relay-side implementation of `betcode.v1.Health`.
#[derive(Clone, Default)]
pub struct RelayHealthService;

impl RelayHealthService {
    pub const fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl Health for RelayHealthService {
    type WatchStream =
        Pin<Box<dyn Stream<Item = Result<HealthCheckResponse, Status>> + Send + 'static>>;

    async fn check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        // If this handler runs, the relay is alive and accepting gRPC.
        Ok(Response::new(HealthCheckResponse {
            status: ServingStatus::Serving.into(),
        }))
    }

    async fn watch(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        Err(Status::unimplemented(
            "Health.Watch is not supported on the relay",
        ))
    }
}
