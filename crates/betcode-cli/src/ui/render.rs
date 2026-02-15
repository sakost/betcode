//! TUI rendering functions.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::detail_panel;
use super::panels;
use crate::app::{App, AppMode, DisplayMessage, MessageRole, ToolCallEntry, ToolCallStatus};

/// Which panel to show at the bottom.
enum BottomPanel {
    Input,
    Permission,
    PermissionEdit,
    Question,
    Fingerprint,
}

const fn bottom_panel_mode(app: &App) -> BottomPanel {
    match app.mode {
        AppMode::PermissionPrompt => BottomPanel::Permission,
        AppMode::PermissionEditInput
        | AppMode::PermissionComment
        | AppMode::PermissionDenyMessage => BottomPanel::PermissionEdit,
        AppMode::UserQuestion => BottomPanel::Question,
        AppMode::FingerprintVerification => BottomPanel::Fingerprint,
        _ => BottomPanel::Input,
    }
}

/// Draw the full UI.
#[allow(clippy::too_many_lines)]
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    use crate::app::COMPLETION_VISIBLE_COUNT;
    use crate::ui::status_panel::{SessionStatusInfo, render_status_panel};

    let bottom_panel = bottom_panel_mode(app);

    let frame_width = frame.area().width;
    // 2 for borders + 2 for horizontal padding
    let inner_input_width = frame_width.saturating_sub(4) as usize;
    let bottom_height = match bottom_panel {
        BottomPanel::Input => {
            let input_lines = if inner_input_width == 0 || app.input.is_empty() {
                1
            } else {
                let (last_row, last_col) =
                    compute_wrapped_cursor(&app.input, app.input.len(), inner_input_width);
                if last_col == 0 && last_row > 0 {
                    last_row
                } else {
                    last_row + 1
                }
            };
            let max_input_height = frame.area().height / 3;
            (input_lines + 2).min(max_input_height).max(3)
        }
        BottomPanel::Permission => 8u16.min(frame.area().height / 3).max(5),
        BottomPanel::PermissionEdit => 6u16.min(frame.area().height / 3).max(4),
        BottomPanel::Question => {
            let opt_count = app.pending_question.as_ref().map_or(0, |q| q.options.len());
            #[allow(clippy::cast_possible_truncation)]
            let lines = 2 + opt_count as u16 + 2;
            (lines + 2).min(frame.area().height / 2).max(6)
        }
        BottomPanel::Fingerprint => 20u16.min(frame.area().height * 2 / 3).max(12),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(bottom_height),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);

    if app.detail_panel.visible {
        let msg_area = chunks[1];
        let (conv_w, panel_w) = compute_detail_split(msg_area.width);
        if conv_w == 0 {
            // Overlay mode: detail panel covers the entire message area
            draw_detail_panel(frame, app, msg_area);
        } else {
            let horiz = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(conv_w), Constraint::Length(panel_w)])
                .split(msg_area);
            draw_messages(frame, app, horiz[0]);
            draw_detail_panel(frame, app, horiz[1]);
        }
    } else {
        draw_messages(frame, app, chunks[1]);
    }

    match bottom_panel {
        BottomPanel::Input => draw_input(frame, app, chunks[2]),
        BottomPanel::Permission => panels::draw_permission_panel(frame, app, chunks[2]),
        BottomPanel::PermissionEdit => panels::draw_permission_edit_panel(frame, app, chunks[2]),
        BottomPanel::Question => panels::draw_question_panel(frame, app, chunks[2]),
        BottomPanel::Fingerprint => {
            if let Some(ref prompt) = app.pending_fingerprint {
                panels::draw_fingerprint_panel(frame, prompt, chunks[2]);
            }
        }
    }

    draw_status_bar(frame, app, chunks[3]);

    // Render completion popup overlay if visible
    let area = frame.area();
    if app.completion_state.popup_visible && !app.completion_state.items.is_empty() {
        let visible_count = app
            .completion_state
            .items
            .len()
            .min(COMPLETION_VISIBLE_COUNT);
        #[allow(clippy::cast_possible_truncation)]
        let popup_height = visible_count as u16 + 2; // +2 for borders

        let popup_area = Rect {
            x: area.x + 1,
            y: area.height.saturating_sub(bottom_height + popup_height + 1),
            width: area.width.saturating_sub(2).min(60),
            height: popup_height,
        };

        let offset = app.completion_state.scroll_offset;
        let end = (offset + COMPLETION_VISIBLE_COUNT).min(app.completion_state.items.len());
        let items: Vec<Line<'_>> = app.completion_state.items[offset..end]
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let actual_index = offset + i;
                let style = if actual_index == app.completion_state.selected_index {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(item.clone(), style))
            })
            .collect();

        let popup = Paragraph::new(items)
            .block(Block::default().borders(Borders::ALL).title("Completions"));
        frame.render_widget(Clear, popup_area);
        frame.render_widget(popup, popup_area);
    }

    // Render status panel overlay if visible
    if app.show_status_panel {
        let info = SessionStatusInfo {
            cwd: std::env::current_dir()
                .map_or_else(|_| "unknown".to_string(), |p| p.display().to_string()),
            session_id: app.session_id.clone().unwrap_or_else(|| "none".to_string()),
            connection: app.connection_type.clone(),
            model: app.model.clone(),
            active_agents: 0,
            pending_permissions: usize::from(app.pending_permission.is_some()),
            worktree: None,
            uptime_secs: 0,
        };
        render_status_panel(frame, frame.area(), &info);
    }
}

fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let model_info = if app.model.is_empty() {
        "BetCode".to_string()
    } else {
        format!("BetCode | {}", app.model)
    };
    let session_info = app
        .session_id
        .as_deref()
        .map(|s| format!(" | Session: {}", &s[..8.min(s.len())]))
        .unwrap_or_default();
    let busy = if app.agent_busy { " [thinking...]" } else { "" };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            model_info,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(session_info),
        Span::styled(busy, Style::default().fg(Color::Yellow)),
    ]));
    frame.render_widget(header, area);
}

/// Filter messages for display, optionally hiding tool result messages.
///
/// When `detail_panel_visible` is `true`, messages with `is_tool_result` set
/// are excluded because the detail panel already provides the full tool output.
/// Returns an iterator of `(original_index, &DisplayMessage)` so callers can
/// still look up the corresponding [`ToolCallEntry`] by `message_index`.
pub fn filter_visible_messages(
    messages: &[DisplayMessage],
    detail_panel_visible: bool,
) -> impl Iterator<Item = (usize, &DisplayMessage)> {
    messages
        .iter()
        .enumerate()
        .filter(move |(_, m)| !(detail_panel_visible && m.is_tool_result))
}

#[allow(clippy::too_many_lines)]
fn draw_messages(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let mut lines: Vec<Line<'_>> = Vec::new();

    for (rendered_count, (i, msg)) in
        filter_visible_messages(&app.messages, app.detail_panel.visible).enumerate()
    {
        // Add empty line separator between rendered messages
        if rendered_count > 0 {
            lines.push(Line::from(""));
        }

        // For Tool role messages, look up the corresponding ToolCallEntry and
        // render with format_tool_status_line + color instead of raw text.
        if msg.role == MessageRole::Tool
            && let Some(entry) = app.tool_calls.iter().find(|e| e.message_index == i)
        {
            let text = format_tool_status_line(entry, app.spinner_tick);
            let color = match entry.status {
                ToolCallStatus::Running => Color::Cyan,
                ToolCallStatus::Done => Color::Green,
                ToolCallStatus::Error => Color::Red,
            };
            lines.push(Line::from(Span::styled(text, Style::default().fg(color))));
            continue;
        }

        let (prefix, color) = match msg.role {
            MessageRole::User => ("You: ", Color::Green),
            MessageRole::Assistant => ("Claude: ", Color::Blue),
            MessageRole::System => ("System: ", Color::Yellow),
            MessageRole::Tool => ("", Color::DarkGray),
        };
        let prefix_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let content_lines: Vec<&str> = msg.content.split('\n').collect();

        // Build optional agent label span
        let mut label_spans: Vec<Span<'_>> = Vec::new();
        if let Some(ref label) = msg.agent_label {
            label_spans.push(Span::styled(
                format!("[{label}] "),
                Style::default().fg(Color::Magenta),
            ));
        }

        if content_lines.is_empty() || (content_lines.len() == 1 && content_lines[0].is_empty()) {
            let cursor = if msg.streaming { "\u{2588}" } else { "" };
            let mut spans = label_spans.clone();
            spans.push(Span::styled(prefix, prefix_style));
            spans.push(Span::styled(cursor, Style::default().fg(Color::White)));
            lines.push(Line::from(spans));
        } else {
            let cursor = if msg.streaming && content_lines.len() == 1 {
                "\u{2588}"
            } else {
                ""
            };
            let mut spans = label_spans;
            spans.push(Span::styled(prefix, prefix_style));
            spans.push(Span::raw(content_lines[0]));
            spans.push(Span::styled(cursor, Style::default().fg(Color::White)));
            lines.push(Line::from(spans));
            let indent = " ".repeat(prefix.len());
            for (j, content_line) in content_lines.iter().enumerate().skip(1) {
                let cursor = if msg.streaming && j == content_lines.len() - 1 {
                    "\u{2588}"
                } else {
                    ""
                };
                lines.push(Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::raw(*content_line),
                    Span::styled(cursor, Style::default().fg(Color::White)),
                ]));
            }
        }
    }

    // 2 for borders + 2 for padding on each axis
    let inner_height = area.height.saturating_sub(4);
    let inner_width = area.width.saturating_sub(4);

    // Use ratatui's own word-wrap line count so scroll range exactly matches
    // what the Paragraph widget actually renders.
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    #[allow(clippy::cast_possible_truncation)]
    let total = paragraph.line_count(inner_width) as u16;

    app.viewport_height = inner_height;
    app.total_lines = total;

    let max_scroll = total.saturating_sub(inner_height);
    let scroll = if app.scroll_pinned {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_offset)
    };

    let title = if app.scroll_pinned {
        "Conversation".to_string()
    } else {
        let position = max_scroll.saturating_sub(scroll);
        format!(
            "Conversation [scroll: {}/{}]",
            position.min(max_scroll),
            max_scroll
        )
    };

    let messages = paragraph
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .padding(Padding::uniform(1)),
        )
        .scroll((scroll, 0));
    frame.render_widget(messages, area);
}

