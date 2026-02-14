//! Bottom-panel rendering for permission prompts, user questions, and fingerprint verification.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::render::compute_wrapped_cursor;
use crate::app::{App, AppMode};
use crate::tui::fingerprint_panel::FingerprintPrompt;

/// Render a bordered panel with the given title, border color, and lines of content.
fn render_bordered_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    border_color: Color,
    lines: Vec<Line<'_>>,
) {
    let panel = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().fg(border_color)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}

/// Draw the permission action panel (Y/A/Tab/N/X) replacing the input area.
#[allow(clippy::too_many_lines)]
pub fn draw_permission_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(ref perm) = app.pending_permission else {
        return;
    };

    let mut lines = vec![
        Line::from(vec![Span::styled(
            format!("Tool: {}", perm.tool_name),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(perm.description.as_str()),
        Line::from(""),
    ];

    if let Some(ref input_val) = perm.original_input {
        let preview = serde_json::to_string(input_val).unwrap_or_default();
        let truncated = if preview.len() > 60 {
            format!("{}...", &preview[..57])
        } else {
            preview
        };
        lines.push(Line::from(Span::styled(
            truncated,
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(vec![
        Span::styled(
            "[Y/1]",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Allow  "),
        Span::styled(
            "[A/2]",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Session  "),
        Span::styled(
            "[Tab/3]",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Edit  "),
        Span::styled(
            "[E/4]",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Comment"),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[N/5]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Deny  "),
        Span::styled(
            "[X/6]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Deny+Stop  "),
        Span::styled(
            "[Esc]",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Cancel"),
    ]));

    let panel = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Permission Required")
                .style(Style::default().fg(Color::White)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(panel, area);
}

/// Draw the permission edit/comment/deny text input panel.
pub fn draw_permission_edit_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(ref perm) = app.pending_permission else {
        return;
    };

    let title = match app.mode {
        AppMode::PermissionEditInput => "Edit Tool Input (Enter=submit, Esc=back)",
        AppMode::PermissionComment => "Comment (Enter=allow+send, Esc=back)",
        AppMode::PermissionDenyMessage => {
            if perm.deny_interrupt {
                "Deny Message (Enter=deny+interrupt, Esc=back)"
            } else {
                "Deny Message (Enter=deny, Esc=back)"
            }
        }
        _ => "Edit",
    };

    let panel = Paragraph::new(perm.edit_buffer.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().fg(Color::White)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(panel, area);

    let inner_width = area.width.saturating_sub(2) as usize;
    let (cursor_row, cursor_col) =
        compute_wrapped_cursor(&perm.edit_buffer, perm.edit_cursor, inner_width);

    let cursor_x = area.x.saturating_add(1).saturating_add(cursor_col);
    let cursor_y = area.y.saturating_add(1).saturating_add(cursor_row);
    let cursor_x = cursor_x.min(area.x.saturating_add(area.width.saturating_sub(2)));
    let cursor_y = cursor_y.min(area.y.saturating_add(area.height.saturating_sub(2)));
    frame.set_cursor_position((cursor_x, cursor_y));
}

/// Draw the user question panel with selectable options.
pub fn draw_question_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(ref q) = app.pending_question else {
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            &q.question,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, opt) in q.options.iter().enumerate() {
        let is_highlighted = i == q.highlight;
        let is_selected = q.selected.contains(&i);

        let marker = if is_selected { "[x]" } else { "[ ]" };

        let style = if is_highlighted {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else if is_selected {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let mut spans = vec![
            Span::styled(marker, style),
            Span::styled(format!(" {}. ", i + 1), style),
            Span::styled(opt.label.clone(), style),
        ];

        if !opt.description.is_empty() {
            spans.push(Span::styled(
                format!(" - {}", opt.description),
                if is_highlighted {
                    style
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ));
        }

        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));

    let select_hint = if q.multi_select { "Space=toggle, " } else { "" };
    lines.push(Line::from(Span::styled(
        format!(
            "{}Up/Down=navigate, 1-{}=select, Enter=submit, Esc=cancel",
            select_hint,
            q.options.len().min(9)
        ),
        Style::default().fg(Color::DarkGray),
    )));

    let title = if q.multi_select {
        "Question (multi-select)"
    } else {
        "Question"
    };

    render_bordered_panel(frame, area, title, Color::White, lines);
}

/// Draw the fingerprint verification panel.
///
/// Shows the daemon's fingerprint with randomart visualization.
/// For TOFU, this is informational. For mismatches, requires user action.
#[allow(clippy::too_many_lines)]
pub fn draw_fingerprint_panel(frame: &mut Frame<'_>, prompt: &FingerprintPrompt, area: Rect) {
    let header_color = if prompt.needs_action() {
        Color::Red
    } else {
        Color::Green
    };

    let mut lines = vec![
        Line::from(Span::styled(
            prompt.header_text(),
            Style::default()
                .fg(header_color)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Machine: {}", prompt.machine_id),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];

    // Add randomart lines
    for line in prompt.randomart.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(""));

    // Add fingerprint hex display
    lines.push(Line::from(Span::styled(
        "Fingerprint:",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    for line in prompt.fingerprint_display.lines() {
        lines.push(Line::from(Span::styled(
            format!("  {line}"),
            Style::default().fg(Color::White),
        )));
    }

    // Show expected fingerprint for mismatches
    if let betcode_crypto::FingerprintCheck::Mismatch { ref expected, .. } = prompt.check {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Expected: {}", &expected[..expected.len().min(50)]),
            Style::default().fg(Color::Red),
        )));
    }

    lines.push(Line::from(""));

    if prompt.needs_action() {
        lines.push(Line::from(vec![
            Span::styled(
                "[Y]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Accept new key  "),
            Span::styled(
                "[N/Esc]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Reject & disconnect"),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "Press any key to continue...",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let (title, border_color) = if prompt.needs_action() {
        ("FINGERPRINT MISMATCH", Color::Red)
    } else {
        ("Daemon Fingerprint", Color::White)
    };

    render_bordered_panel(frame, area, title, border_color, lines);
}
