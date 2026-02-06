//! Application state and types.

use std::collections::VecDeque;

use betcode_proto::v1::AgentEvent;

/// Application mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    /// Permission dialog with action keys (Y/A/Tab/N/X).
    PermissionPrompt,
    /// Editing tool input before allowing (Tab).
    PermissionEditInput,
    /// Writing a follow-up comment to send after allowing (E).
    PermissionComment,
    /// Writing a deny message (N=continue, X=interrupt).
    PermissionDenyMessage,
    /// Claude asked a question with selectable options.
    UserQuestion,
    SessionList,
}

/// A displayable message in the conversation.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: MessageRole,
    pub content: String,
    pub streaming: bool,
}

/// Message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Pending permission request shown as dialog.
#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    /// Original tool input from the PermissionRequest (Struct serialized to JSON).
    pub original_input: Option<serde_json::Value>,
    /// Text buffer for editing (tool input JSON / comment / deny message).
    pub edit_buffer: String,
    /// Cursor position in edit_buffer.
    pub edit_cursor: usize,
    /// Whether deny should interrupt the current turn (N=false, X=true).
    pub deny_interrupt: bool,
}

/// Pending question from Claude (AskUserQuestion tool).
#[derive(Debug, Clone)]
pub struct PendingUserQuestion {
    pub question_id: String,
    pub question: String,
    pub options: Vec<QuestionOptionDisplay>,
    pub multi_select: bool,
    /// Currently highlighted option index (arrow navigation).
    pub highlight: usize,
    /// Selected option indices (for multi-select).
    pub selected: Vec<usize>,
}

/// Display-friendly question option.
#[derive(Debug, Clone)]
pub struct QuestionOptionDisplay {
    pub label: String,
    pub description: String,
}

/// Token usage info for status bar.
#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
}