#[allow(clippy::option_if_let_else)]
fn draw_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    use crate::completion::controller::detect_trigger;
    use crate::completion::ghost::ghost_suffix;

    // Build input line with optional ghost text suffix inserted at cursor position
    let input_line = if let Some(ref ghost) = app.completion_state.ghost_text {
        let query = detect_trigger(&app.input, app.cursor_pos).and_then(|t| match t {
            crate::completion::controller::CompletionTrigger::Command { query }
            | crate::completion::controller::CompletionTrigger::Agent { query, .. }
            | crate::completion::controller::CompletionTrigger::File { query } => Some(query),
            crate::completion::controller::CompletionTrigger::Bash { .. } => None,
        });

        // Only show ghost text when cursor is at the end of the current token
        let pos = app.cursor_pos.min(app.input.len());
        let at_token_end =
            pos == app.input.len() || app.input[pos..].starts_with(char::is_whitespace);

        if at_token_end {
            if let Some(suffix) = query.as_deref().and_then(|q| ghost_suffix(q, ghost)) {
                let before = &app.input[..pos];
                let after = &app.input[pos..];
                Line::from(vec![
                    Span::raw(before.to_string()),
                    Span::styled(suffix.to_string(), Style::default().fg(Color::DarkGray)),
                    Span::raw(after.to_string()),
                ])
            } else {
                Line::from(app.input.as_str())
            }
        } else {
            Line::from(app.input.as_str())
        }
    } else {
        Line::from(app.input.as_str())
    };

    let input = Paragraph::new(input_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.agent_busy {
                    "Waiting..."
                } else {
                    "Input"
                })
                .padding(Padding::horizontal(1)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, area);

    // 2 for borders + 2 for horizontal padding
    let inner_width = area.width.saturating_sub(4) as usize;
    let (cursor_row, cursor_col) = compute_wrapped_cursor(&app.input, app.cursor_pos, inner_width);
    // +2 for left border + left padding
    let cursor_x = area
        .x
        .saturating_add(2)
        .saturating_add(cursor_col)
        .min(area.x.saturating_add(area.width.saturating_sub(3)));
    // +1 for top border (no vertical padding on input)
    let cursor_y = area
        .y
        .saturating_add(1)
        .saturating_add(cursor_row)
        .min(area.y.saturating_add(area.height.saturating_sub(2)));
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn draw_status_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let dim = Style::default().fg(Color::DarkGray);

    // -- Left side: connection type · [running tool] · token usage --
    let mut left_spans: Vec<Span<'_>> = Vec::new();

    // Connection / status label
    left_spans.push(Span::styled(&app.status, dim));

    // Running tool (most recent Running entry, if any)
    let running_tool = app
        .tool_calls
        .iter()
        .rev()
        .find(|t| matches!(t.status, ToolCallStatus::Running));
    if let Some(entry) = running_tool {
        let spinner = SPINNER_CHARS[app.spinner_tick % SPINNER_CHARS.len()];
        let tool_text = if entry.description.is_empty() {
            format!(" \u{00b7} {spinner} {}", entry.tool_name)
        } else {
            format!(
                " \u{00b7} {spinner} {} {}",
                entry.tool_name, entry.description
            )
        };
        left_spans.push(Span::styled(tool_text, Style::default().fg(Color::Cyan)));
    }

    // Token usage
    if let Some(ref u) = app.token_usage {
        left_spans.push(Span::styled(
            format!(
                " \u{00b7} {}in/{}out \u{00b7} ${:.4}",
                u.input_tokens, u.output_tokens, u.cost_usd
            ),
            dim,
        ));
    }

    // -- Right side: contextual keybinding hints --
    let keys_hint =
        match app.mode {
            AppMode::PermissionPrompt => "Y:allow A:session Tab:edit N:deny X:deny+stop",
            AppMode::PermissionEditInput
            | AppMode::PermissionComment
            | AppMode::PermissionDenyMessage => "Enter:submit Esc:back",
            AppMode::UserQuestion => "Enter:submit Esc:cancel",
            AppMode::FingerprintVerification => {
                if app.pending_fingerprint.as_ref().is_some_and(
                    super::super::tui::fingerprint_panel::FingerprintPrompt::needs_action,
                ) {
                    "Y:accept N/Esc:reject"
                } else {
                    "Press any key to continue"
                }
            }
            AppMode::Normal | AppMode::SessionList => {
                if app.detail_panel.visible {
                    "Ctrl+Up/Down: tools | Ctrl+C: quit"
                } else {
                    "Ctrl+D: detail | Ctrl+C: quit"
                }
            }
        };

    // Compute widths to right-align keybinding hints
    let left_width: usize = left_spans.iter().map(Span::width).sum();
    let right_width = keys_hint.len();
    let bar_width = area.width as usize;
    let gap = bar_width.saturating_sub(left_width + right_width);

    left_spans.push(Span::raw(" ".repeat(gap)));
    left_spans.push(Span::styled(keys_hint, dim));

    let status = Paragraph::new(Line::from(left_spans));
    frame.render_widget(status, area);
}

