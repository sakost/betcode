//! NDJSON parser for Claude Code protocol.
//!
//! Implements tolerant reader pattern: unknown fields ignored, unknown types logged.

use serde_json::Value;

use super::types::*;
use crate::error::{Error, Result};

/// Parse a single NDJSON line from Claude's stdout.
pub fn parse_line(line: &str) -> Result<Message> {
    let raw: Value = serde_json::from_str(line)?;
    parse_value(&raw)
}

/// Parse a JSON value into a canonical message.
pub fn parse_value(raw: &Value) -> Result<Message> {
    let msg_type = raw
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::NdjsonParse("Missing 'type' field".into()))?;

    match msg_type {
        "system" => parse_system(raw),
        "assistant" => parse_assistant(raw),
        "user" => parse_user(raw),
        "stream_event" => parse_stream_event(raw),
        "control_request" => parse_control_request(raw),
        "result" => parse_result(raw),
        _ => Ok(Message::Unknown {
            msg_type: msg_type.to_string(),
            payload: raw.clone(),
        }),
    }
}

fn parse_system(raw: &Value) -> Result<Message> {
    let session_id = raw
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let model = raw
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let cwd = raw
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_default();

    let tools: Vec<ToolSchema> = raw
        .get("tools")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let api_version = raw
        .get("api_version")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(Message::SystemInit(SystemInit {
        session_id,
        model,
        cwd,
        tools,
        api_version,
    }))
}

fn parse_assistant(raw: &Value) -> Result<Message> {
    let msg = raw.get("message").unwrap_or(raw);

    let content = parse_content_blocks(msg.get("content"));
    let stop_reason = parse_stop_reason(msg.get("stop_reason"));
    let usage = parse_usage(msg.get("usage"));

    Ok(Message::Assistant(AssistantMessage {
        content,
        stop_reason,
        usage,
    }))
}

fn parse_content_blocks(content: Option<&Value>) -> Vec<ContentBlock> {
    let Some(arr) = content.and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    arr.iter()
        .filter_map(|block| {
            let block_type = block.get("type")?.as_str()?;
            match block_type {
                "text" => {
                    let text = block.get("text")?.as_str()?.to_string();
                    Some(ContentBlock::Text { text })
                }
                "tool_use" => {
                    let id = block.get("id")?.as_str()?.to_string();
                    let name = block.get("name")?.as_str()?.to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    Some(ContentBlock::ToolUse { id, name, input })
                }
                _ => None,
            }
        })
        .collect()
}

fn parse_stop_reason(val: Option<&Value>) -> StopReason {
    match val.and_then(|v| v.as_str()) {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some(other) => StopReason::Unknown(other.to_string()),
        None => StopReason::EndTurn,
    }
}

pub(crate) fn parse_usage(val: Option<&Value>) -> Usage {
    val.and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn parse_user(raw: &Value) -> Result<Message> {
    let msg = raw.get("message").unwrap_or(raw);
    let content = msg
        .get("content")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|block| {
                    if block.get("type")?.as_str()? != "tool_result" {
                        return None;
                    }
                    Some(ToolResult {
                        tool_use_id: block.get("tool_use_id")?.as_str()?.to_string(),
                        content: block
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        is_error: block
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Message::User(UserMessage { content }))
}

fn parse_stream_event(raw: &Value) -> Result<Message> {
    let event = raw.get("event").unwrap_or(raw);
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

    let stream_type = match event_type {
        "content_block_start" => StreamEventType::ContentBlockStart {
            index: event.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            block_type: event
                .get("content_block")
                .and_then(|b| b.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "content_block_delta" => {
            let index = event.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let delta = event.get("delta").cloned().unwrap_or(Value::Null);
            let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let delta = match delta_type {
                "text_delta" => Delta::Text(
                    delta
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                "input_json_delta" => Delta::InputJson(
                    delta
                        .get("partial_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                _ => Delta::Unknown(delta),
            };
            StreamEventType::ContentBlockDelta { index, delta }
        }
        "content_block_stop" => StreamEventType::ContentBlockStop {
            index: event.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        },
        "message_start" => StreamEventType::MessageStart,
        "message_delta" => StreamEventType::MessageDelta {
            stop_reason: event
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        "message_stop" => StreamEventType::MessageStop,
        _ => StreamEventType::Unknown(event.clone()),
    };

    Ok(Message::StreamEvent(StreamEvent {
        event_type: stream_type,
    }))
}

fn parse_control_request(raw: &Value) -> Result<Message> {
    let request_id = raw
        .get("request_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::NdjsonParse("Missing request_id".into()))?
        .to_string();

    let request = raw.get("request").cloned().unwrap_or(Value::Null);
    let subtype = request
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let request_type = match subtype {
        "can_use_tool" => ControlRequestType::CanUseTool {
            tool_name: request
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            input: request.get("input").cloned().unwrap_or(Value::Null),
        },
        _ => ControlRequestType::Unknown(request),
    };

    Ok(Message::ControlRequest(ControlRequest {
        request_id,
        request: request_type,
    }))
}

fn parse_result(raw: &Value) -> Result<Message> {
    let session_id = raw
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let subtype = match raw.get("subtype").and_then(|v| v.as_str()) {
        Some("success") => ResultSubtype::Success,
        Some("error") => ResultSubtype::Error,
        Some(other) => ResultSubtype::Unknown(other.to_string()),
        None => ResultSubtype::Success,
    };

    let duration_ms = raw.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let cost_usd = raw.get("total_cost_usd").and_then(|v| v.as_f64());
    let usage = parse_usage(raw.get("usage"));

    Ok(Message::Result(SessionResult {
        subtype,
        session_id,
        duration_ms,
        cost_usd,
        usage,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_system_init() {
        let json = r#"{"type":"system","subtype":"init","session_id":"abc123","model":"claude-sonnet-4-20250514","cwd":"/home/user","tools":[]}"#;
        let msg = parse_line(json).unwrap();
        assert!(matches!(msg, Message::SystemInit(_)));
    }

    #[test]
    fn parse_control_request_works() {
        let json = r#"{"type":"control_request","request_id":"req_001","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}"#;
        let msg = parse_line(json).unwrap();
        assert!(matches!(msg, Message::ControlRequest(_)));
    }

    #[test]
    fn tolerant_reader_ignores_unknown_fields() {
        let json = r#"{"type":"system","session_id":"x","model":"m","cwd":"/","tools":[],"unknown":"ignored"}"#;
        assert!(parse_line(json).is_ok());
    }

    #[test]
    fn unknown_type_returns_unknown_message() {
        let json = r#"{"type":"future_type","data":"something"}"#;
        let msg = parse_line(json).unwrap();
        assert!(matches!(msg, Message::Unknown { .. }));
    }
}
