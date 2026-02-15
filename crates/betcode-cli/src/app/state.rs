//! Application state and types.

use std::collections::VecDeque;

use crate::commands::cache::CommandCache;
use crate::tui::fingerprint_panel::FingerprintPrompt;
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
    /// Fingerprint verification panel (relay connections).
    FingerprintVerification,
}

/// A displayable message in the conversation.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: MessageRole,
    pub content: String,
    pub streaming: bool,
    /// Whether this message was created from a `ToolCallResult` event.
    /// Used by the renderer to skip inline result messages (the Done/Error
    /// status line already covers both start and result).
    pub is_tool_result: bool,
    /// Label for messages originating from a subagent (e.g. `"subagent"`).
    /// `None` for main-agent messages.
    pub agent_label: Option<String>,
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
    /// Original tool input from the `PermissionRequest` (Struct serialized to JSON).
    pub original_input: Option<serde_json::Value>,
    /// Text buffer for editing (tool input JSON / comment / deny message).
    pub edit_buffer: String,
    /// Cursor position in `edit_buffer`.
    pub edit_cursor: usize,
    /// Whether deny should interrupt the current turn (N=false, X=true).
    pub deny_interrupt: bool,
}

/// Pending question from Claude (`AskUserQuestion` tool).
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

/// Maximum number of completion items visible in the popup at once.
pub const COMPLETION_VISIBLE_COUNT: usize = 8;

/// Completion state for the inline autocomplete system.
#[derive(Debug, Clone, Default)]
pub struct CompletionState {
    /// Whether the completion popup is visible.
    pub popup_visible: bool,
    /// Current completion items (text labels).
    pub items: Vec<String>,
    /// Currently selected item index.
    pub selected_index: usize,
    /// Scroll offset for the visible window into `items`.
    pub scroll_offset: usize,
    /// Ghost text suffix to display after the typed text.
    pub ghost_text: Option<String>,
}

impl CompletionState {
    /// Adjust `scroll_offset` so that `selected_index` is within the visible window.
    pub const fn adjust_scroll(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + COMPLETION_VISIBLE_COUNT {
            self.scroll_offset = self.selected_index + 1 - COMPLETION_VISIBLE_COUNT;
        }
    }
}

/// State for the toggleable detail panel.
#[derive(Debug, Clone, Default)]
pub struct DetailPanelState {
    pub visible: bool,
    pub selected_tool_index: Option<usize>,
    pub scroll_offset: u16,
}

/// Token usage info for status bar.
#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
}

/// Status of a tracked tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    Running,
    Done,
    Error,
}

/// Structured tracking of a single tool invocation.
#[derive(Debug, Clone)]
pub struct ToolCallEntry {
    pub tool_id: String,
    pub tool_name: String,
    pub description: String,
    pub input_json: Option<String>,
    pub output: Option<String>,
    pub status: ToolCallStatus,
    pub duration_ms: Option<u32>,
    pub finished_at: Option<std::time::Instant>,
    /// Index of the "[Tool: ...]" message in `App.messages`.
    pub message_index: usize,
}

/// TUI application state.
#[allow(clippy::struct_excessive_bools)]
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
    pub pending_fingerprint: Option<FingerprintPrompt>,
    pub agent_busy: bool,
    pub completion_state: CompletionState,
    pub show_status_panel: bool,
    /// State for the toggleable detail panel (Ctrl+D).
    pub detail_panel: DetailPanelState,
    /// Connection type displayed in the status panel ("local" or "relay").
    pub connection_type: String,
    /// Structured tracking of tool call lifecycle.
    pub tool_calls: Vec<ToolCallEntry>,
    /// Current spinner animation tick (incremented by the ticker).
    pub spinner_tick: usize,
    /// Local cache of commands fetched from daemon for `/` completion.
    pub command_cache: CommandCache,
    /// Sender to request async completion fetches (agents, files).
    pub completion_request_tx: Option<tokio::sync::mpsc::Sender<CompletionRequest>>,
    /// Sender for slash-command execution requests.
    pub service_command_tx: Option<tokio::sync::mpsc::Sender<crate::tui::ServiceCommandExec>>,
}

/// A request for async completion data from the daemon.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub kind: CompletionFetchKind,
    pub query: String,
}