/// Compute (row, col) for the cursor position in wrapped text.
///
/// Matches ratatui's `WordWrapper` with `trim: false`:
/// - Breaks at word boundaries (spaces) when a word would overflow the line
/// - Long words that exceed the line width break at the character boundary
/// - Leading whitespace is preserved on wrapped lines
#[allow(clippy::cast_possible_truncation)]
pub fn compute_wrapped_cursor(text: &str, cursor_pos: usize, max_width: usize) -> (u16, u16) {
    if max_width == 0 {
        return (0, 0);
    }
    let max_w = max_width as u16;

    // Compute the byte offset of each line break, matching ratatui's WordWrapper (trim=false).
    // line_breaks[i] = byte offset where line i+1 starts.
    let line_breaks = compute_word_wrap_breaks(text, max_w);

    // Find which line the cursor falls on.
    let mut row = 0u16;
    for &lb in &line_breaks {
        if cursor_pos >= lb {
            row += 1;
        } else {
            break;
        }
    }

    // Compute column: sum of display widths from line start to cursor_pos.
    let line_start = if row == 0 {
        0
    } else {
        line_breaks[(row - 1) as usize]
    };

    let mut col = 0u16;
    for (byte_idx, ch) in text[line_start..].char_indices() {
        if line_start + byte_idx >= cursor_pos {
            break;
        }
        col += UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]) as &str) as u16;
    }

    (row, col)
}

