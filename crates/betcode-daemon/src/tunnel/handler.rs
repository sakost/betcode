//! Tunnel request handler that routes incoming frames to local services.

use std::collections::HashMap;

use tracing::{debug, error, warn};

use betcode_proto::v1::{FrameType, StreamPayload, TunnelError, TunnelErrorCode, TunnelFrame};

/// Handles incoming tunnel frames by dispatching to local gRPC services.
///
/// In the current implementation, the handler decodes incoming request frames
/// and produces response frames. Full local service routing (calling the actual
/// AgentService, etc.) will be wired in Sprint 3.8 when end-to-end proxying is
/// implemented. For now, this provides the frame-level protocol handling.
pub struct TunnelRequestHandler {
    /// Machine ID for this daemon.
    machine_id: String,
}

impl TunnelRequestHandler {
    pub fn new(machine_id: String) -> Self {
        Self { machine_id }
    }

    /// Process an incoming request frame and produce a response frame.
    ///
    /// Returns `None` for frames that don't require a response (e.g., control frames).
    pub fn handle_frame(&self, frame: TunnelFrame) -> Option<TunnelFrame> {
        let request_id = frame.request_id.clone();

        match FrameType::try_from(frame.frame_type) {
            Ok(FrameType::Request) => self.handle_request(request_id, frame),
            Ok(FrameType::Control) => {
                debug!(request_id = %request_id, "Received control frame");
                None // Control frames handled at tunnel level
            }
            Ok(FrameType::Error) => {
                warn!(
                    request_id = %request_id,
                    "Received error frame from relay"
                );
                None
            }
            Ok(frame_type) => {
                warn!(
                    request_id = %request_id,
                    frame_type = ?frame_type,
                    "Unexpected frame type received by daemon"
                );
                Some(Self::error_response(
                    &request_id,
                    TunnelErrorCode::Internal,
                    &format!("Unexpected frame type: {:?}", frame_type),
                ))
            }
            Err(_) => {
                error!(
                    request_id = %request_id,
                    frame_type = frame.frame_type,
                    "Unknown frame type"
                );
                Some(Self::error_response(
                    &request_id,
                    TunnelErrorCode::Internal,
                    "Unknown frame type",
                ))
            }
        }
    }

    /// Handle a request frame by routing to the appropriate local service.
    fn handle_request(&self, request_id: String, frame: TunnelFrame) -> Option<TunnelFrame> {
        let payload = match frame.payload {
            Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) => p,
            _ => {
                return Some(Self::error_response(
                    &request_id,
                    TunnelErrorCode::Internal,
                    "Request frame missing StreamPayload",
                ));
            }
        };

        debug!(
            request_id = %request_id,
            method = %payload.method,
            data_len = payload.data.len(),
            machine_id = %self.machine_id,
            "Handling tunneled request"
        );

        // Route based on method name. Full routing to local gRPC services
        // will be implemented in Sprint 3.8. For now, return an acknowledgement
        // that the request was received and processed.
        Some(TunnelFrame {
            request_id,
            frame_type: FrameType::Response as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                StreamPayload {
                    method: payload.method,
                    data: vec![], // Response data will come from local service
                    sequence: 0,
                    metadata: HashMap::new(),
                },
            )),
        })
    }

    /// Create an error response frame.
    fn error_response(request_id: &str, code: TunnelErrorCode, message: &str) -> TunnelFrame {
        TunnelFrame {
            request_id: request_id.to_string(),
            frame_type: FrameType::Error as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::Error(
                TunnelError {
                    code: code as i32,
                    message: message.to_string(),
                    details: HashMap::new(),
                },
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handler() -> TunnelRequestHandler {
        TunnelRequestHandler::new("test-machine".into())
    }

    fn make_request_frame(request_id: &str, method: &str) -> TunnelFrame {
        TunnelFrame {
            request_id: request_id.into(),
            frame_type: FrameType::Request as i32,
            timestamp: None,
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                StreamPayload {
                    method: method.into(),
                    data: vec![1, 2, 3],
                    sequence: 0,
                    metadata: HashMap::new(),
                },
            )),
        }
    }

    #[test]
    fn handle_request_produces_response() {
        let handler = make_handler();
        let frame = make_request_frame("req-1", "betcode.v1.AgentService/Converse");

        let response = handler.handle_frame(frame);
        assert!(response.is_some());

        let resp = response.unwrap();
        assert_eq!(resp.request_id, "req-1");
        assert_eq!(resp.frame_type, FrameType::Response as i32);
    }

    #[test]
    fn handle_control_frame_returns_none() {
        let handler = make_handler();
        let frame = TunnelFrame {
            request_id: "ctrl-1".into(),
            frame_type: FrameType::Control as i32,
            timestamp: None,
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::Control(
                betcode_proto::v1::TunnelControl {
                    control_type: betcode_proto::v1::TunnelControlType::Ping as i32,
                    params: HashMap::new(),
                },
            )),
        };

        assert!(handler.handle_frame(frame).is_none());
    }

    #[test]
    fn handle_error_frame_returns_none() {
        let handler = make_handler();
        let frame = TunnelFrame {
            request_id: "err-1".into(),
            frame_type: FrameType::Error as i32,
            timestamp: None,
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::Error(
                TunnelError {
                    code: TunnelErrorCode::Internal as i32,
                    message: "test error".into(),
                    details: HashMap::new(),
                },
            )),
        };

        assert!(handler.handle_frame(frame).is_none());
    }

    #[test]
    fn handle_request_without_payload_returns_error() {
        let handler = make_handler();
        let frame = TunnelFrame {
            request_id: "req-2".into(),
            frame_type: FrameType::Request as i32,
            timestamp: None,
            payload: None,
        };

        let response = handler.handle_frame(frame).unwrap();
        assert_eq!(response.frame_type, FrameType::Error as i32);
    }

    #[test]
    fn handle_unexpected_frame_type_returns_error() {
        let handler = make_handler();
        let frame = TunnelFrame {
            request_id: "req-3".into(),
            frame_type: FrameType::Response as i32, // Daemon shouldn't receive Response frames
            timestamp: None,
            payload: None,
        };

        let response = handler.handle_frame(frame).unwrap();
        assert_eq!(response.frame_type, FrameType::Error as i32);
    }
}