/// TUI application state.
pub struct App {
    pub mode: AppMode,
    pub session_id: Option<String>,
    pub model: String,
    pub messages: Vec<DisplayMessage>,
    pub input: String,
    pub cursor_pos: usize,
    pub input_history: VecDeque<String>,
    pub history_index: Option<usize>,
    /// Manual scroll offset from the bottom (0 = pinned to bottom).
    pub scroll_offset: u16,
    /// Whether the user has manually scrolled up (disables auto-scroll).
    pub scroll_pinned: bool,
    /// Height of the message viewport (set each frame by the renderer).
    pub viewport_height: u16,
    /// Total line count of rendered messages (set each frame by the renderer).
    pub total_lines: u16,
    pub should_quit: bool,
    pub status: String,
    pub token_usage: Option<TokenUsage>,
    pub pending_permission: Option<PendingPermission>,
    pub pending_question: Option<PendingUserQuestion>,
    pub agent_busy: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: AppMode::Normal,
            session_id: None,
            model: String::new(),
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            input_history: VecDeque::with_capacity(100),
            history_index: None,
            scroll_offset: 0,
            scroll_pinned: true,
            viewport_height: 0,
            total_lines: 0,
            should_quit: false,
            status: "Connecting...".to_string(),
            token_usage: None,
            pending_permission: None,
            pending_question: None,
            agent_busy: false,
        }
    }

    /// Scroll up by `n` lines.
    pub fn scroll_up(&mut self, n: u16) {
        let max_scroll = self.total_lines.saturating_sub(self.viewport_height);
        self.scroll_offset = self.scroll_offset.saturating_add(n).min(max_scroll);
        if self.scroll_offset > 0 {
            self.scroll_pinned = false;
        }
    }

    /// Scroll down by `n` lines.
    pub fn scroll_down(&mut self, n: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        if self.scroll_offset == 0 {
            self.scroll_pinned = true;
        }
    }

    /// Snap scroll to the bottom (most recent messages).
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.scroll_pinned = true;
    }

    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(DisplayMessage {
            role: MessageRole::User,
            content,
            streaming: false,
        });
    }

    pub fn start_assistant_message(&mut self) {
        self.messages.push(DisplayMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            streaming: true,
        });
    }

    pub fn append_text(&mut self, text: &str) {
        if let Some(msg) = self.messages.last_mut() {
            if msg.streaming {
                msg.content.push_str(text);
            }
        }
    }

    pub fn finish_streaming(&mut self) {
        if let Some(msg) = self.messages.last_mut() {
            msg.streaming = false;
        }
    }

    pub fn add_system_message(&mut self, role: MessageRole, content: String) {
        self.messages.push(DisplayMessage {
            role,
            content,
            streaming: false,
        });
    }

    /// Process an incoming agent event.
    pub fn handle_event(&mut self, event: AgentEvent) {
        use betcode_proto::v1::agent_event::Event;

        match event.event {
            Some(Event::TextDelta(delta)) => {
                // Skip empty text deltas to avoid blank "Claude:" lines
                if delta.text.is_empty() && !delta.is_complete {
                    return;
                }
                if !delta.text.is_empty() {
                    if self.messages.last().is_none_or(|m| !m.streaming) {
                        self.start_assistant_message();
                    }
                    self.append_text(&delta.text);
                }
                if delta.is_complete {
                    self.finish_streaming();
                }
            }
            Some(Event::ToolCallStart(tool)) => {
                // Finish any open streaming message before tool output
                self.finish_streaming();
                let msg = if tool.description.is_empty() {
                    format!("[Tool: {}]", tool.tool_name)
                } else {
                    format!("[Tool: {} - {}]", tool.tool_name, tool.description)
                };
                self.add_system_message(MessageRole::Tool, msg);
            }
            Some(Event::ToolCallResult(result)) => {
                let status = if result.is_error { "ERROR" } else { "OK" };
                let preview = if result.output.len() > 200 {
                    format!("{}...", &result.output[..200])
                } else {
                    result.output.clone()
                };
                let msg = format!("[Tool Result ({}): {}]", status, preview);
                self.add_system_message(MessageRole::Tool, msg);
            }
            Some(Event::PermissionRequest(perm)) => {
                let original_input = perm.input.map(struct_to_json);
                self.mode = AppMode::PermissionPrompt;
                self.pending_permission = Some(PendingPermission {
                    request_id: perm.request_id,
                    tool_name: perm.tool_name,
                    description: perm.description,
                    original_input,
                    edit_buffer: String::new(),
                    edit_cursor: 0,
                    deny_interrupt: true,
                });
            }
            Some(Event::UserQuestion(q)) => {
                self.mode = AppMode::UserQuestion;
                self.pending_question = Some(PendingUserQuestion {
                    question_id: q.question_id,
                    question: q.question,
                    options: q.options.into_iter().map(|opt| QuestionOptionDisplay {
                        label: opt.label,
                        description: opt.description,
                    }).collect(),
                    multi_select: q.multi_select,
                    highlight: 0,
                    selected: Vec::new(),
                });
            }
            Some(Event::SessionInfo(info)) => {
                self.session_id = Some(info.session_id.clone());
                self.model = info.model.clone();
                self.status = format!("Session: {} | Model: {}", info.session_id, info.model);
            }
            Some(Event::StatusChange(sc)) => {
                // Only update status text if the message is non-empty,
                // otherwise we'd blank the session/model info from SessionInfo.
                if !sc.message.is_empty() {
                    self.status = sc.message;
                }
                self.agent_busy = sc.status == 1;
            }
            Some(Event::Usage(usage)) => {
                self.token_usage = Some(TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cost_usd: usage.cost_usd,
                });
            }
            Some(Event::TurnComplete(_)) => {
                self.finish_streaming();
                self.agent_busy = false;
            }
            Some(Event::Error(err)) => {
                let msg = format!("[Error: {} - {}]", err.code, err.message);
                self.add_system_message(MessageRole::System, msg);
                if err.is_fatal {
                    self.status = format!("Fatal error: {}", err.message);
                }
            }
            _ => {}
        }
    }

    /// Replay a historical event into the message list (non-interactive).
    ///
    /// Used when loading conversation history via ResumeSession.
    /// All messages are added as non-streaming. PermissionRequest events are
    /// skipped (historical, not actionable). StatusChange and Usage are skipped
    /// per user preference (system-internal events).
    pub fn load_history_event(&mut self, event: AgentEvent) {
        use betcode_proto::v1::agent_event::Event;

        match event.event {
            Some(Event::TextDelta(delta)) => {
                if delta.text.is_empty() {
                    return;
                }
                // For history replay, accumulate text into the last assistant
                // message if it exists; otherwise create a new one.
                let should_create = self
                    .messages
                    .last()
                    .map_or(true, |m| m.role != MessageRole::Assistant || !m.streaming);
                if should_create {
                    self.messages.push(DisplayMessage {
                        role: MessageRole::Assistant,
                        content: delta.text,
                        streaming: true, // temporary, will be finished
                    });
                } else if let Some(msg) = self.messages.last_mut() {
                    msg.content.push_str(&delta.text);
                }
                if delta.is_complete {
                    if let Some(msg) = self.messages.last_mut() {
                        msg.streaming = false;
                    }
                }
            }
            Some(Event::ToolCallStart(tool)) => {
                // Finish any open streaming message
                if let Some(msg) = self.messages.last_mut() {
                    msg.streaming = false;
                }
                let msg = if tool.description.is_empty() {
                    format!("[Tool: {}]", tool.tool_name)
                } else {
                    format!("[Tool: {} - {}]", tool.tool_name, tool.description)
                };
                self.add_system_message(MessageRole::Tool, msg);
            }
            Some(Event::ToolCallResult(result)) => {
                let status = if result.is_error { "ERROR" } else { "OK" };
                let preview = if result.output.len() > 200 {
                    format!("{}...", &result.output[..200])
                } else {
                    result.output.clone()
                };
                self.add_system_message(
                    MessageRole::Tool,
                    format!("[Tool Result ({}): {}]", status, preview),
                );
            }
            Some(Event::SessionInfo(info)) => {
                self.session_id = Some(info.session_id.clone());
                self.model = info.model.clone();
            }
            Some(Event::Error(err)) => {
                let msg = format!("[Error: {} - {}]", err.code, err.message);
                self.add_system_message(MessageRole::System, msg);
            }
            Some(Event::UserInput(input)) => {
                self.add_user_message(input.content);
            }
            Some(Event::TurnComplete(_)) => {
                // Finish any open streaming message
                if let Some(msg) = self.messages.last_mut() {
                    msg.streaming = false;
                }
            }
            // Skip: PermissionRequest (not actionable), StatusChange, Usage (system-internal)
            _ => {}
        }
    }

    /// Finalize history loading — ensure no messages are left in streaming state.
    pub fn finish_history_load(&mut self) {
        for msg in &mut self.messages {
            msg.streaming = false;
        }
    }

    /// Submit the current input.
    pub fn submit_input(&mut self) -> Option<String> {
        if self.input.is_empty() {
            return None;
        }
        let text = self.input.clone();
        self.input_history.push_front(text.clone());
        if self.input_history.len() > 100 {
            self.input_history.pop_back();
        }
        self.input.clear();
        self.cursor_pos = 0;
        self.history_index = None;
        self.add_user_message(text.clone());
        Some(text)
    }

    /// Navigate input history (up).
    pub fn history_up(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => 0,
            Some(i) => (i + 1).min(self.input_history.len().saturating_sub(1)),
        };
        self.history_index = Some(idx);
        self.input = self.input_history[idx].clone();
        self.cursor_pos = self.input.len();
    }

    /// Navigate input history (down).
    pub fn history_down(&mut self) {
        match self.history_index {
            None => {}
            Some(0) => {
                self.history_index = None;
                self.input.clear();
                self.cursor_pos = 0;
            }
            Some(i) => {
                let idx = i - 1;
                self.history_index = Some(idx);
                self.input = self.input_history[idx].clone();
                self.cursor_pos = self.input.len();
            }
        }
    }

    // -- Permission edit helpers --

    /// Initialize edit buffer from original tool input JSON and switch mode.
    pub fn start_permission_edit(&mut self) {
        if let Some(ref mut perm) = self.pending_permission {
            perm.edit_buffer = perm
                .original_input
                .as_ref()
                .and_then(|v| serde_json::to_string_pretty(v).ok())
                .unwrap_or_default();
            perm.edit_cursor = perm.edit_buffer.len();
        }
        self.mode = AppMode::PermissionEditInput;
    }

    /// Clear edit buffer and switch to comment or deny-message mode.
    pub fn start_permission_text(&mut self, mode: AppMode, interrupt: bool) {
        if let Some(ref mut perm) = self.pending_permission {
            perm.edit_buffer.clear();
            perm.edit_cursor = 0;
            perm.deny_interrupt = interrupt;
        }
        self.mode = mode;
    }

    // -- User question helpers --

    /// Move question highlight up or down.
    pub fn move_question_highlight(&mut self, delta: isize) {
        if let Some(ref mut q) = self.pending_question {
            if q.options.is_empty() {
                return;
            }
            let max = q.options.len() - 1;
            let current = q.highlight as isize;
            q.highlight = (current + delta).clamp(0, max as isize) as usize;
        }
    }

    /// Toggle selection of the highlighted option (for multi-select) or set it (single-select).
    pub fn select_question_option(&mut self, index: usize) {
        if let Some(ref mut q) = self.pending_question {
            if index >= q.options.len() {
                return;
            }
            q.highlight = index;
            if q.multi_select {
                if let Some(pos) = q.selected.iter().position(|&i| i == index) {
                    q.selected.remove(pos);
                } else {
                    q.selected.push(index);
                }
            } else {
                q.selected = vec![index];
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a prost_types::Struct to serde_json::Value.
fn struct_to_json(s: betcode_proto::prost_types::Struct) -> serde_json::Value {
    use betcode_proto::prost_types::value::Kind;
    fn value_to_json(v: betcode_proto::prost_types::Value) -> serde_json::Value {
        match v.kind {
            Some(Kind::NullValue(_)) => serde_json::Value::Null,
            Some(Kind::NumberValue(n)) => serde_json::json!(n),
            Some(Kind::StringValue(s)) => serde_json::Value::String(s),
            Some(Kind::BoolValue(b)) => serde_json::Value::Bool(b),
            Some(Kind::StructValue(s)) => struct_to_json(s),
            Some(Kind::ListValue(l)) => {
                serde_json::Value::Array(l.values.into_iter().map(value_to_json).collect())
            }
            None => serde_json::Value::Null,
        }
    }
    let map: serde_json::Map<String, serde_json::Value> = s
        .fields
        .into_iter()
        .map(|(k, v)| (k, value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

/// Convert a serde_json::Value back to prost_types::Struct.
pub fn json_to_struct(v: &serde_json::Value) -> betcode_proto::prost_types::Struct {
    use betcode_proto::prost_types::{value::Kind, ListValue, Struct, Value};
    fn json_to_value(v: &serde_json::Value) -> Value {
        Value {
            kind: Some(match v {
                serde_json::Value::Null => Kind::NullValue(0),
                serde_json::Value::Bool(b) => Kind::BoolValue(*b),
                serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
                serde_json::Value::String(s) => Kind::StringValue(s.clone()),
                serde_json::Value::Array(arr) => Kind::ListValue(ListValue {
                    values: arr.iter().map(json_to_value).collect(),
                }),
                serde_json::Value::Object(_) => Kind::StructValue(json_to_struct(v)),
            }),
        }
    }
    match v {
        serde_json::Value::Object(map) => Struct {
            fields: map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect(),
        },
        _ => Struct::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_app_state() {
        let app = App::new();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(!app.should_quit);
        assert!(!app.agent_busy);
        assert!(app.messages.is_empty());
    }

    #[test]
    fn submit_input_adds_message() {
        let mut app = App::new();
        app.input = "Hello".to_string();
        app.cursor_pos = 5;

        let text = app.submit_input();
        assert_eq!(text, Some("Hello".to_string()));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::User);
        assert!(app.input.is_empty());
    }

    #[test]
    fn streaming_text_appends() {
        let mut app = App::new();
        app.start_assistant_message();
        app.append_text("Hello ");
        app.append_text("world");
        app.finish_streaming();

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "Hello world");
        assert!(!app.messages[0].streaming);
    }

    #[test]
    fn input_history_navigation() {
        let mut app = App::new();
        app.input = "first".to_string();
        app.submit_input();
        app.input = "second".to_string();
        app.submit_input();

        app.history_up();
        assert_eq!(app.input, "second");
        app.history_up();
        assert_eq!(app.input, "first");
        app.history_down();
        assert_eq!(app.input, "second");
        app.history_down();
        assert!(app.input.is_empty());
    }

    #[test]
    fn empty_submit_returns_none() {
        let mut app = App::new();
        assert_eq!(app.submit_input(), None);
    }

    fn make_event(event: betcode_proto::v1::agent_event::Event) -> AgentEvent {
        AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(event),
        }
    }

    #[test]
    fn empty_text_delta_does_not_create_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: String::new(),
            is_complete: false,
        })));
        assert!(app.messages.is_empty(), "Empty text delta should not create a message");
    }

    #[test]
    fn text_delta_creates_and_appends() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Hello ".to_string(),
            is_complete: false,
        })));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "Hello ");
        assert!(app.messages[0].streaming);

        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "world".to_string(),
            is_complete: false,
        })));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "Hello world");
    }

    #[test]
    fn tool_call_start_finishes_streaming_and_adds_tool_msg() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // Start streaming text
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Let me check.".to_string(),
            is_complete: false,
        })));
        assert!(app.messages[0].streaming);

        // Tool call arrives — should finish streaming first
        app.handle_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Bash".to_string(),
            input: None,
            description: "ls -la".to_string(),
        })));
        assert_eq!(app.messages.len(), 2);
        assert!(!app.messages[0].streaming); // streaming finished
        assert_eq!(app.messages[0].role, MessageRole::Assistant);
        assert_eq!(app.messages[1].role, MessageRole::Tool);
        assert!(app.messages[1].content.contains("Bash"));
        assert!(app.messages[1].content.contains("ls -la"));
    }

    #[test]
    fn tool_call_start_empty_description_no_dash() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.handle_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Read".to_string(),
            input: None,
            description: String::new(),
        })));
        assert_eq!(app.messages[0].content, "[Tool: Read]");
    }

    #[test]
    fn status_change_empty_message_preserves_status() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.status = "Session: abc | Model: claude".to_string();

        app.handle_event(make_event(Event::StatusChange(betcode_proto::v1::StatusChange {
            status: 1, // Thinking
            message: String::new(),
        })));
        assert_eq!(app.status, "Session: abc | Model: claude");
        assert!(app.agent_busy);
    }

    #[test]
    fn status_change_with_message_updates_status() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.status = "old status".to_string();

        app.handle_event(make_event(Event::StatusChange(betcode_proto::v1::StatusChange {
            status: 0,
            message: "new status".to_string(),
        })));
        assert_eq!(app.status, "new status");
        assert!(!app.agent_busy);
    }

    #[test]
    fn text_after_tool_creates_new_assistant_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // Text → tool → text should create separate assistant messages
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "First".to_string(),
            is_complete: false,
        })));
        app.handle_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Bash".to_string(),
            input: None,
            description: "ls".to_string(),
        })));
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Second".to_string(),
            is_complete: false,
        })));

        assert_eq!(app.messages.len(), 3);
        assert_eq!(app.messages[0].content, "First");
        assert_eq!(app.messages[0].role, MessageRole::Assistant);
        assert_eq!(app.messages[1].role, MessageRole::Tool);
        assert_eq!(app.messages[2].content, "Second");
        assert_eq!(app.messages[2].role, MessageRole::Assistant);
    }

    // =========================================================================
    // History loading tests (load_history_event / finish_history_load)
    // =========================================================================

    #[test]
    fn history_text_creates_non_streaming_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Hello from history".to_string(),
            is_complete: true,
        })));

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::Assistant);
        assert_eq!(app.messages[0].content, "Hello from history");
        assert!(!app.messages[0].streaming, "History messages should not be streaming");
    }

    #[test]
    fn history_text_accumulates_chunks() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "chunk1 ".to_string(),
            is_complete: false,
        })));
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "chunk2".to_string(),
            is_complete: true,
        })));

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "chunk1 chunk2");
        assert!(!app.messages[0].streaming);
    }

    #[test]
    fn history_empty_text_skipped() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: String::new(),
            is_complete: false,
        })));

        assert!(app.messages.is_empty());
    }

    #[test]
    fn history_tool_call_adds_tool_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Bash".to_string(),
            input: None,
            description: "git status".to_string(),
        })));

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::Tool);
        assert!(app.messages[0].content.contains("Bash"));
        assert!(app.messages[0].content.contains("git status"));
    }

    #[test]
    fn history_tool_result_adds_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::ToolCallResult(betcode_proto::v1::ToolCallResult {
            tool_id: "t1".to_string(),
            output: "on branch main".to_string(),
            is_error: false,
            duration_ms: 100,
        })));

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::Tool);
        assert!(app.messages[0].content.contains("OK"));
        assert!(app.messages[0].content.contains("on branch main"));
    }

    #[test]
    fn history_session_info_sets_model() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::SessionInfo(betcode_proto::v1::SessionInfo {
            session_id: "s1".to_string(),
            model: "claude-sonnet-4".to_string(),
            working_directory: String::new(),
            worktree_id: String::new(),
            message_count: 0,
            is_resumed: false,
            is_compacted: false,
            context_usage_percent: 0.0,
        })));

        assert_eq!(app.session_id, Some("s1".to_string()));
        assert_eq!(app.model, "claude-sonnet-4");
        assert!(app.messages.is_empty(), "SessionInfo should not create a display message");
    }

    #[test]
    fn history_skips_permission_status_usage() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // PermissionRequest — historical, not actionable
        app.load_history_event(make_event(Event::PermissionRequest(betcode_proto::v1::PermissionRequest {
            request_id: "p1".to_string(),
            tool_name: "Bash".to_string(),
            description: "ls".to_string(),
            input: None,
        })));
        // StatusChange — transient
        app.load_history_event(make_event(Event::StatusChange(betcode_proto::v1::StatusChange {
            status: 1,
            message: "thinking".to_string(),
        })));
        // Usage — transient
        app.load_history_event(make_event(Event::Usage(betcode_proto::v1::UsageReport {
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.01,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: String::new(),
            duration_ms: 0,
        })));

        assert!(app.messages.is_empty(), "PermissionRequest, StatusChange, Usage should be skipped");
        assert_eq!(app.mode, AppMode::Normal, "PermissionRequest should not enter prompt mode");
    }

    #[test]
    fn history_error_adds_system_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::Error(betcode_proto::v1::ErrorEvent {
            code: "RATE_LIMIT".to_string(),
            message: "Too many requests".to_string(),
            is_fatal: false,
            details: Default::default(),
        })));

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::System);
        assert!(app.messages[0].content.contains("RATE_LIMIT"));
    }

    #[test]
    fn history_full_conversation_replay() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // Simulate a full historical turn: session_info → text → tool → result → text → turn_complete
        app.load_history_event(make_event(Event::SessionInfo(betcode_proto::v1::SessionInfo {
            session_id: "s1".to_string(),
            model: "claude-sonnet-4".to_string(),
            working_directory: String::new(),
            worktree_id: String::new(),
            message_count: 0,
            is_resumed: false,
            is_compacted: false,
            context_usage_percent: 0.0,
        })));
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Let me check.".to_string(),
            is_complete: true,
        })));
        app.load_history_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Bash".to_string(),
            input: None,
            description: "ls".to_string(),
        })));
        app.load_history_event(make_event(Event::ToolCallResult(betcode_proto::v1::ToolCallResult {
            tool_id: "t1".to_string(),
            output: "file.txt".to_string(),
            is_error: false,
            duration_ms: 50,
        })));
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Found file.txt".to_string(),
            is_complete: true,
        })));
        app.load_history_event(make_event(Event::TurnComplete(betcode_proto::v1::TurnComplete {
            stop_reason: "end_turn".to_string(),
        })));
        app.finish_history_load();

        assert_eq!(app.messages.len(), 4); // text, tool, result, text
        assert_eq!(app.messages[0].role, MessageRole::Assistant);
        assert_eq!(app.messages[0].content, "Let me check.");
        assert_eq!(app.messages[1].role, MessageRole::Tool);
        assert_eq!(app.messages[2].role, MessageRole::Tool);
        assert_eq!(app.messages[3].role, MessageRole::Assistant);
        assert_eq!(app.messages[3].content, "Found file.txt");

        // All messages should be non-streaming after finish_history_load
        for msg in &app.messages {
            assert!(!msg.streaming, "All history messages should be non-streaming");
        }
    }

    #[test]
    fn finish_history_load_closes_open_streaming() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // Text without is_complete — simulates interrupted history
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "partial".to_string(),
            is_complete: false,
        })));
        assert!(app.messages[0].streaming, "Should still be streaming before finish");

        app.finish_history_load();
        assert!(!app.messages[0].streaming, "finish_history_load should close streaming");
    }

    #[test]
    fn history_user_input_adds_user_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::UserInput(betcode_proto::v1::UserInput {
            content: "Hello Claude".to_string(),
        })));
        app.finish_history_load();

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::User);
        assert_eq!(app.messages[0].content, "Hello Claude");
    }

    #[test]
    fn history_user_input_interleaved_with_assistant() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // User prompt → assistant reply → user prompt → assistant reply
        app.load_history_event(make_event(Event::UserInput(betcode_proto::v1::UserInput {
            content: "What is 2+2?".to_string(),
        })));
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "4".to_string(),
            is_complete: true,
        })));
        app.load_history_event(make_event(Event::TurnComplete(betcode_proto::v1::TurnComplete {
            stop_reason: "end_turn".to_string(),
        })));
        app.load_history_event(make_event(Event::UserInput(betcode_proto::v1::UserInput {
            content: "And 3+3?".to_string(),
        })));
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "6".to_string(),
            is_complete: true,
        })));
        app.finish_history_load();

        assert_eq!(app.messages.len(), 4);
        assert_eq!(app.messages[0].role, MessageRole::User);
        assert_eq!(app.messages[0].content, "What is 2+2?");
        assert_eq!(app.messages[1].role, MessageRole::Assistant);
        assert_eq!(app.messages[1].content, "4");
        assert_eq!(app.messages[2].role, MessageRole::User);
        assert_eq!(app.messages[2].content, "And 3+3?");
        assert_eq!(app.messages[3].role, MessageRole::Assistant);
        assert_eq!(app.messages[3].content, "6");
    }
}