/// Compute byte offsets where each wrapped line starts (after line 0).
///
/// Replicates ratatui's `WordWrapper` (trim=false) line-breaking:
/// - Words are kept together; if a word overflows the line, it wraps to the next
/// - The space before a wrapped word is consumed (not rendered)
/// - Whitespace is preserved as leading indent when trim=false
/// - Long words exceeding the line width are broken at character boundaries
#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::expect_used
)]
fn compute_word_wrap_breaks(text: &str, max_width: u16) -> Vec<usize> {
    if text.is_empty() {
        return Vec::new();
    }

    // Split text into tokens: alternating whitespace and word segments.
    // Each token: (byte_start, byte_end, display_width, is_whitespace)
    let mut tokens: Vec<(usize, usize, u16, bool)> = Vec::new();
    let mut tok_start = 0;
    let mut tok_width = 0u16;
    let mut tok_is_ws: Option<bool> = None;

    for (bi, ch) in text.char_indices() {
        let is_ws = ch.is_whitespace();
        let ch_w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]) as &str) as u16;

        match tok_is_ws {
            None => {
                tok_start = bi;
                tok_width = ch_w;
                tok_is_ws = Some(is_ws);
            }
            Some(prev_ws) if prev_ws == is_ws => {
                tok_width += ch_w;
            }
            Some(_) => {
                tokens.push((
                    tok_start,
                    bi,
                    tok_width,
                    tok_is_ws.expect("tok_is_ws is Some in this match arm"),
                ));
                tok_start = bi;
                tok_width = ch_w;
                tok_is_ws = Some(is_ws);
            }
        }
    }
    if let Some(is_ws) = tok_is_ws {
        tokens.push((tok_start, text.len(), tok_width, is_ws));
    }

    // Now simulate word-wrapping using the tokens.
    let mut breaks = Vec::new();
    let mut line_width: u16 = 0;
    let mut i = 0;

    while i < tokens.len() {
        let (tok_start, tok_end, tok_w, is_ws) = tokens[i];

        if is_ws {
            // Whitespace token: peek at next word to decide if it fits
            let next_word = tokens.get(i + 1);
            if let Some(&(word_start, _word_end, word_w, false)) = next_word {
                let total = line_width + tok_w + word_w;
                if total > max_width {
                    if line_width == 0 {
                        // Empty line: whitespace + word overflow.
                        // With trim=false, whitespace is preserved.
                        // First try fitting ws, then break the word.
                        if tok_w + word_w > max_width {
                            // Put whitespace on this line, break word across lines
                            line_width += tok_w;
                            i += 1;
                            continue;
                        }
                    }
                    // Wrap: new line starts at the word (skip the space).
                    // But with trim=false, if there's multi-char whitespace,
                    // one space is consumed and rest is preserved.
                    // Actually looking at the output: "    " becomes "   " on line 2.
                    // The break happens after the first whitespace char.
                    if tok_w <= 1 {
                        // Single space: consumed entirely, word starts new line
                        breaks.push(word_start);
                    } else {
                        // Multi-space: first space consumed, rest preserved
                        // Find byte offset of second whitespace char
                        let mut ws_chars = text[tok_start..tok_end].char_indices();
                        ws_chars.next(); // skip first ws char
                        if let Some((offset, _)) = ws_chars.next() {
                            breaks.push(tok_start + offset);
                        } else {
                            breaks.push(word_start);
                        }
                    }
                    // Re-compute line_width for the new line from the break point
                    let bp = *breaks.last().expect("breaks is non-empty after push");
                    // The new line includes the remaining whitespace + the word
                    let remaining_ws: u16 = text[bp..word_start]
                        .chars()
                        .map(|c| UnicodeWidthStr::width(c.encode_utf8(&mut [0; 4]) as &str) as u16)
                        .sum();
                    if remaining_ws + word_w > max_width {
                        // Even with remaining ws, word overflows — handle below
                        line_width = remaining_ws;
                        i += 1;
                        continue;
                    }
                    line_width = remaining_ws + word_w;
                    i += 2; // consumed ws + word
                    continue;
                }
                // Fits: add ws + word
                line_width += tok_w + word_w;
                i += 2;
                continue;
            }
            // Trailing whitespace, just add it
            line_width += tok_w;
            i += 1;
            continue;
        }
        // Word token
        if line_width + tok_w > max_width {
            if line_width > 0 {
                // Word overflows current line: wrap
                breaks.push(tok_start);
            }
            // Word alone may exceed line width: break at character boundary
            if tok_w > max_width {
                let remaining = break_long_word(text, tok_start, tok_end, max_width, &mut breaks);
                line_width = remaining;
            } else {
                line_width = tok_w;
            }
        } else {
            line_width += tok_w;
        }
        i += 1;
    }

    breaks
}