/// What kind of completion data to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionFetchKind {
    Agents,
    Files,
    /// Combined agents + files (used for `@text` without forced `@@`).
    Mixed,
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
            pending_fingerprint: None,
            agent_busy: false,
            completion_state: CompletionState::default(),
            show_status_panel: false,
            detail_panel: DetailPanelState::default(),
            connection_type: "local".to_string(),
            tool_calls: Vec::new(),
            spinner_tick: 0,
            command_cache: CommandCache::new(),
            completion_request_tx: None,
            service_command_tx: None,
        }
    }

    /// Update completion state based on current input and cursor position.
    pub fn update_completion_state(&mut self) {
        use crate::completion::controller::{CompletionTrigger, detect_trigger};

        let trigger = detect_trigger(&self.input, self.cursor_pos);

        match trigger {
            Some(CompletionTrigger::Command { ref query }) => {
                // Search local cache instantly for `/` commands.
                let results = self.command_cache.search(query, 50);
                self.completion_state.items = results.iter().map(|c| c.name.clone()).collect();
                self.completion_state.selected_index = 0;
                self.completion_state.scroll_offset = 0;
                self.completion_state.ghost_text = self.completion_state.items.first().cloned();
                self.completion_state.popup_visible = !self.completion_state.items.is_empty();
            }
            Some(CompletionTrigger::Agent { ref query, forced }) => {
                // `@@` → agents only; `@` → mixed (agents + files).
                let kind = if forced {
                    CompletionFetchKind::Agents
                } else {
                    CompletionFetchKind::Mixed
                };
                if let Some(tx) = &self.completion_request_tx {
                    let _ = tx.try_send(CompletionRequest {
                        kind,
                        query: query.clone(),
                    });
                }
                self.completion_state.ghost_text = self.completion_state.items.first().cloned();
            }
            Some(CompletionTrigger::File { ref query }) => {
                // Send async fetch request for file paths.
                if let Some(tx) = &self.completion_request_tx {
                    let _ = tx.try_send(CompletionRequest {
                        kind: CompletionFetchKind::Files,
                        query: query.clone(),
                    });
                }
                self.completion_state.ghost_text = self.completion_state.items.first().cloned();
            }
            Some(CompletionTrigger::Bash { .. }) => {
                // No completion for bash commands
                self.completion_state.ghost_text = None;
            }
            None => {
                // No trigger - clear completion state
                self.completion_state.ghost_text = None;
                self.completion_state.popup_visible = false;
                self.completion_state.items.clear();
                self.completion_state.selected_index = 0;
                self.completion_state.scroll_offset = 0;
            }
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
    pub const fn scroll_down(&mut self, n: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        if self.scroll_offset == 0 {
            self.scroll_pinned = true;
        }
    }

    /// Snap scroll to the bottom (most recent messages).
    pub const fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.scroll_pinned = true;
    }

    /// Scroll the detail panel content by `delta` lines (positive = down, negative = up).
    pub const fn scroll_detail_panel(&mut self, delta: i16) {
        if delta > 0 {
            self.detail_panel.scroll_offset = self
                .detail_panel
                .scroll_offset
                .saturating_add(delta.cast_unsigned());
        } else {
            self.detail_panel.scroll_offset = self
                .detail_panel
                .scroll_offset
                .saturating_sub(delta.unsigned_abs());
        }
    }

    /// Toggle the detail panel visibility.
    pub const fn toggle_detail_panel(&mut self) {
        self.detail_panel.visible = !self.detail_panel.visible;
        if self.detail_panel.visible
            && self.detail_panel.selected_tool_index.is_none()
            && !self.tool_calls.is_empty()
        {
            self.detail_panel.selected_tool_index = Some(self.tool_calls.len() - 1);
        }
    }

    /// Select the next tool call in the detail panel (wrapping).
    pub fn select_next_tool(&mut self) {
        if self.tool_calls.is_empty() {
            return;
        }
        let len = self.tool_calls.len();
        self.detail_panel.selected_tool_index = Some(
            self.detail_panel
                .selected_tool_index
                .map_or(0, |i| (i + 1) % len),
        );
        self.detail_panel.scroll_offset = 0;
    }

    /// Select the previous tool call in the detail panel (wrapping).
    pub fn select_prev_tool(&mut self) {
        if self.tool_calls.is_empty() {
            return;
        }
        let len = self.tool_calls.len();
        self.detail_panel.selected_tool_index = Some(
            self.detail_panel
                .selected_tool_index
                .map_or(len - 1, |i| if i == 0 { len - 1 } else { i - 1 }),
        );
        self.detail_panel.scroll_offset = 0;
    }

    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(DisplayMessage {
            role: MessageRole::User,
            content,
            streaming: false,
            is_tool_result: false,
            agent_label: None,
        });
    }

    pub fn start_assistant_message(&mut self) {
        self.messages.push(DisplayMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            streaming: true,
            is_tool_result: false,
            agent_label: None,
        });
    }

    pub fn append_text(&mut self, text: &str) {
        if let Some(msg) = self.messages.last_mut()
            && msg.streaming
        {
            msg.content.push_str(text);
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
            is_tool_result: false,
            agent_label: None,
        });
    }

    /// Process an incoming agent event.
    #[allow(clippy::too_many_lines)]
    pub fn handle_event(&mut self, event: AgentEvent) {
        use betcode_proto::v1::agent_event::Event;

        let agent_label = if event.parent_tool_use_id.is_empty() {
            None
        } else {
            Some("subagent".to_string())
        };

        match event.event {
            Some(Event::TextDelta(delta)) => {
                // Skip empty text deltas to avoid blank "Claude:" lines
                if delta.text.is_empty() && !delta.is_complete {
                    return;
                }
                if !delta.text.is_empty() {
                    if self.messages.last().is_none_or(|m| !m.streaming) {
                        self.start_assistant_message();
                        if let Some(msg) = self.messages.last_mut() {
                            msg.agent_label.clone_from(&agent_label);
                        }
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
                if let Some(last) = self.messages.last_mut() {
                    last.agent_label.clone_from(&agent_label);
                }
                self.tool_calls.push(ToolCallEntry {
                    tool_id: tool.tool_id.clone(),
                    tool_name: tool.tool_name.clone(),
                    description: tool.description.clone(),
                    input_json: tool.input.as_ref().map(|s| format!("{s:?}")),
                    output: None,
                    status: ToolCallStatus::Running,
                    duration_ms: None,
                    finished_at: None,
                    message_index: self.messages.len() - 1,
                });
            }
            Some(Event::ToolCallResult(result)) => {
                let status = if result.is_error { "ERROR" } else { "OK" };
                let preview = if result.output.len() > 200 {
                    format!("{}...", &result.output[..200])
                } else {
                    result.output.clone()
                };
                let msg = format!("[Tool Result ({status}): {preview}]");
                self.add_system_message(MessageRole::Tool, msg);
                // Mark as tool result so the renderer can skip it
                if let Some(last) = self.messages.last_mut() {
                    last.is_tool_result = true;
                    last.agent_label.clone_from(&agent_label);
                }
                if let Some(entry) = self
                    .tool_calls
                    .iter_mut()
                    .rev()
                    .find(|e| e.tool_id == result.tool_id)
                {
                    entry.status = if result.is_error {
                        ToolCallStatus::Error
                    } else {
                        ToolCallStatus::Done
                    };
                    entry.output = Some(result.output);
                    entry.duration_ms = Some(result.duration_ms);
                    entry.finished_at = Some(std::time::Instant::now());
                }
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
                    options: q
                        .options
                        .into_iter()
                        .map(|opt| QuestionOptionDisplay {
                            label: opt.label,
                            description: opt.description,
                        })
                        .collect(),
                    multi_select: q.multi_select,
                    highlight: 0,
                    selected: Vec::new(),
                });
            }
            Some(Event::SessionInfo(info)) => {
                self.session_id = Some(info.session_id.clone());
                self.model.clone_from(&info.model);
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
                if let Some(last) = self.messages.last_mut() {
                    last.agent_label.clone_from(&agent_label);
                }
                if err.is_fatal {
                    self.status = format!("Fatal error: {}", err.message);
                }
            }
            _ => {}
        }
    }

    /// Replay a historical event into the message list (non-interactive).
    ///
    /// Used when loading conversation history via `ResumeSession`.
    /// All messages are added as non-streaming. `PermissionRequest` events are
    /// skipped (historical, not actionable). `StatusChange` and Usage are skipped
    /// per user preference (system-internal events).
    #[allow(clippy::too_many_lines)]
    pub fn load_history_event(&mut self, event: AgentEvent) {
        use betcode_proto::v1::agent_event::Event;

        let agent_label = if event.parent_tool_use_id.is_empty() {
            None
        } else {
            Some("subagent".to_string())
        };

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
                    .is_none_or(|m| m.role != MessageRole::Assistant || !m.streaming);
                if should_create {
                    self.messages.push(DisplayMessage {
                        role: MessageRole::Assistant,
                        content: delta.text,
                        streaming: true, // temporary, will be finished
                        is_tool_result: false,
                        agent_label,
                    });
                } else if let Some(msg) = self.messages.last_mut() {
                    msg.content.push_str(&delta.text);
                }
                if delta.is_complete
                    && let Some(msg) = self.messages.last_mut()
                {
                    msg.streaming = false;
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
                if let Some(last) = self.messages.last_mut() {
                    last.agent_label.clone_from(&agent_label);
                }
                self.tool_calls.push(ToolCallEntry {
                    tool_id: tool.tool_id.clone(),
                    tool_name: tool.tool_name.clone(),
                    description: tool.description.clone(),
                    input_json: tool.input.as_ref().map(|s| format!("{s:?}")),
                    output: None,
                    status: ToolCallStatus::Running,
                    duration_ms: None,
                    finished_at: None,
                    message_index: self.messages.len() - 1,
                });
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
                    format!("[Tool Result ({status}): {preview}]"),
                );
                // Mark as tool result so the renderer can skip it
                if let Some(last) = self.messages.last_mut() {
                    last.is_tool_result = true;
                    last.agent_label.clone_from(&agent_label);
                }
                if let Some(entry) = self
                    .tool_calls
                    .iter_mut()
                    .rev()
                    .find(|e| e.tool_id == result.tool_id)
                {
                    entry.status = if result.is_error {
                        ToolCallStatus::Error
                    } else {
                        ToolCallStatus::Done
                    };
                    entry.output = Some(result.output);
                    entry.duration_ms = Some(result.duration_ms);
                    // History replay: original wall-clock time is lost, leave as None
                    entry.finished_at = None;
                }
            }
            Some(Event::SessionInfo(info)) => {
                self.session_id = Some(info.session_id.clone());
                self.model = info.model;
            }
            Some(Event::Error(err)) => {
                let msg = format!("[Error: {} - {}]", err.code, err.message);
                self.add_system_message(MessageRole::System, msg);
                if let Some(last) = self.messages.last_mut() {
                    last.agent_label.clone_from(&agent_label);
                }
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
            #[allow(clippy::cast_possible_wrap)]
            let current = q.highlight as isize;
            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
            {
                q.highlight = (current + delta).clamp(0, max as isize) as usize;
            }
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

/// Convert a `prost_types::Struct` to `serde_json::Value`.
fn struct_to_json(s: betcode_proto::prost_types::Struct) -> serde_json::Value {
    use betcode_proto::prost_types::value::Kind;
    fn value_to_json(v: betcode_proto::prost_types::Value) -> serde_json::Value {
        match v.kind {
            Some(Kind::NullValue(_)) | None => serde_json::Value::Null,
            Some(Kind::NumberValue(n)) => serde_json::json!(n),
            Some(Kind::StringValue(s)) => serde_json::Value::String(s),
            Some(Kind::BoolValue(b)) => serde_json::Value::Bool(b),
            Some(Kind::StructValue(s)) => struct_to_json(s),
            Some(Kind::ListValue(l)) => {
                serde_json::Value::Array(l.values.into_iter().map(value_to_json).collect())
            }
        }
    }
    let map: serde_json::Map<String, serde_json::Value> = s
        .fields
        .into_iter()
        .map(|(k, v)| (k, value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

/// Convert a `serde_json::Value` back to `prost_types::Struct`.
pub fn json_to_struct(v: &serde_json::Value) -> betcode_proto::prost_types::Struct {
    use betcode_proto::prost_types::{ListValue, Struct, Value, value::Kind};
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
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::default_trait_access
)]
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
        assert!(
            app.messages.is_empty(),
            "Empty text delta should not create a message"
        );
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
        app.handle_event(make_event(Event::ToolCallStart(
            betcode_proto::v1::ToolCallStart {
                tool_id: "t1".to_string(),
                tool_name: "Bash".to_string(),
                input: None,
                description: "ls -la".to_string(),
            },
        )));
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
        app.handle_event(make_event(Event::ToolCallStart(
            betcode_proto::v1::ToolCallStart {
                tool_id: "t1".to_string(),
                tool_name: "Read".to_string(),
                input: None,
                description: String::new(),
            },
        )));
        assert_eq!(app.messages[0].content, "[Tool: Read]");
    }

    #[test]
    fn status_change_empty_message_preserves_status() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.status = "Session: abc | Model: claude".to_string();

        app.handle_event(make_event(Event::StatusChange(
            betcode_proto::v1::StatusChange {
                status: 1, // Thinking
                message: String::new(),
            },
        )));
        assert_eq!(app.status, "Session: abc | Model: claude");
        assert!(app.agent_busy);
    }

    #[test]
    fn status_change_with_message_updates_status() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.status = "old status".to_string();

        app.handle_event(make_event(Event::StatusChange(
            betcode_proto::v1::StatusChange {
                status: 0,
                message: "new status".to_string(),
            },
        )));
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
        app.handle_event(make_event(Event::ToolCallStart(
            betcode_proto::v1::ToolCallStart {
                tool_id: "t1".to_string(),
                tool_name: "Bash".to_string(),
                input: None,
                description: "ls".to_string(),
            },
        )));
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
        assert!(
            !app.messages[0].streaming,
            "History messages should not be streaming"
        );
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

        app.load_history_event(make_event(Event::ToolCallStart(
            betcode_proto::v1::ToolCallStart {
                tool_id: "t1".to_string(),
                tool_name: "Bash".to_string(),
                input: None,
                description: "git status".to_string(),
            },
        )));

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::Tool);
        assert!(app.messages[0].content.contains("Bash"));
        assert!(app.messages[0].content.contains("git status"));
    }

    #[test]
    fn history_tool_result_adds_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::ToolCallResult(
            betcode_proto::v1::ToolCallResult {
                tool_id: "t1".to_string(),
                output: "on branch main".to_string(),
                is_error: false,
                duration_ms: 100,
            },
        )));

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::Tool);
        assert!(app.messages[0].content.contains("OK"));
        assert!(app.messages[0].content.contains("on branch main"));
    }

    #[test]
    fn history_session_info_sets_model() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.load_history_event(make_event(Event::SessionInfo(
            betcode_proto::v1::SessionInfo {
                session_id: "s1".to_string(),
                model: "claude-sonnet-4".to_string(),
                working_directory: String::new(),
                worktree_id: String::new(),
                message_count: 0,
                is_resumed: false,
                is_compacted: false,
                context_usage_percent: 0.0,
            },
        )));

        assert_eq!(app.session_id, Some("s1".to_string()));
        assert_eq!(app.model, "claude-sonnet-4");
        assert!(
            app.messages.is_empty(),
            "SessionInfo should not create a display message"
        );
    }

    #[test]
    fn history_skips_permission_status_usage() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // PermissionRequest — historical, not actionable
        app.load_history_event(make_event(Event::PermissionRequest(
            betcode_proto::v1::PermissionRequest {
                request_id: "p1".to_string(),
                tool_name: "Bash".to_string(),
                description: "ls".to_string(),
                input: None,
            },
        )));
        // StatusChange — transient
        app.load_history_event(make_event(Event::StatusChange(
            betcode_proto::v1::StatusChange {
                status: 1,
                message: "thinking".to_string(),
            },
        )));
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

        assert!(
            app.messages.is_empty(),
            "PermissionRequest, StatusChange, Usage should be skipped"
        );
        assert_eq!(
            app.mode,
            AppMode::Normal,
            "PermissionRequest should not enter prompt mode"
        );
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
        app.load_history_event(make_event(Event::SessionInfo(
            betcode_proto::v1::SessionInfo {
                session_id: "s1".to_string(),
                model: "claude-sonnet-4".to_string(),
                working_directory: String::new(),
                worktree_id: String::new(),
                message_count: 0,
                is_resumed: false,
                is_compacted: false,
                context_usage_percent: 0.0,
            },
        )));
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Let me check.".to_string(),
            is_complete: true,
        })));
        app.load_history_event(make_event(Event::ToolCallStart(
            betcode_proto::v1::ToolCallStart {
                tool_id: "t1".to_string(),
                tool_name: "Bash".to_string(),
                input: None,
                description: "ls".to_string(),
            },
        )));
        app.load_history_event(make_event(Event::ToolCallResult(
            betcode_proto::v1::ToolCallResult {
                tool_id: "t1".to_string(),
                output: "file.txt".to_string(),
                is_error: false,
                duration_ms: 50,
            },
        )));
        app.load_history_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Found file.txt".to_string(),
            is_complete: true,
        })));
        app.load_history_event(make_event(Event::TurnComplete(
            betcode_proto::v1::TurnComplete {
                stop_reason: "end_turn".to_string(),
            },
        )));
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
            assert!(
                !msg.streaming,
                "All history messages should be non-streaming"
            );
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
        assert!(
            app.messages[0].streaming,
            "Should still be streaming before finish"
        );

        app.finish_history_load();
        assert!(
            !app.messages[0].streaming,
            "finish_history_load should close streaming"
        );
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
    fn fingerprint_verification_mode_exists() {
        let mut app = App::new();
        app.mode = AppMode::FingerprintVerification;
        assert_eq!(app.mode, AppMode::FingerprintVerification);
    }

    #[test]
    fn pending_fingerprint_defaults_to_none() {
        let app = App::new();
        assert!(app.pending_fingerprint.is_none());
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
        app.load_history_event(make_event(Event::TurnComplete(
            betcode_proto::v1::TurnComplete {
                stop_reason: "end_turn".to_string(),
            },
        )));
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

    // =========================================================================
    // ToolCallEntry lifecycle tests
    // =========================================================================

    #[test]
    fn tool_call_entry_tracks_lifecycle() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        assert!(app.tool_calls.is_empty());

        app.handle_event(make_event(Event::ToolCallStart(
            betcode_proto::v1::ToolCallStart {
                tool_id: "t1".to_string(),
                tool_name: "Read".to_string(),
                input: None,
                description: "/src/main.rs".to_string(),
            },
        )));

        assert_eq!(app.tool_calls.len(), 1);
        assert_eq!(app.tool_calls[0].tool_name, "Read");
        assert!(matches!(app.tool_calls[0].status, ToolCallStatus::Running));
        assert!(app.tool_calls[0].finished_at.is_none());

        app.handle_event(make_event(Event::ToolCallResult(
            betcode_proto::v1::ToolCallResult {
                tool_id: "t1".to_string(),
                output: "file contents here".to_string(),
                is_error: false,
                duration_ms: 150,
            },
        )));

        assert_eq!(app.tool_calls.len(), 1);
        assert!(matches!(app.tool_calls[0].status, ToolCallStatus::Done));
        assert_eq!(
            app.tool_calls[0].output.as_deref(),
            Some("file contents here")
        );
        assert_eq!(app.tool_calls[0].duration_ms, Some(150));
    }

    #[test]
    fn tool_call_entry_tracks_error() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        app.handle_event(make_event(Event::ToolCallStart(
            betcode_proto::v1::ToolCallStart {
                tool_id: "t2".to_string(),
                tool_name: "Bash".to_string(),
                input: None,
                description: "rm -rf /".to_string(),
            },
        )));

        app.handle_event(make_event(Event::ToolCallResult(
            betcode_proto::v1::ToolCallResult {
                tool_id: "t2".to_string(),
                output: "permission denied".to_string(),
                is_error: true,
                duration_ms: 50,
            },
        )));

        assert!(matches!(app.tool_calls[0].status, ToolCallStatus::Error));
    }

    // =========================================================================
    // CompletionState scroll tests
    // =========================================================================

    #[test]
    fn completion_scroll_stays_zero_within_visible() {
        let mut cs = CompletionState {
            items: (0..5).map(|i| format!("item-{i}")).collect(),
            selected_index: 4,
            scroll_offset: 0,
            ..Default::default()
        };
        cs.adjust_scroll();
        assert_eq!(cs.scroll_offset, 0, "5 items fit in 8-item window");
    }

    #[test]
    fn completion_scroll_follows_selection_down() {
        let mut cs = CompletionState {
            items: (0..20).map(|i| format!("item-{i}")).collect(),
            selected_index: 0,
            scroll_offset: 0,
            ..Default::default()
        };
        // Simulate pressing Down until past the visible window
        for target in 1..=12 {
            cs.selected_index = target;
            cs.adjust_scroll();
        }
        // selected=12 means scroll_offset must be at least 12+1-8=5
        assert_eq!(cs.scroll_offset, 5);
        assert!(cs.selected_index >= cs.scroll_offset);
        assert!(cs.selected_index < cs.scroll_offset + COMPLETION_VISIBLE_COUNT);
    }

    #[test]
    fn completion_scroll_follows_selection_up() {
        let mut cs = CompletionState {
            items: (0..20).map(|i| format!("item-{i}")).collect(),
            selected_index: 10,
            scroll_offset: 5,
            ..Default::default()
        };
        // Simulate pressing Up back to item 3 (below current scroll_offset)
        cs.selected_index = 3;
        cs.adjust_scroll();
        assert_eq!(cs.scroll_offset, 3);
    }

    #[test]
    fn completion_scroll_wraps_down_to_zero() {
        let mut cs = CompletionState {
            items: (0..20).map(|i| format!("item-{i}")).collect(),
            selected_index: 19,
            scroll_offset: 12,
            ..Default::default()
        };
        // Wrap from last to first
        cs.selected_index = 0;
        cs.adjust_scroll();
        assert_eq!(cs.scroll_offset, 0);
    }

    #[test]
    fn completion_scroll_wraps_up_to_end() {
        let mut cs = CompletionState {
            items: (0..20).map(|i| format!("item-{i}")).collect(),
            selected_index: 0,
            scroll_offset: 0,
            ..Default::default()
        };
        // Wrap from first to last
        cs.selected_index = 19;
        cs.adjust_scroll();
        assert_eq!(cs.scroll_offset, 12); // 19+1-8=12
    }

    // =========================================================================
    // DetailPanelState tests
    // =========================================================================

    #[test]
    fn detail_panel_toggles() {
        let mut app = App::new();
        assert!(!app.detail_panel.visible);

        app.toggle_detail_panel();
        assert!(app.detail_panel.visible);

        app.toggle_detail_panel();
        assert!(!app.detail_panel.visible);
    }

    #[test]
    fn detail_panel_scroll() {
        let mut app = App::new();
        app.detail_panel.visible = true;
        app.detail_panel.selected_tool_index = Some(0);

        app.scroll_detail_panel(5);
        assert_eq!(app.detail_panel.scroll_offset, 5);

        app.scroll_detail_panel(-3);
        assert_eq!(app.detail_panel.scroll_offset, 2);

        // Can't scroll below 0
        app.scroll_detail_panel(-10);
        assert_eq!(app.detail_panel.scroll_offset, 0);
    }

    #[test]
    fn detail_panel_selects_tool() {
        let mut app = App::new();
        // Add two tool calls
        app.tool_calls.push(ToolCallEntry {
            tool_id: "t1".to_string(),
            tool_name: "Read".to_string(),
            description: String::new(),
            input_json: None,
            output: Some("output1".to_string()),
            status: ToolCallStatus::Done,
            duration_ms: Some(100),
            finished_at: None,
            message_index: 0,
        });
        app.tool_calls.push(ToolCallEntry {
            tool_id: "t2".to_string(),
            tool_name: "Bash".to_string(),
            description: String::new(),
            input_json: None,
            output: Some("output2".to_string()),
            status: ToolCallStatus::Done,
            duration_ms: Some(200),
            finished_at: None,
            message_index: 1,
        });

        // Defaults to last tool call
        app.toggle_detail_panel();
        assert_eq!(app.detail_panel.selected_tool_index, Some(1));

        app.select_prev_tool();
        assert_eq!(app.detail_panel.selected_tool_index, Some(0));

        app.select_prev_tool();
        // Wraps to end
        assert_eq!(app.detail_panel.selected_tool_index, Some(1));

        app.select_next_tool();
        assert_eq!(app.detail_panel.selected_tool_index, Some(0));
    }

    // =========================================================================
    // Agent identity (agent_label) tests
    // =========================================================================

    #[test]
    fn messages_tagged_with_agent_context() {
        let mut app = App::new();

        // Event from main agent (empty parent_tool_use_id)
        app.handle_event(AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "Hello".to_string(),
                    is_complete: true,
                },
            )),
        });

        assert!(app.messages.last().unwrap().agent_label.is_none());

        // Event from subagent
        app.handle_event(AgentEvent {
            sequence: 2,
            timestamp: None,
            parent_tool_use_id: "tool-abc-123".to_string(),
            event: Some(betcode_proto::v1::agent_event::Event::TextDelta(
                betcode_proto::v1::TextDelta {
                    text: "Sub output".to_string(),
                    is_complete: true,
                },
            )),
        });

        assert_eq!(
            app.messages.last().unwrap().agent_label.as_deref(),
            Some("subagent")
        );
    }
}
