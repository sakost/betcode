//! `AgentService` gRPC implementation.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status, Streaming};
use tracing::{error, info, instrument, warn};

use betcode_proto::v1::{
    AgentEvent, AgentRequest, CancelTurnRequest, CancelTurnResponse, ClearSessionGrantsRequest,
    ClearSessionGrantsResponse, CompactSessionRequest, CompactSessionResponse, InputLockRequest,
    InputLockResponse, KeyExchangeRequest, KeyExchangeResponse, ListSessionGrantsRequest,
    ListSessionGrantsResponse, ListSessionsRequest, ListSessionsResponse, RenameSessionRequest,
    RenameSessionResponse, ResumeSessionRequest, SessionSummary, SetSessionGrantRequest,
    SetSessionGrantResponse, agent_service_server::AgentService,
};

use super::handler::{HandlerContext, handle_agent_request};
use crate::relay::SessionRelay;
use crate::session::SessionMultiplexer;
use crate::storage::{Database, DatabaseError};

/// `AgentService` implementation backed by `SessionRelay`.
pub struct AgentServiceImpl {
    db: Database,
    relay: Arc<SessionRelay>,
    multiplexer: Arc<SessionMultiplexer>,
}

impl AgentServiceImpl {
    /// Create a new `AgentService`.
    pub const fn new(
        db: Database,
        relay: Arc<SessionRelay>,
        multiplexer: Arc<SessionMultiplexer>,
    ) -> Self {
        Self {
            db,
            relay,
            multiplexer,
        }
    }
}

impl AgentServiceImpl {
    /// Look up a session handle from the relay, returning a gRPC `NOT_FOUND`
    /// status if the session is not currently active.
    async fn require_session_handle(
        &self,
        session_id: &str,
    ) -> Result<crate::relay::RelayHandle, Status> {
        self.relay
            .get_handle(session_id)
            .await
            .ok_or_else(|| Status::not_found(format!("Session {session_id} not active")))
    }
}

type AgentEventStream = Pin<Box<dyn Stream<Item = Result<AgentEvent, Status>> + Send>>;

#[tonic::async_trait]
impl AgentService for AgentServiceImpl {
    type ConverseStream = AgentEventStream;
    type ResumeSessionStream = AgentEventStream;

    #[instrument(skip(self, request), fields(rpc = "Converse"))]
    async fn converse(
        &self,
        request: Request<Streaming<AgentRequest>>,
    ) -> Result<Response<Self::ConverseStream>, Status> {
        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);

        let relay = Arc::clone(&self.relay);
        let multiplexer = Arc::clone(&self.multiplexer);
        let db = self.db.clone();

