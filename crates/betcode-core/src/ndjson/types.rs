//! Message types for Claude Code NDJSON protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Canonical message types from Claude Code.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    SystemInit(SystemInit),
    Assistant(AssistantMessage),
    User(UserMessage),
    StreamEvent(StreamEvent),
    ControlRequest(ControlRequest),
    Result(SessionResult),
    Unknown { msg_type: String, payload: Value },
}

/// System initialization message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInit {
    pub session_id: String,
    pub model: String,
    pub cwd: PathBuf,
    pub tools: Vec<ToolSchema>,
    #[serde(default)]
    pub api_version: Option<String>,
}

/// Tool schema from system init.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Value>,
}

/// Complete assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

/// Content block in assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

/// Reason the assistant stopped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StopReason {
    #[default]
    EndTurn,
    ToolUse,
    MaxTokens,
    Unknown(String),
}

/// Token usage statistics.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
}

/// User message (tool results echo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessage {
    pub content: Vec<ToolResult>,
}

/// Tool execution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

/// Streaming event for real-time output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEvent {
    pub event_type: StreamEventType,
}

/// Stream event types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEventType {
    ContentBlockStart { index: u32, block_type: String },
    ContentBlockDelta { index: u32, delta: Delta },
    ContentBlockStop { index: u32 },
    MessageStart,
    MessageDelta { stop_reason: Option<String> },
    MessageStop,
    Unknown(Value),
}

/// Delta content in streaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    Text(String),
    InputJson(String),
    Unknown(Value),
}

/// Permission or input request from Claude.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlRequest {
    pub request_id: String,
    pub request: ControlRequestType,
}

/// Control request type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequestType {
    CanUseTool { tool_name: String, input: Value },
    Unknown(Value),
}

/// Session completion result.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionResult {
    pub subtype: ResultSubtype,
    pub session_id: String,
    pub duration_ms: u64,
    pub cost_usd: Option<f64>,
    pub usage: Usage,
}

/// Result subtype.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ResultSubtype {
    #[default]
    Success,
    Error,
    Unknown(String),
}
