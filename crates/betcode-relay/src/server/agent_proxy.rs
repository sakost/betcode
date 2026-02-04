//! AgentService proxy that forwards Converse calls through the tunnel to daemons.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};
use tracing::error;

use betcode_proto::v1::agent_service_server::AgentService;
use betcode_proto::v1::{
    AgentEvent, AgentRequest, CancelTurnRequest, CancelTurnResponse, CompactSessionRequest,
    CompactSessionResponse, InputLockRequest, InputLockResponse, ListSessionsRequest,
    ListSessionsResponse, ResumeSessionRequest,
};

use crate::router::RequestRouter;
use crate::server::interceptor::extract_claims;

type EventStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<AgentEvent, Status>> + Send>>;

/// Proxies AgentService calls through the tunnel to a target daemon.
pub struct AgentProxyService {
    router: Arc<RequestRouter>,
}

impl AgentProxyService {
    pub fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
    }
}

#[tonic::async_trait]
impl AgentService for AgentProxyService {
    type ConverseStream = EventStream;
    type ResumeSessionStream = EventStream;

    async fn converse(
        &self,
        request: Request<Streaming<AgentRequest>>,
    ) -> Result<Response<Self::ConverseStream>, Status> {
        let _claims = {
            let c = extract_claims(&request)?;
            c.sub.clone()
        };

        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);
        let _router = Arc::clone(&self.router);

        tokio::spawn(async move {
            // Read the first message to determine target machine
            match in_stream.next().await {
                Some(Ok(_req)) => {
                    // Proxy is a placeholder - full routing requires
                    // machine_id in request metadata or session-to-machine mapping
                    let _ = tx
                        .send(Err(Status::unimplemented(
                            "Converse proxy requires machine routing (Sprint 3.8)",
                        )))
                        .await;
                }
                Some(Err(e)) => {
                    error!(error = %e, "Proxy converse stream error");
                }
                None => {}
            }
        });

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(out_stream)))
    }

    async fn list_sessions(
        &self,
        _request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        Err(Status::unimplemented(
            "ListSessions proxy not yet implemented",
        ))
    }

    async fn resume_session(
        &self,
        _request: Request<ResumeSessionRequest>,
    ) -> Result<Response<Self::ResumeSessionStream>, Status> {
        Err(Status::unimplemented(
            "ResumeSession proxy not yet implemented",
        ))
    }

    async fn compact_session(
        &self,
        _request: Request<CompactSessionRequest>,
    ) -> Result<Response<CompactSessionResponse>, Status> {
        Err(Status::unimplemented(
            "CompactSession proxy not yet implemented",
        ))
    }

    async fn cancel_turn(
        &self,
        _request: Request<CancelTurnRequest>,
    ) -> Result<Response<CancelTurnResponse>, Status> {
        Err(Status::unimplemented(
            "CancelTurn proxy not yet implemented",
        ))
    }

    async fn request_input_lock(
        &self,
        _request: Request<InputLockRequest>,
    ) -> Result<Response<InputLockResponse>, Status> {
        Err(Status::unimplemented(
            "RequestInputLock proxy not yet implemented",
        ))
    }
}