/// Break a long word that exceeds `max_width` at character boundaries.
/// Returns the width of the last (partial) line produced.
#[allow(clippy::cast_possible_truncation)]
fn break_long_word(
    text: &str,
    start: usize,
    end: usize,
    max_width: u16,
    breaks: &mut Vec<usize>,
) -> u16 {
    let mut w = 0u16;
    for (bi, ch) in text[start..end].char_indices() {
        let ch_w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]) as &str) as u16;
        if w + ch_w > max_width {
            breaks.push(start + bi);
            w = ch_w;
        } else {
            w += ch_w;
        }
    }
    w
}

/// Braille spinner animation characters.
const SPINNER_CHARS: &[char] = &[
    '\u{280B}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283C}', '\u{2834}', '\u{2826}', '\u{2827}',
    '\u{2807}', '\u{280F}',
];

/// Format a tool call entry as a single status line with icon prefix.
///
/// - Running: `⠋ Read /src/main.rs` (spinner animates through `SPINNER_CHARS`)
/// - Done: `✓ Read 0.2s`
/// - Error: `✗ Bash 0.1s`
pub fn format_tool_status_line(entry: &ToolCallEntry, tick: usize) -> String {
    match entry.status {
        ToolCallStatus::Running => {
            let spinner = SPINNER_CHARS[tick % SPINNER_CHARS.len()];
            if entry.description.is_empty() {
                format!("{spinner} {}", entry.tool_name)
            } else {
                format!("{spinner} {} {}", entry.tool_name, entry.description)
            }
        }
        ToolCallStatus::Done => {
            let duration = format_duration_ms(entry.duration_ms);
            format!("\u{2713} {} {duration}", entry.tool_name)
        }
        ToolCallStatus::Error => {
            let duration = format_duration_ms(entry.duration_ms);
            format!("\u{2717} {} {duration}", entry.tool_name)
        }
    }
}

/// Format a duration in milliseconds as a human-readable string.
///
/// - `>= 1000ms` → `"1.2s"`
/// - `< 1000ms` → `"50ms"`
/// - `None` → `""`
pub fn format_duration_ms(ms: Option<u32>) -> String {
    match ms {
        Some(ms) if ms >= 1000 => format!("{:.1}s", f64::from(ms) / 1000.0),
        Some(ms) => format!("{ms}ms"),
        None => String::new(),
    }
}

