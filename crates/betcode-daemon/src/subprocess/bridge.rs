//! NDJSON to gRPC event bridge.
//!
//! Converts NDJSON messages from Claude's stdout into gRPC `AgentEvent` messages.

use betcode_core::ndjson::{
    AssistantMessage, ContentBlock, ControlRequest as NdjsonControlRequest,
    ControlRequestType as NdjsonControlRequestType, Delta, Message, ResultSubtype, SessionResult,
    StopReason, StreamEvent, StreamEventType, SystemInit, UserMessage,
};
use betcode_proto::v1::{
    self as proto, AgentEvent, AgentStatus, PermissionRequest, QuestionOption, SessionInfo,
    StatusChange, TextDelta, ToolCallStart, TurnComplete, UsageReport, UserQuestion,
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
    /// Pending `AskUserQuestion` inputs keyed by `request_id`.
    /// Stored when converting `control_request` → `UserQuestion` so the relay
    /// can reconstruct the `updatedInput` in the response.
    pending_question_inputs: HashMap<String, serde_json::Value>,
    /// Pending permission (`CanUseTool`) inputs keyed by `request_id`.
    /// Stored when converting `control_request` → `PermissionRequest` so the relay
    /// can pass back the original `updatedInput` in the response.
    pending_permission_inputs: HashMap<String, serde_json::Value>,
    /// MCP tool entries extracted from the last `system_init` message.
    mcp_entries: Vec<betcode_core::commands::CommandEntry>,
}

impl Default for EventBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBridge {
    /// Create a new event bridge starting at sequence 0.
    pub fn new() -> Self {
        Self::with_start_sequence(0)
    }

