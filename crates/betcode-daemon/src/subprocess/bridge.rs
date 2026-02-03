//! NDJSON to gRPC event bridge.
//!
//! Converts NDJSON messages from Claude's stdout into gRPC AgentEvent messages.

use betcode_core::ndjson::{
    AssistantMessage, ContentBlock, ControlRequest as NdjsonControlRequest,
    ControlRequestType as NdjsonControlRequestType, Delta, Message, SessionResult, StopReason,
    StreamEvent, StreamEventType, SystemInit,
};
use betcode_proto::v1::{
    self as proto, AgentEvent, AgentStatus, PermissionRequest, SessionInfo, StatusChange,
    TextDelta, ToolCallStart, TurnComplete, UsageReport,
};
use prost_types::Timestamp;
use std::collections::HashMap;
use std::time::SystemTime;
use tracing::{debug, warn};

/// Bridge for converting NDJSON messages to gRPC events.
pub struct EventBridge {
    /// Current sequence number for events.
    sequence: u64,
    /// Pending tool calls (id -> name) for matching results.
    pending_tools: HashMap<String, String>,
    /// Current session info.
    session_info: Option<SessionInfo>,
}

impl Default for EventBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBridge {
    /// Create a new event bridge.
    pub fn new() -> Self {
        Self {
            sequence: 0,
            pending_tools: HashMap::new(),
            session_info: None,
        }
    }

    /// Convert an NDJSON message to gRPC events.
    ///
    /// Returns a vector because some messages produce multiple events.
    pub fn convert(&mut self, msg: Message) -> Vec<AgentEvent> {
        match msg {
            Message::SystemInit(init) => self.handle_system_init(init),
            Message::Assistant(assistant) => self.handle_assistant(assistant),
            Message::StreamEvent(stream) => self.handle_stream_event(stream),
            Message::ControlRequest(req) => self.handle_control_request(req),
            Message::Result(result) => self.handle_result(result),
            Message::User(_) => vec![], // User messages are echoes, no event needed
            Message::Unknown { msg_type, .. } => {
                warn!(msg_type, "Unknown NDJSON message type");
                vec![]
            }
        }
    }

    fn next_event(&mut self) -> AgentEvent {
        self.sequence += 1;
        AgentEvent {
            sequence: self.sequence,
            timestamp: Some(now_timestamp()),
            parent_tool_use_id: String::new(),
            event: None,
        }
    }

    fn handle_system_init(&mut self, init: SystemInit) -> Vec<AgentEvent> {
        let info = SessionInfo {
            session_id: init.session_id.clone(),
            model: init.model.clone(),
            working_directory: init.cwd.to_string_lossy().to_string(),
            worktree_id: String::new(),
            message_count: 0,
            is_resumed: false,
            is_compacted: false,
            context_usage_percent: 0.0,
        };

        self.session_info = Some(info.clone());

        let mut event = self.next_event();
        event.event = Some(proto::agent_event::Event::SessionInfo(info));

        vec![event]
    }

    fn handle_assistant(&mut self, msg: AssistantMessage) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        // Emit tool call events for each tool_use block
        for block in &msg.content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                self.pending_tools.insert(id.clone(), name.clone());

