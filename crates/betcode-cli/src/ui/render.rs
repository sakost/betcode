//! TUI rendering functions.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::panels;
use crate::app::{App, AppMode, MessageRole};

/// Which panel to show at the bottom.
enum BottomPanel {
    Input,
    Permission,
    PermissionEdit,
    Question,
}

fn bottom_panel_mode(app: &App) -> BottomPanel {
    match app.mode {
        AppMode::PermissionPrompt => BottomPanel::Permission,
        AppMode::PermissionEditInput
        | AppMode::PermissionComment
        | AppMode::PermissionDenyMessage => BottomPanel::PermissionEdit,
        AppMode::UserQuestion => BottomPanel::Question,
        _ => BottomPanel::Input,
    }
}

/// Draw the full UI.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let bottom_panel = bottom_panel_mode(app);

    let frame_width = frame.area().width;
    let inner_input_width = frame_width.saturating_sub(2) as usize;
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
            let opt_count = app
                .pending_question
                .as_ref()
                .map(|q| q.options.len())
                .unwrap_or(0);
            let lines = 2 + opt_count as u16 + 2;
            (lines + 2).min(frame.area().height / 2).max(6)
        }
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
    draw_messages(frame, app, chunks[1]);

    match bottom_panel {
        BottomPanel::Input => draw_input(frame, app, chunks[2]),
        BottomPanel::Permission => panels::draw_permission_panel(frame, app, chunks[2]),
        BottomPanel::PermissionEdit => panels::draw_permission_edit_panel(frame, app, chunks[2]),
        BottomPanel::Question => panels::draw_question_panel(frame, app, chunks[2]),
    }

    draw_status_bar(frame, app, chunks[3]);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
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

fn draw_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        let (prefix, color) = match msg.role {
            MessageRole::User => ("You: ", Color::Green),
            MessageRole::Assistant => ("Claude: ", Color::Blue),
            MessageRole::System => ("System: ", Color::Yellow),
            MessageRole::Tool => ("", Color::DarkGray),
        };
        let prefix_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let content_lines: Vec<&str> = msg.content.split('\n').collect();

        if content_lines.is_empty() || (content_lines.len() == 1 && content_lines[0].is_empty()) {
            let cursor = if msg.streaming { "\u{2588}" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(cursor, Style::default().fg(Color::White)),
            ]));
        } else {
            let cursor = if msg.streaming && content_lines.len() == 1 {
                "\u{2588}"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::raw(content_lines[0]),
                Span::styled(cursor, Style::default().fg(Color::White)),
            ]));
            let indent = " ".repeat(prefix.len());
            for (i, content_line) in content_lines.iter().enumerate().skip(1) {
                let cursor = if msg.streaming && i == content_lines.len() - 1 {
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

    let inner_height = area.height.saturating_sub(2);
    let inner_width = area.width.saturating_sub(2) as usize;

    let total: u16 = lines
        .iter()
        .map(|line| {
            if inner_width == 0 {
                return 1u16;
            }
            let display_width: usize = line
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            1u16.max(
                display_width
                    .saturating_add(inner_width - 1)
                    .checked_div(inner_width)
                    .unwrap_or(1) as u16,
            )
        })
        .sum();

    app.viewport_height = inner_height;
    app.total_lines = total;

    let max_scroll = total.saturating_sub(inner_height);
    let scroll = if app.scroll_pinned {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_offset)
    };

    let title = if !app.scroll_pinned {
        let position = max_scroll.saturating_sub(scroll);
        format!(
            "Conversation [scroll: {}/{}]",
            position.min(max_scroll),
            max_scroll
        )
    } else {
        "Conversation".to_string()
    };

    let messages = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(messages, area);
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    let input = Paragraph::new(app.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.agent_busy {
                    "Waiting..."
                } else {
                    "Input"
                }),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, area);

    let inner_width = area.width.saturating_sub(2) as usize;
    let (cursor_row, cursor_col) = compute_wrapped_cursor(&app.input, app.cursor_pos, inner_width);
    let cursor_x = area
        .x
        .saturating_add(1)
        .saturating_add(cursor_col)
        .min(area.x.saturating_add(area.width.saturating_sub(2)));
    let cursor_y = area
        .y
        .saturating_add(1)
        .saturating_add(cursor_row)
        .min(area.y.saturating_add(area.height.saturating_sub(2)));
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let usage = app.token_usage.as_ref().map_or(String::new(), |u| {
        format!(
            " | Tokens: {}in/{}out | ${:.4}",
            u.input_tokens, u.output_tokens, u.cost_usd
        )
    });
    let keys_hint = match app.mode {
        AppMode::PermissionPrompt => " | Y:allow A:session Tab:edit N:deny X:deny+stop",
        AppMode::PermissionEditInput
        | AppMode::PermissionComment
        | AppMode::PermissionDenyMessage => " | Enter:submit Esc:back",
        AppMode::UserQuestion => " | Enter:submit Esc:cancel",
        _ => " | Ctrl+C: quit | Enter: send",
    };

    let status = Paragraph::new(Line::from(vec![
        Span::styled(&app.status, Style::default().fg(Color::DarkGray)),
        Span::styled(usage, Style::default().fg(Color::DarkGray)),
        Span::styled(keys_hint, Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(status, area);
}

/// Compute (row, col) for the cursor position in wrapped text.
pub fn compute_wrapped_cursor(text: &str, cursor_pos: usize, inner_width: usize) -> (u16, u16) {
    if inner_width == 0 {
        return (0, 0);
    }
    let mut row = 0u16;
    let mut col = 0u16;
    for (byte_idx, ch) in text.char_indices() {
        if byte_idx >= cursor_pos {
            break;
        }
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]) as &str) as u16;
        if col + ch_width > inner_width as u16 {
            row += 1;
            col = ch_width;
        } else {
            col += ch_width;
        }
    }
    (row, col)
}