    /// Create a new event bridge continuing from a given sequence number.
    ///
    /// Used when resuming a session so new events don't collide with
    /// already-stored events in the database.
    pub fn with_start_sequence(start_sequence: u64) -> Self {
        Self {
            sequence: start_sequence,
            pending_tools: HashMap::new(),
            session_info: None,
            pending_question_inputs: HashMap::new(),
            pending_permission_inputs: HashMap::new(),
            mcp_entries: Vec::new(),
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
            Message::User(user) => self.handle_user(user),
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

    #[allow(clippy::needless_pass_by_value)]
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
        self.mcp_entries = betcode_core::commands::mcp_tools_to_entries(&init.tools);

        let mut event = self.next_event();
        event.event = Some(proto::agent_event::Event::SessionInfo(info));

        vec![event]
    }

    #[allow(clippy::needless_pass_by_value)]
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
                    description: tool_description(name, input),
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
                Delta::Text(text) if !text.is_empty() => {
                    let mut event = self.next_event();
                    event.event = Some(proto::agent_event::Event::TextDelta(TextDelta {
                        text,
                        is_complete: false,
                    }));
                    vec![event]
                }
                Delta::Text(_) | Delta::InputJson(_) | Delta::Unknown(_) => vec![],
            },
            StreamEventType::ContentBlockStop { .. } => {
                // No event emitted — the assistant message already triggers
                // TurnComplete and emitting an empty TextDelta here causes
                // the TUI to render a blank "Claude:" line after the response.
                vec![]
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
            NdjsonControlRequestType::CanUseTool { tool_name, input }
                if tool_name == "AskUserQuestion" =>
            {
                self.handle_ask_user_question(req.request_id, input)
            }
            NdjsonControlRequestType::CanUseTool { tool_name, input } => {
                let desc = tool_description(&tool_name, &input);
                self.pending_permission_inputs
                    .insert(req.request_id.clone(), input.clone());
                let mut event = self.next_event();
                event.event = Some(proto::agent_event::Event::PermissionRequest(
                    PermissionRequest {
                        request_id: req.request_id,
                        tool_name,
                        description: desc,
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

    /// Take the original input for a pending `AskUserQuestion`.
    /// Returns `None` if the `request_id` is not found or was already taken.
    pub fn take_question_input(&mut self, request_id: &str) -> Option<serde_json::Value> {
        self.pending_question_inputs.remove(request_id)
    }

    /// Take the original input for a pending permission request (`CanUseTool`).
    /// Returns `None` if the `request_id` is not found or was already taken.
    pub fn take_permission_input(&mut self, request_id: &str) -> Option<serde_json::Value> {
        self.pending_permission_inputs.remove(request_id)
    }

    #[allow(clippy::needless_pass_by_value)]
    fn handle_ask_user_question(
        &mut self,
        request_id: String,
        input: serde_json::Value,
    ) -> Vec<AgentEvent> {
        self.pending_question_inputs
            .insert(request_id.clone(), input.clone());
        // Extract the first question from the questions array.
        // AskUserQuestion input: {"questions": [{"question": "...", "options": [...], "multi_select": bool}]}
        let questions = input
            .get("questions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let first = questions
            .first()
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let question_text = first
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let multi_select = first
            .get("multi_select")
            .or_else(|| first.get("multiSelect"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let options = first
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|opt| QuestionOption {
                        value: opt
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        label: opt
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        description: opt
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut event = self.next_event();
        event.event = Some(proto::agent_event::Event::UserQuestion(UserQuestion {
            question_id: request_id,
            question: question_text,
            options,
            multi_select,
        }));

        vec![event]
    }

    fn handle_user(&mut self, user: UserMessage) -> Vec<AgentEvent> {
        user.content
            .into_iter()
            .map(|tr| {
                let mut event = self.next_event();
                event.event = Some(proto::agent_event::Event::ToolCallResult(
                    proto::ToolCallResult {
                        tool_id: tr.tool_use_id,
                        output: tr.content,
                        is_error: tr.is_error,
                        duration_ms: 0,
                    },
                ));
                event
            })
            .collect()
    }

    #[allow(clippy::needless_pass_by_value)]
    fn handle_result(&mut self, result: SessionResult) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        // If Claude reported an error, emit an ErrorEvent before the status change.
        // Use `subtype` as the authoritative signal: when subtype is Success and
        // there are no error messages, `is_error: true` is spurious (Claude Code
        // can send this contradictory combination) and should not kill the session.
        let has_real_error = result.is_error
            && (result.subtype != ResultSubtype::Success || !result.errors.is_empty());
        if has_real_error {
            let error_message = if result.errors.is_empty() {
                format!("Claude exited with error (subtype: {:?})", result.subtype)
            } else {
                result.errors.join("; ")
            };
            warn!(errors = ?result.errors, "Claude result indicates error");
            let mut error_event = self.next_event();
            error_event.event = Some(proto::agent_event::Event::Error(proto::ErrorEvent {
                code: "session_error".to_string(),
                message: error_message,
                is_fatal: true,
                details: HashMap::default(),
            }));
            events.push(error_event);
        } else if result.is_error {
            // is_error is set but subtype is Success with no error messages —
            // log for observability but don't emit a fatal error to the client.
            warn!(
                subtype = ?result.subtype,
                "Claude result has is_error=true but subtype=Success with no errors, ignoring"
            );
        }

        // Emit usage report
        let mut usage_event = self.next_event();
        usage_event.event = Some(proto::agent_event::Event::Usage(UsageReport {
            input_tokens: result.usage.input_tokens,
            output_tokens: result.usage.output_tokens,
            cache_read_tokens: result.usage.cache_read_input_tokens,
            cache_creation_tokens: result.usage.cache_creation_input_tokens,
            model: String::new(),
            cost_usd: result.cost_usd.unwrap_or(0.0),
            #[allow(clippy::cast_possible_truncation)]
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
    pub const fn session_info(&self) -> Option<&SessionInfo> {
        self.session_info.as_ref()
    }

    /// Returns MCP tool entries discovered from the most recent `system_init`.
    pub fn mcp_entries(&self) -> &[betcode_core::commands::CommandEntry] {
        &self.mcp_entries
    }

    /// Get the current sequence number.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

fn now_timestamp() -> Timestamp {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        #[allow(clippy::cast_possible_wrap)]
        seconds: now.as_secs() as i64,
        #[allow(clippy::cast_possible_wrap)]
        nanos: now.subsec_nanos() as i32,
    }
}

/// Generate a human-readable description from tool name and input.
fn tool_description(name: &str, input: &serde_json::Value) -> String {
    let Some(obj) = input.as_object() else {
        return String::new();
    };

    match name {
        "Bash" => obj
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| truncate_str(c, 120))
            .unwrap_or_default(),
        "Read" | "Write" => obj
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Edit" => obj
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Grep" => {
            let pattern = obj.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            format!("{} in {}", truncate_str(pattern, 60), path)
        }
        "Glob" => obj
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "WebFetch" | "WebSearch" => obj
            .get("url")
            .or_else(|| obj.get("query"))
            .and_then(|v| v.as_str())
            .map(|s| truncate_str(s, 120))
            .unwrap_or_default(),
        "ToolSearch" => obj
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => {
            // For unknown tools, show first string value as summary
            obj.values()
                .find_map(|v| v.as_str())
                .map(|s| truncate_str(s, 80))
                .unwrap_or_default()
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .take_while(|(i, _)| *i <= max)
            .last()
            .map_or(0, |(i, _)| i);
        format!("{}...", &s[..boundary])
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
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::uninlined_format_args,
    clippy::default_trait_access,
    clippy::needless_pass_by_value
)]
mod tests {
    use super::*;

    #[test]
    fn bridge_starts_at_sequence_zero() {
        let bridge = EventBridge::new();
        assert_eq!(bridge.sequence(), 0);
    }

    #[test]
    fn bridge_with_start_sequence_continues_from_offset() {
        let mut bridge = EventBridge::with_start_sequence(42);
        assert_eq!(bridge.sequence(), 42);

        // First event should be sequence 43
        let events = bridge.convert(Message::StreamEvent(StreamEvent {
            event_type: StreamEventType::MessageStart,
        }));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, 43);
        assert_eq!(bridge.sequence(), 43);
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
        match &events[0].event {
            Some(proto::agent_event::Event::TextDelta(td)) => {
                assert_eq!(td.text, "Hello");
                assert!(!td.is_complete);
            }
            other => panic!("Expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn empty_text_delta_is_suppressed() {
        let mut bridge = EventBridge::new();
        let stream = StreamEvent {
            event_type: StreamEventType::ContentBlockDelta {
                index: 0,
                delta: Delta::Text(String::new()),
            },
        };
        let events = bridge.convert(Message::StreamEvent(stream));
        assert!(
            events.is_empty(),
            "Empty text deltas should produce no events"
        );
    }

    #[test]
    fn content_block_stop_produces_no_event() {
        let mut bridge = EventBridge::new();
        let stream = StreamEvent {
            event_type: StreamEventType::ContentBlockStop { index: 0 },
        };
        let events = bridge.convert(Message::StreamEvent(stream));
        assert!(
            events.is_empty(),
            "ContentBlockStop should produce no events"
        );
    }

    #[test]
    fn assistant_with_tool_use_produces_tool_call_start() {
        let mut bridge = EventBridge::new();
        let msg = AssistantMessage {
            content: vec![
                ContentBlock::Text {
                    text: "Let me run that.".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({"command": "ls -la"}),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage: Default::default(),
        };
        let events = bridge.convert(Message::Assistant(msg));
        assert_eq!(events.len(), 1);
        match &events[0].event {
            Some(proto::agent_event::Event::ToolCallStart(tc)) => {
                assert_eq!(tc.tool_name, "Bash");
                assert_eq!(tc.tool_id, "tool_1");
                assert_eq!(tc.description, "ls -la");
            }
            other => panic!("Expected ToolCallStart, got {:?}", other),
        }
    }

    #[test]
    fn assistant_end_turn_produces_turn_complete() {
        let mut bridge = EventBridge::new();
        let msg = AssistantMessage {
            content: vec![ContentBlock::Text {
                text: "Done!".to_string(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        };
        let events = bridge.convert(Message::Assistant(msg));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].event,
            Some(proto::agent_event::Event::TurnComplete(_))
        ));
    }

    #[test]
    fn assistant_tool_use_no_turn_complete() {
        let mut bridge = EventBridge::new();
        let msg = AssistantMessage {
            content: vec![ContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/tmp/foo.rs"}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Default::default(),
        };
        let events = bridge.convert(Message::Assistant(msg));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].event,
            Some(proto::agent_event::Event::ToolCallStart(_))
        ));
    }

    #[test]
    fn user_message_produces_tool_call_results() {
        use betcode_core::ndjson::ToolResult;
        let mut bridge = EventBridge::new();
        let msg = UserMessage {
            content: vec![
                ToolResult {
                    tool_use_id: "tool_1".to_string(),
                    content: "file1.rs\nfile2.rs".to_string(),
                    is_error: false,
                },
                ToolResult {
                    tool_use_id: "tool_2".to_string(),
                    content: "permission denied".to_string(),
                    is_error: true,
                },
            ],
        };
        let events = bridge.convert(Message::User(msg));
        assert_eq!(events.len(), 2);
        match &events[0].event {
            Some(proto::agent_event::Event::ToolCallResult(r)) => {
                assert_eq!(r.tool_id, "tool_1");
                assert!(!r.is_error);
            }
            other => panic!("Expected ToolCallResult, got {:?}", other),
        }
        match &events[1].event {
            Some(proto::agent_event::Event::ToolCallResult(r)) => {
                assert_eq!(r.tool_id, "tool_2");
                assert!(r.is_error);
            }
            other => panic!("Expected ToolCallResult, got {:?}", other),
        }
    }

    #[test]
    fn tool_description_bash() {
        let desc = tool_description("Bash", &serde_json::json!({"command": "cargo test"}));
        assert_eq!(desc, "cargo test");
    }

    #[test]
    fn tool_description_read() {
        let desc = tool_description("Read", &serde_json::json!({"file_path": "/tmp/foo.rs"}));
        assert_eq!(desc, "/tmp/foo.rs");
    }

    #[test]
    fn tool_description_grep() {
        let desc = tool_description(
            "Grep",
            &serde_json::json!({"pattern": "fn main", "path": "src/"}),
        );
        assert_eq!(desc, "fn main in src/");
    }

    #[test]
    fn tool_description_unknown_tool_uses_first_string() {
        let desc = tool_description(
            "SomeNewTool",
            &serde_json::json!({"query": "search term", "count": 5}),
        );
        assert_eq!(desc, "search term");
    }

    #[test]
    fn tool_description_null_input() {
        let desc = tool_description("Bash", &serde_json::Value::Null);
        assert!(desc.is_empty());
    }

    #[test]
    fn tool_description_truncates_long_command() {
        let long_cmd = "x".repeat(200);
        let desc = tool_description("Bash", &serde_json::json!({"command": long_cmd}));
        assert!(desc.len() <= 124);
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn permission_request_includes_description() {
        let mut bridge = EventBridge::new();
        let req = NdjsonControlRequest {
            request_id: "req_1".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "Bash".to_string(),
                input: serde_json::json!({"command": "rm -rf /tmp/test"}),
            },
        };
        let events = bridge.convert(Message::ControlRequest(req));
        assert_eq!(events.len(), 1);
        match &events[0].event {
            Some(proto::agent_event::Event::PermissionRequest(p)) => {
                assert_eq!(p.tool_name, "Bash");
                assert_eq!(p.description, "rm -rf /tmp/test");
                assert_eq!(p.request_id, "req_1");
            }
            other => panic!("Expected PermissionRequest, got {:?}", other),
        }
    }

    #[test]
    fn input_json_delta_produces_no_event() {
        let mut bridge = EventBridge::new();
        let stream = StreamEvent {
            event_type: StreamEventType::ContentBlockDelta {
                index: 1,
                delta: Delta::InputJson("{\"command\":".to_string()),
            },
        };
        let events = bridge.convert(Message::StreamEvent(stream));
        assert!(events.is_empty());
    }

    #[test]
    fn ask_user_question_produces_user_question_event() {
        let mut bridge = EventBridge::new();
        let req = NdjsonControlRequest {
            request_id: "req_q1".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "AskUserQuestion".to_string(),
                input: serde_json::json!({
                    "questions": [{
                        "question": "Which database?",
                        "options": [
                            {"label": "PostgreSQL", "description": "Full-featured RDBMS"},
                            {"label": "SQLite", "description": "Embedded database"}
                        ],
                        "multi_select": false
                    }]
                }),
            },
        };
        let events = bridge.convert(Message::ControlRequest(req));
        assert_eq!(
            events.len(),
            1,
            "AskUserQuestion should produce exactly 1 event"
        );
        match &events[0].event {
            Some(proto::agent_event::Event::UserQuestion(q)) => {
                assert_eq!(q.question_id, "req_q1");
                assert_eq!(q.question, "Which database?");
                assert_eq!(q.options.len(), 2);
                assert_eq!(q.options[0].label, "PostgreSQL");
                assert_eq!(q.options[0].description, "Full-featured RDBMS");
                assert_eq!(q.options[1].label, "SQLite");
                assert!(!q.multi_select);
            }
            other => panic!("Expected UserQuestion, got {:?}", other),
        }
    }

    #[test]
    fn ask_user_question_multi_select() {
        let mut bridge = EventBridge::new();
        let req = NdjsonControlRequest {
            request_id: "req_q2".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "AskUserQuestion".to_string(),
                input: serde_json::json!({
                    "questions": [{
                        "question": "Which features?",
                        "options": [
                            {"label": "Auth", "description": ""},
                            {"label": "Cache", "description": ""}
                        ],
                        "multi_select": true
                    }]
                }),
            },
        };
        let events = bridge.convert(Message::ControlRequest(req));
        assert_eq!(events.len(), 1);
        match &events[0].event {
            Some(proto::agent_event::Event::UserQuestion(q)) => {
                assert!(q.multi_select);
                assert_eq!(q.question, "Which features?");
            }
            other => panic!("Expected UserQuestion, got {:?}", other),
        }
    }

    #[test]
    fn ask_user_question_not_treated_as_permission() {
        let mut bridge = EventBridge::new();
        let req = NdjsonControlRequest {
            request_id: "req_q3".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "AskUserQuestion".to_string(),
                input: serde_json::json!({
                    "questions": [{
                        "question": "Pick one",
                        "options": [{"label": "A", "description": ""}],
                        "multi_select": false
                    }]
                }),
            },
        };
        let events = bridge.convert(Message::ControlRequest(req));
        assert_eq!(events.len(), 1);
        // Must NOT be a PermissionRequest
        assert!(
            !matches!(
                &events[0].event,
                Some(proto::agent_event::Event::PermissionRequest(_))
            ),
            "AskUserQuestion must not be converted to PermissionRequest"
        );
    }

    #[test]
    fn regular_tool_permission_still_works() {
        let mut bridge = EventBridge::new();
        let req = NdjsonControlRequest {
            request_id: "req_p1".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "Bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
            },
        };
        let events = bridge.convert(Message::ControlRequest(req));
        assert_eq!(events.len(), 1);
        assert!(
            matches!(
                &events[0].event,
                Some(proto::agent_event::Event::PermissionRequest(_))
            ),
            "Regular tool should still produce PermissionRequest"
        );
    }

    #[test]
    fn permission_request_stores_original_input() {
        let mut bridge = EventBridge::new();
        let input = serde_json::json!({"command": "rm -rf /tmp/test"});
        let req = NdjsonControlRequest {
            request_id: "req_perm_1".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "Bash".to_string(),
                input: input.clone(),
            },
        };
        bridge.convert(Message::ControlRequest(req));

        // The bridge should store the original input for later retrieval
        let taken = bridge.take_permission_input("req_perm_1");
        assert_eq!(taken, Some(input));
    }

    #[test]
    fn take_permission_input_returns_none_after_taken() {
        let mut bridge = EventBridge::new();
        let req = NdjsonControlRequest {
            request_id: "req_perm_2".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "Bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
            },
        };
        bridge.convert(Message::ControlRequest(req));

        // First take succeeds
        assert!(bridge.take_permission_input("req_perm_2").is_some());
        // Second take returns None (already consumed)
        assert!(bridge.take_permission_input("req_perm_2").is_none());
    }

    #[test]
    fn take_permission_input_returns_none_for_unknown_id() {
        let mut bridge = EventBridge::new();
        assert!(bridge.take_permission_input("nonexistent").is_none());
    }

    #[test]
    fn ask_user_question_does_not_store_permission_input() {
        let mut bridge = EventBridge::new();
        let req = NdjsonControlRequest {
            request_id: "req_q_only".to_string(),
            request: NdjsonControlRequestType::CanUseTool {
                tool_name: "AskUserQuestion".to_string(),
                input: serde_json::json!({"questions": [{"question": "Pick", "options": [{"label": "A", "description": ""}], "multi_select": false}]}),
            },
        };
        bridge.convert(Message::ControlRequest(req));

        // AskUserQuestion stores in question_inputs, NOT permission_inputs
        assert!(bridge.take_permission_input("req_q_only").is_none());
    }

    #[test]
    fn sequence_increments_only_for_emitted_events() {
        let mut bridge = EventBridge::new();

        bridge.convert(Message::StreamEvent(StreamEvent {
            event_type: StreamEventType::MessageStart,
        }));
        assert_eq!(bridge.sequence(), 1);

        bridge.convert(Message::StreamEvent(StreamEvent {
            event_type: StreamEventType::ContentBlockDelta {
                index: 0,
                delta: Delta::Text("hi".to_string()),
            },
        }));
        assert_eq!(bridge.sequence(), 2);

        // Events that produce nothing shouldn't increment
        bridge.convert(Message::StreamEvent(StreamEvent {
            event_type: StreamEventType::ContentBlockStop { index: 0 },
        }));
        assert_eq!(bridge.sequence(), 2);
    }

    #[test]
    fn handle_result_error_emits_error_event() {
        use betcode_core::ndjson::{ResultSubtype, Usage};

        let mut bridge = EventBridge::new();
        let result = SessionResult {
            subtype: ResultSubtype::Unknown("error_during_execution".to_string()),
            session_id: "sess-err".to_string(),
            duration_ms: 0,
            cost_usd: Some(0.0),
            usage: Usage::default(),
            is_error: true,
            errors: vec!["No conversation found with session ID: abc-123".to_string()],
        };

        let events = bridge.convert(Message::Result(result));
        // Should produce 3 events: ErrorEvent, UsageReport, StatusChange(Idle)
        assert_eq!(events.len(), 3, "expected error + usage + status events");

        // First event: ErrorEvent
        match &events[0].event {
            Some(proto::agent_event::Event::Error(e)) => {
                assert_eq!(e.code, "session_error");
                assert!(
                    e.message.contains("No conversation found"),
                    "error message should contain the original error"
                );
                assert!(e.is_fatal);
            }
            other => panic!("Expected ErrorEvent, got {:?}", other),
        }

        // Second event: UsageReport
        assert!(matches!(
            &events[1].event,
            Some(proto::agent_event::Event::Usage(_))
        ));

        // Third event: StatusChange(Idle)
        match &events[2].event {
            Some(proto::agent_event::Event::StatusChange(sc)) => {
                assert_eq!(sc.status, i32::from(AgentStatus::Idle));
            }
            other => panic!("Expected StatusChange, got {:?}", other),
        }
    }

    #[test]
    fn handle_result_success_no_error_event() {
        use betcode_core::ndjson::{ResultSubtype, Usage};

        let mut bridge = EventBridge::new();
        let result = SessionResult {
            subtype: ResultSubtype::Success,
            session_id: "sess-ok".to_string(),
            duration_ms: 1000,
            cost_usd: Some(0.01),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
            is_error: false,
            errors: vec![],
        };

        let events = bridge.convert(Message::Result(result));
        // Should produce only 2 events: UsageReport, StatusChange(Idle) — no ErrorEvent
        assert_eq!(events.len(), 2, "expected usage + status events only");

        assert!(matches!(
            &events[0].event,
            Some(proto::agent_event::Event::Usage(_))
        ));
        assert!(matches!(
            &events[1].event,
            Some(proto::agent_event::Event::StatusChange(_))
        ));
    }

    #[test]
    fn handle_result_is_error_true_but_subtype_success_no_errors_is_not_fatal() {
        use betcode_core::ndjson::{ResultSubtype, Usage};

        let mut bridge = EventBridge::new();
        let result = SessionResult {
            subtype: ResultSubtype::Success,
            session_id: "sess-spurious".to_string(),
            duration_ms: 500,
            cost_usd: Some(0.005),
            usage: Usage {
                input_tokens: 50,
                output_tokens: 25,
                ..Default::default()
            },
            is_error: true,
            errors: vec![],
        };

        let events = bridge.convert(Message::Result(result));
        // is_error=true but subtype=Success with no errors: should NOT emit ErrorEvent
        assert_eq!(
            events.len(),
            2,
            "expected usage + status events only (no error)"
        );

        assert!(matches!(
            &events[0].event,
            Some(proto::agent_event::Event::Usage(_))
        ));
        assert!(matches!(
            &events[1].event,
            Some(proto::agent_event::Event::StatusChange(_))
        ));
    }

    #[test]
    fn truncate_str_handles_multibyte_utf8() {
        // ASCII
        assert_eq!(truncate_str("hello world", 5), "hello...");
        assert_eq!(truncate_str("short", 10), "short");
        // Multi-byte: "café" is 5 bytes (c=1, a=1, f=1, é=2)
        assert_eq!(truncate_str("café latte", 4), "caf...");
        // Empty
        assert_eq!(truncate_str("", 5), "");
        // Exactly at boundary
        assert_eq!(truncate_str("abc", 3), "abc");
    }

    #[test]
    fn system_init_with_mcp_tools_populates_mcp_entries() {
        use betcode_core::commands::CommandCategory;
        use betcode_core::ndjson::ToolSchema;

        let mut bridge = EventBridge::new();
        let init = SystemInit {
            session_id: "mcp-test".to_string(),
            model: "claude-sonnet-4".to_string(),
            cwd: "/tmp".into(),
            tools: vec![
                ToolSchema {
                    name: "mcp__my-server__list_items".to_string(),
                    description: Some("List all items".to_string()),
                    input_schema: None,
                },
                ToolSchema {
                    name: "mcp__my-server__get_item".to_string(),
                    description: None,
                    input_schema: None,
                },
                // Non-MCP tool -- should be ignored
                ToolSchema {
                    name: "Bash".to_string(),
                    description: Some("Run bash commands".to_string()),
                    input_schema: None,
                },
            ],
            api_version: None,
        };

        let events = bridge.convert(Message::SystemInit(init));
        assert_eq!(events.len(), 1, "system_init should produce one event");

        let entries = bridge.mcp_entries();
        assert_eq!(
            entries.len(),
            2,
            "should have 2 MCP entries (Bash excluded)"
        );

        // First entry: my-server:list_items
        assert_eq!(entries[0].name, "my-server:list_items");
        assert_eq!(entries[0].category, CommandCategory::Mcp);
        assert_eq!(entries[0].group.as_deref(), Some("my-server"));
        assert_eq!(entries[0].description, "List all items");

        // Second entry: my-server:get_item (no description -> fallback)
        assert_eq!(entries[1].name, "my-server:get_item");
        assert_eq!(entries[1].category, CommandCategory::Mcp);
        assert_eq!(entries[1].group.as_deref(), Some("my-server"));
    }
}