/// Compute conversation/panel widths for the detail panel split.
/// Returns `(conversation_width, panel_width)`.
/// If terminal is too narrow (<80), returns `(0, full_width)` for overlay mode.
pub fn compute_detail_split(total_width: u16) -> (u16, u16) {
    if total_width < 80 {
        return (0, total_width);
    }
    let panel_width = (total_width * 2 / 5).max(30).min(total_width - 30);
    let conv_width = total_width - panel_width;
    (conv_width, panel_width)
}

/// Render the detail panel with full tool call information.
///
/// Delegates to [`detail_panel::draw_detail_panel`] which shows the selected
/// tool call's status, output, and scroll position.
fn draw_detail_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    detail_panel::draw_detail_panel(frame, app, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::tui::fingerprint_panel::FingerprintPrompt;
    use betcode_crypto::FingerprintCheck;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn draw_with_fingerprint_panel_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.mode = AppMode::FingerprintVerification;
        app.pending_fingerprint = Some(FingerprintPrompt::new(
            "test-machine",
            "aa:bb:cc:dd:ee:ff:11:22",
            FingerprintCheck::TrustOnFirstUse,
        ));
        terminal.draw(|f| draw(f, &mut app)).unwrap();
    }

    #[test]
    fn compute_wrapped_cursor_word_boundary() {
        // "hello world" with width 8:
        // Paragraph word-wraps as: "hello" (5) | "world" (5)
        // Cursor at end of "hello " (pos=6) should be on row 0, col 6
        // Cursor at "w" (pos=6..=6 for 'w') => row 1, col 0
        // Because "world" (5 chars) would need 1(space)+5=6 which fits 8,
        // let's use a tighter example.

        // Width 10: "abcde fghij" = 11 chars
        // Word wrap: line 1 = "abcde" (5), line 2 = "fghij" (5)
        // Because "abcde " = 6 + "fghij" = 5 => 11 > 10, so "fghij" wraps.
        // Cursor at pos 6 ('f') should be row 1, col 0
        let (row, col) = compute_wrapped_cursor("abcde fghij", 6, 10);
        assert_eq!((row, col), (1, 0), "cursor at 'f' should wrap to next line");

        // Cursor at pos 5 (the space) should still be on row 0
        let (row, col) = compute_wrapped_cursor("abcde fghij", 5, 10);
        assert_eq!(
            (row, col),
            (0, 5),
            "cursor at space should stay on first line"
        );

        // Cursor at end of "fghij" (pos 11) should be row 1, col 5
        let (row, col) = compute_wrapped_cursor("abcde fghij", 11, 10);
        assert_eq!((row, col), (1, 5), "cursor at end should be on second line");
    }

    #[test]
    fn compute_wrapped_cursor_preserves_whitespace_no_trim() {
        // Width 20: "AAAAAAAAAAAAAAAAAAAA    AAA" (20 A's + 4 spaces + 3 A's)
        // Word wrap (trim=false): ["AAAAAAAAAAAAAAAAAAAA", "   AAA"]
        // (ratatui preserves leading whitespace on wrapped line)
        // Cursor at pos 24 (first 'A' of "AAA") should be row 1, col 3
        let (row, col) = compute_wrapped_cursor("AAAAAAAAAAAAAAAAAAAA    AAA", 24, 20);
        assert_eq!(
            (row, col),
            (1, 3),
            "cursor should account for preserved whitespace on wrapped line"
        );
    }

    #[test]
    fn compute_wrapped_cursor_long_word_breaks_at_width() {
        // A word longer than the line width should be broken at the width boundary.
        // Width 10, text = "abcdefghijklmno" (15 chars, no spaces)
        // Word wrap: ["abcdefghij", "klmno"]
        // Cursor at pos 10 ('k') should be row 1, col 0
        let (row, col) = compute_wrapped_cursor("abcdefghijklmno", 10, 10);
        assert_eq!((row, col), (1, 0), "long word wraps at width boundary");
    }

    #[test]
    fn compute_wrapped_cursor_sentence_matches_paragraph() {
        // Test with a realistic sentence similar to what the user typed.
        // Width 40 (inner width of ~42 col terminal minus borders):
        // "I am testing this project right now for commands and"
        // This should word-wrap correctly.
        let text = "abcdefghij klmnopqrst uvwxyz abcdefghijk";
        // Width 20: words are "abcdefghij"(10), "klmnopqrst"(10), "uvwxyz"(6), "abcdefghijk"(11)
        // Line 1: "abcdefghij" (10) — "klmnopqrst" won't fit (10+1+10=21>20)
        // Line 2: "klmnopqrst" (10) — "uvwxyz" won't fit (10+1+6=17<=20? yes!)
        // Actually: "klmnopqrst uvwxyz" = 17 fits in 20.
        // Line 2: "klmnopqrst uvwxyz" (17)
        // Line 3: "abcdefghijk" (11)
        // Cursor at pos 11 ('k' of klmnopqrst) should be row 1, col 0
        let (row, col) = compute_wrapped_cursor(text, 11, 20);
        assert_eq!((row, col), (1, 0), "second word wraps to new line");

        // Cursor at pos 29 ('a' of last word) should be row 2, col 0
        // (pos 28 is the consumed space between "uvwxyz" and "abcdefghijk")
        let (row, col) = compute_wrapped_cursor(text, 29, 20);
        assert_eq!((row, col), (2, 0), "last word wraps to third line");
    }

    #[test]
    fn tool_result_messages_hidden_when_detail_panel_open() {
        let messages = vec![
            DisplayMessage {
                role: MessageRole::Assistant,
                content: "Looking at it".into(),
                streaming: false,
                agent_label: None,
                is_tool_result: false,
            },
            DisplayMessage {
                role: MessageRole::Tool,
                content: "[Tool: Read]".into(),
                streaming: false,
                agent_label: None,
                is_tool_result: false,
            },
            DisplayMessage {
                role: MessageRole::Tool,
                content: "[Tool Result (OK): ...]".into(),
                streaming: false,
                agent_label: None,
                is_tool_result: true,
            },
            DisplayMessage {
                role: MessageRole::Assistant,
                content: "Here's what I found".into(),
                streaming: false,
                agent_label: None,
                is_tool_result: false,
            },
        ];

        // When detail panel is open, tool result messages are hidden
        let visible: Vec<_> = filter_visible_messages(&messages, true).collect();
        assert_eq!(visible.len(), 3, "result message hidden when panel open");
        // Verify the hidden message is the tool result (index 2)
        assert!(
            visible.iter().all(|(_, m)| !m.is_tool_result),
            "no tool result messages in filtered output"
        );

        // When detail panel is closed, all messages are shown
        assert_eq!(
            filter_visible_messages(&messages, false).count(),
            4,
            "all shown when panel closed"
        );
    }

    #[test]
    fn filter_visible_messages_preserves_original_indices() {
        let messages = vec![
            DisplayMessage {
                role: MessageRole::Assistant,
                content: "msg0".into(),
                streaming: false,
                agent_label: None,
                is_tool_result: false,
            },
            DisplayMessage {
                role: MessageRole::Tool,
                content: "result".into(),
                streaming: false,
                agent_label: None,
                is_tool_result: true,
            },
            DisplayMessage {
                role: MessageRole::Assistant,
                content: "msg2".into(),
                streaming: false,
                agent_label: None,
                is_tool_result: false,
            },
        ];

        let visible: Vec<_> = filter_visible_messages(&messages, true).collect();
        assert_eq!(visible.len(), 2);
        // Original indices must be preserved for tool call lookup
        assert_eq!(visible[0].0, 0, "first visible message keeps index 0");
        assert_eq!(visible[1].0, 2, "second visible message keeps index 2");
    }
}
