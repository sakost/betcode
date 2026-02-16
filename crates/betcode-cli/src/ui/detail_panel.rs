//! Detail panel rendering for selected tool call.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, ToolCallEntry, ToolCallStatus};

use super::render::format_duration_ms;

/// Build the title string for the detail panel block.
///
/// - If the entry has a non-empty description: `"Read (/src/main.rs)"`
/// - If the description is empty: `"Read"`
pub fn panel_title(entry: &ToolCallEntry) -> String {
    if entry.description.is_empty() {
        entry.tool_name.clone()
    } else {
        format!("{} ({})", entry.tool_name, entry.description)
    }
}

/// Format a status line with icon, status text, and optional duration.
///
/// - Running: `"• Running..."`
/// - Done:    `"✓ Done (1.2s)"`
/// - Error:   `"✗ Error (50ms)"`
pub fn format_status_line(status: ToolCallStatus, duration_ms: Option<u32>) -> String {
    match status {
        ToolCallStatus::Running => "\u{2022} Running...".to_string(),
        ToolCallStatus::Done => {
            let d = format_duration_ms(duration_ms);
            format!("\u{2713} Done ({d})")
        }
        ToolCallStatus::Error => {
            let d = format_duration_ms(duration_ms);
            format!("\u{2717} Error ({d})")
        }
    }
}

/// Render the detail panel showing the selected tool call's full information.
///
/// If no tool call is selected, displays a placeholder message with navigation hints.
/// If a compaction summary is available and no tool is selected, shows the summary.
/// Otherwise shows the tool name/description as the block title, a colored status
/// line, a separator, and the tool output (or a "Waiting..." placeholder).
pub fn draw_detail_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let entry = app
        .detail_panel
        .selected_tool_index
        .and_then(|i| app.tool_calls.get(i));

    let Some(entry) = entry else {
        // Show compaction summary if available and no tool call is selected
        if let Some(ref summary) = app.compaction_summary {
            let mut lines: Vec<Line<'_>> = vec![
                Line::from(Span::styled(
                    "Context was compacted. Summary:",
                    Style::default().fg(Color::Yellow),
                )),
                Line::from("\u{2500}".repeat(area.width.saturating_sub(2) as usize)),
            ];
            for line in summary.lines() {
                let owned: String = line.to_owned();
                lines.push(Line::from(owned));
            }
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Compaction Summary")
                .title_style(Style::default().fg(Color::Yellow));
            let scroll = app.detail_panel.scroll_offset;
            let para = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0));
            frame.render_widget(para, area);
            return;
        }

        let block = Block::default().borders(Borders::ALL).title("Detail");
        let para = Paragraph::new("No tool call selected.\nUse Ctrl+Up/Down to navigate.")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(para, area);
        return;
    };

    let title = panel_title(entry);
    let status_color = match entry.status {
        ToolCallStatus::Running => Color::Cyan,
        ToolCallStatus::Done => Color::Green,
        ToolCallStatus::Error => Color::Red,
    };

    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            format_status_line(entry.status, entry.duration_ms),
            Style::default().fg(status_color),
        )),
        Line::from("\u{2500}".repeat(area.width.saturating_sub(2) as usize)),
    ];

    if let Some(ref output) = entry.output {
        for line in output.lines() {
            let owned: String = line.to_owned();
            lines.push(Line::from(owned));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Waiting for output...",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(Color::Yellow));

    let scroll = app.detail_panel.scroll_offset;
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(para, area);
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation
)]
mod tests {
    use super::*;
    use crate::app::ToolCallStatus;
    use crate::ui::test_helpers::make_tool_entry;

    #[test]
    fn panel_title_includes_tool_name_and_description() {
        let entry = make_tool_entry(
            "Read",
            "/src/main.rs",
            ToolCallStatus::Done,
            Some(1200),
            Some("file contents"),
        );
        let title = panel_title(&entry);
        assert_eq!(title, "Read (/src/main.rs)");
    }

    #[test]
    fn panel_title_no_description() {
        let entry = make_tool_entry("Read", "", ToolCallStatus::Running, None, None);
        let title = panel_title(&entry);
        assert_eq!(title, "Read");
    }

    #[test]
    fn status_line_formatting() {
        assert_eq!(
            format_status_line(ToolCallStatus::Done, Some(1200)),
            "\u{2713} Done (1.2s)"
        );
        assert_eq!(
            format_status_line(ToolCallStatus::Error, Some(50)),
            "\u{2717} Error (50ms)"
        );
        assert_eq!(
            format_status_line(ToolCallStatus::Running, None),
            "\u{2022} Running..."
        );
    }
}