                let mut event = self.next_event();
                event.event = Some(proto::agent_event::Event::ToolCallStart(ToolCallStart {
                    tool_id: id.clone(),
                    tool_name: name.clone(),
                    input: Some(json_to_struct(input)),
                    description: String::new(),
                }));
                events.push(event);
            }
        }

        // Emit turn complete if we're done
        if matches!(msg.stop_reason, StopReason::EndTurn) {
            let mut event = self.next_event();
            event.event = Some(proto::agent_event::Event::TurnComplete(TurnComplete {
                stop_reason: "end_turn".to_string(),
            }));
            events.push(event);
        }

        events
    }

    fn handle_stream_event(&mut self, stream: StreamEvent) -> Vec<AgentEvent> {
        match stream.event_type {
            StreamEventType::ContentBlockDelta { delta, .. } => match delta {
                Delta::Text(text) => {
                    let mut event = self.next_event();
                    event.event = Some(proto::agent_event::Event::TextDelta(TextDelta {
                        text,
                        is_complete: false,
                    }));
                    vec![event]
                }
                Delta::InputJson(_) => vec![], // Buffered internally
                Delta::Unknown(_) => vec![],
            },
            StreamEventType::ContentBlockStop { .. } => {
                let mut event = self.next_event();
                event.event = Some(proto::agent_event::Event::TextDelta(TextDelta {
                    text: String::new(),
                    is_complete: true,
                }));
                vec![event]
            }
            StreamEventType::MessageStart => {
                let mut event = self.next_event();
                event.event = Some(proto::agent_event::Event::StatusChange(StatusChange {
                    status: AgentStatus::Thinking.into(),
                    message: String::new(),
                }));
                vec![event]
            }
            StreamEventType::MessageStop => {
                let mut event = self.next_event();
                event.event = Some(proto::agent_event::Event::StatusChange(StatusChange {
                    status: AgentStatus::Idle.into(),
                    message: String::new(),
                }));
                vec![event]
            }
            _ => vec![],
        }
    }

    fn handle_control_request(&mut self, req: NdjsonControlRequest) -> Vec<AgentEvent> {
        match req.request {
            NdjsonControlRequestType::CanUseTool { tool_name, input } => {
                let mut event = self.next_event();
                event.event = Some(proto::agent_event::Event::PermissionRequest(
                    PermissionRequest {
                        request_id: req.request_id,
                        tool_name,
                        description: String::new(),
                        input: Some(json_to_struct(&input)),
                    },
                ));
                vec![event]
            }
            NdjsonControlRequestType::Unknown(_) => {
                debug!(request_id = %req.request_id, "Unknown control request type");
                vec![]
            }
        }
    }

    fn handle_result(&mut self, result: SessionResult) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        // Emit usage report
        let mut usage_event = self.next_event();
        usage_event.event = Some(proto::agent_event::Event::Usage(UsageReport {
            input_tokens: result.usage.input_tokens,
            output_tokens: result.usage.output_tokens,
            cache_read_tokens: result.usage.cache_read_input_tokens,
            cache_creation_tokens: result.usage.cache_creation_input_tokens,
            model: String::new(),
            cost_usd: result.cost_usd.unwrap_or(0.0),
            duration_ms: result.duration_ms as u32,
        }));
        events.push(usage_event);

        // Emit status change to idle
        let mut status_event = self.next_event();
        status_event.event = Some(proto::agent_event::Event::StatusChange(StatusChange {
            status: AgentStatus::Idle.into(),
            message: String::new(),
        }));
        events.push(status_event);

        events
    }

    /// Get the current session info.
    pub fn session_info(&self) -> Option<&SessionInfo> {
        self.session_info.as_ref()
    }

    /// Get the current sequence number.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

fn now_timestamp() -> Timestamp {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}

fn json_to_struct(value: &serde_json::Value) -> prost_types::Struct {
    // Simple conversion - a full implementation would recursively convert
    let mut fields = std::collections::BTreeMap::new();

    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            fields.insert(k.clone(), json_value_to_prost(v));
        }
    }

    prost_types::Struct { fields }
}

fn json_value_to_prost(value: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;

    let kind = match value {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => Kind::ListValue(prost_types::ListValue {
            values: arr.iter().map(json_value_to_prost).collect(),
        }),
        serde_json::Value::Object(map) => {
            let mut fields = std::collections::BTreeMap::new();
            for (k, v) in map {
                fields.insert(k.clone(), json_value_to_prost(v));
            }
            Kind::StructValue(prost_types::Struct { fields })
        }
    };

    prost_types::Value { kind: Some(kind) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_starts_at_sequence_zero() {
        let bridge = EventBridge::new();
        assert_eq!(bridge.sequence(), 0);
    }

    #[test]
    fn system_init_produces_session_info() {
        let mut bridge = EventBridge::new();
        let init = SystemInit {
            session_id: "test-123".to_string(),
            model: "claude-sonnet-4".to_string(),
            cwd: "/tmp".into(),
            tools: vec![],
            api_version: None,
        };

        let events = bridge.convert(Message::SystemInit(init));
        assert_eq!(events.len(), 1);
        assert_eq!(bridge.sequence(), 1);

        let info = bridge.session_info().unwrap();
        assert_eq!(info.session_id, "test-123");
    }

    #[test]
    fn text_delta_produces_text_event() {
        let mut bridge = EventBridge::new();
        let stream = StreamEvent {
            event_type: StreamEventType::ContentBlockDelta {
                index: 0,
                delta: Delta::Text("Hello".to_string()),
            },
        };

        let events = bridge.convert(Message::StreamEvent(stream));
        assert_eq!(events.len(), 1);
    }
}