        tokio::spawn(async move {
            let client_id = uuid::Uuid::new_v4().to_string();
            let mut session_id: Option<String> = None;

            while let Some(result) = in_stream.next().await {
                match result {
                    Ok(req) => {
                        let handler_ctx = HandlerContext {
                            relay: &relay,
                            multiplexer: &multiplexer,
                            db: &db,
                            tx: &tx,
                            client_id: &client_id,
                        };
                        if let Err(e) =
                            handle_agent_request(&handler_ctx, &mut session_id, req).await
                        {
                            error!(?e, "Error handling agent request");
                            let _ = tx.send(Err(Status::internal(e.to_string()))).await;
                            break;
                        }
                    }
                    Err(e) => {
                        error!(?e, "Stream error");
                        break;
                    }
                }
            }

            if let Some(ref sid) = session_id {
                multiplexer.unsubscribe(sid, &client_id).await;
            }
            info!(client_id, "Converse stream ended");
        });

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(out_stream)))
    }

    #[instrument(skip(self, request), fields(rpc = "ListSessions"))]
    async fn list_sessions(
        &self,
        request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let req = request.into_inner();
        let working_dir = if req.working_directory.is_empty() {
            None
        } else {
            Some(req.working_directory.as_str())
        };
        let limit = if req.limit == 0 { 50 } else { req.limit };

        let sessions = self
            .db
            .list_sessions(working_dir, limit, req.offset)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let summaries: Vec<SessionSummary> =
            sessions.into_iter().map(SessionSummary::from).collect();

        #[allow(clippy::cast_possible_truncation)]
        let total = summaries.len() as u32;
        Ok(Response::new(ListSessionsResponse {
            sessions: summaries,
            total,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "ResumeSession"))]
    async fn resume_session(
        &self,
        request: Request<ResumeSessionRequest>,
    ) -> Result<Response<Self::ResumeSessionStream>, Status> {
        let req = request.into_inner();
        #[allow(clippy::cast_possible_wrap)]
        let from_seq = req.from_sequence as i64;
        let messages = self
            .db
            .get_messages_from_sequence(&req.session_id, from_seq)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);

        tokio::spawn(async move {
            for msg in messages {
                // Payload is base64-encoded protobuf bytes
                let bytes = match base64_decode(&msg.payload) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!(error = %e, "Failed to decode base64 payload");
                        continue;
                    }
                };

                match prost::Message::decode(bytes.as_slice()) {
                    Ok(event) => {
                        if tx.send(Ok(event)).await.is_err() {
                            warn!("Resume stream receiver dropped");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(?e, "Failed to decode stored message");
                        let err_event = AgentEvent {
                            sequence: 0,
                            timestamp: None,
                            parent_tool_use_id: String::new(),
                            event: Some(betcode_proto::v1::agent_event::Event::Error(
                                betcode_proto::v1::ErrorEvent {
                                    code: "DECODE_ERROR".to_string(),
                                    message: format!("Decode error: {e}"),
                                    is_fatal: false,
                                    details: HashMap::default(),
                                },
                            )),
                        };
                        if tx.send(Ok(err_event)).await.is_err() {
                            break;
                        }
                    }
                }
            }
            info!(session_id = %req.session_id, "Resume replay completed");
        });

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(out_stream)))
    }

    #[instrument(skip(self, request), fields(rpc = "CompactSession"))]
    async fn compact_session(
        &self,
        request: Request<CompactSessionRequest>,
    ) -> Result<Response<CompactSessionResponse>, Status> {
        let req = request.into_inner();
        let sid = &req.session_id;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let messages_before = self
            .db
            .count_messages(sid)
            .await
            .map_err(|e| Status::internal(e.to_string()))? as u32;

        if messages_before == 0 {
            return Ok(Response::new(CompactSessionResponse {
                messages_before: 0,
                messages_after: 0,
                tokens_saved: 0,
            }));
        }

        // Keep the most recent half of messages (at least 10)
        let max_seq = self
            .db
            .max_message_sequence(sid)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let keep_count = (messages_before / 2).max(10).min(messages_before);
        let cutoff = max_seq - i64::from(keep_count);

        if cutoff <= 0 {
            return Ok(Response::new(CompactSessionResponse {
                messages_before,
                messages_after: messages_before,
                tokens_saved: 0,
            }));
        }

        let deleted = self
            .db
            .delete_messages_before_sequence(sid, cutoff)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        self.db
            .update_compaction_sequence(sid, cutoff)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        #[allow(clippy::cast_possible_truncation)]
        let messages_after = messages_before - deleted as u32;
        // Rough estimate: ~100 tokens per message on average
        #[allow(clippy::cast_possible_truncation)]
        let tokens_saved = deleted as u32 * 100;

        info!(
            session_id = %sid,
            messages_before,
            messages_after,
            tokens_saved,
            "Session compacted"
        );

        Ok(Response::new(CompactSessionResponse {
            messages_before,
            messages_after,
            tokens_saved,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "CancelTurn"))]
    async fn cancel_turn(
        &self,
        request: Request<CancelTurnRequest>,
    ) -> Result<Response<CancelTurnResponse>, Status> {
        let req = request.into_inner();
        let was_active = self
            .relay
            .cancel_session(&req.session_id)
            .await
            .unwrap_or(false);
        Ok(Response::new(CancelTurnResponse { was_active }))
    }

    #[instrument(skip(self, request), fields(rpc = "RequestInputLock"))]
    async fn request_input_lock(
        &self,
        request: Request<InputLockRequest>,
    ) -> Result<Response<InputLockResponse>, Status> {
        // Extract client_id from metadata before consuming the request
        let client_id = request_client_id(&request)
            .unwrap_or_else(|| format!("unary-{}", uuid::Uuid::new_v4()));
        let req = request.into_inner();
        let sid = &req.session_id;

        let previous = self
            .db
            .acquire_input_lock(sid, &client_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        info!(
            session_id = %sid,
            client_id = %client_id,
            previous_holder = ?previous,
            "Input lock acquired"
        );

        Ok(Response::new(InputLockResponse {
            granted: true,
            previous_holder: previous.unwrap_or_default(),
        }))
    }

    #[instrument(skip(self, _request), fields(rpc = "ExchangeKeys"))]
    async fn exchange_keys(
        &self,
        _request: Request<KeyExchangeRequest>,
    ) -> Result<Response<KeyExchangeResponse>, Status> {
        // Key exchange is only meaningful for remote (tunneled) connections.
        // Direct local gRPC connections don't need E2E encryption.
        Err(Status::unimplemented(
            "ExchangeKeys is only supported over tunnel connections",
        ))
    }

    #[instrument(skip(self, request), fields(rpc = "ListSessionGrants"))]
    async fn list_session_grants(
        &self,
        request: Request<ListSessionGrantsRequest>,
    ) -> Result<Response<ListSessionGrantsResponse>, Status> {
        let req = request.into_inner();
        let handle = self.require_session_handle(&req.session_id).await?;

        let entries = handle.list_grants().await;
        Ok(Response::new(ListSessionGrantsResponse { grants: entries }))
    }

    #[instrument(skip(self, request), fields(rpc = "ClearSessionGrants"))]
    async fn clear_session_grants(
        &self,
        request: Request<ClearSessionGrantsRequest>,
    ) -> Result<Response<ClearSessionGrantsResponse>, Status> {
        let req = request.into_inner();
        let handle = self.require_session_handle(&req.session_id).await?;

        handle.clear_grants(&req.tool_name).await;
        if req.tool_name.is_empty() {
            info!(session_id = %req.session_id, "Cleared all session grants");
        } else {
            info!(session_id = %req.session_id, tool_name = %req.tool_name, "Cleared session grant");
        }

        Ok(Response::new(ClearSessionGrantsResponse {}))
    }

    #[instrument(skip(self, request), fields(rpc = "SetSessionGrant"))]
    async fn set_session_grant(
        &self,
        request: Request<SetSessionGrantRequest>,
    ) -> Result<Response<SetSessionGrantResponse>, Status> {
        let req = request.into_inner();
        if req.tool_name.is_empty() {
            return Err(Status::invalid_argument("tool_name must not be empty"));
        }
        let handle = self.require_session_handle(&req.session_id).await?;

        handle.set_grant(req.tool_name.clone(), req.granted).await;
        info!(
            session_id = %req.session_id,
            tool_name = %req.tool_name,
            granted = req.granted,
            "Set session grant"
        );

        Ok(Response::new(SetSessionGrantResponse {}))
    }

    #[instrument(skip(self, request), fields(rpc = "RenameSession"))]
    async fn rename_session(
        &self,
        request: Request<RenameSessionRequest>,
    ) -> Result<Response<RenameSessionResponse>, Status> {
        let req = request.into_inner();
        self.db
            .update_session_name(&req.session_id, &req.name)
            .await
            .map_err(|e| match e {
                DatabaseError::NotFound(_) => Status::not_found(e.to_string()),
                _ => Status::internal(e.to_string()),
            })?;

        info!(session_id = %req.session_id, name = %req.name, "Session renamed");

        Ok(Response::new(RenameSessionResponse {}))
    }
}

/// Extract `client_id` from gRPC request metadata.
fn request_client_id<T>(request: &Request<T>) -> Option<String> {
    request
        .metadata()
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string)
}

use betcode_core::db::base64_decode;

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `AgentServiceImpl` backed by an in-memory DB.
    async fn test_agent_service() -> AgentServiceImpl {
        let tc = crate::testutil::test_components().await;
        AgentServiceImpl::new(tc.db, tc.relay, tc.multiplexer)
    }

    #[tokio::test]
    async fn agent_service_creation() {
        let _service = test_agent_service().await;
    }

    #[tokio::test]
    async fn exchange_keys_returns_unimplemented() {
        let service = test_agent_service().await;

        let req = Request::new(KeyExchangeRequest {
            machine_id: "m1".into(),
            identity_pubkey: Vec::new(),
            fingerprint: String::new(),
            ephemeral_pubkey: vec![0u8; 32],
        });
        let err = service.exchange_keys(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}
