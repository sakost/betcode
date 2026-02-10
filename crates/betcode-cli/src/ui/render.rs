//! TUI rendering functions.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
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
    Fingerprint,
}

fn bottom_panel_mode(app: &App) -> BottomPanel {
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
    draw_messages(frame, app, chunks[1]);

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
        let popup_height = (app.completion_state.items.len() as u16).min(8) + 2; // +2 for borders
        let popup_area = Rect {
            x: area.x + 1,
            y: area.height.saturating_sub(bottom_height + popup_height + 1),
            width: area.width.saturating_sub(2).min(60),
            height: popup_height,
        };

        let items: Vec<Line> = app
            .completion_state
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let style = if i == app.completion_state.selected_index {
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
        use crate::ui::status_panel::{render_status_panel, SessionStatusInfo};
        let info = SessionStatusInfo {
            cwd: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            session_id: app.session_id.clone().unwrap_or_else(|| "none".to_string()),
            connection: "local".to_string(),
            model: app.model.clone(),
            active_agents: 0,
            pending_permissions: if app.pending_permission.is_some() {
                1
            } else {
                0
            },
            worktree: None,
            uptime_secs: 0,
        };
        render_status_panel(frame, frame.area(), &info);
    }
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

    for (i, msg) in app.messages.iter().enumerate() {
        // Add empty line separator between messages (not before the first)
        if i > 0 {
            lines.push(Line::from(""));
        }
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
    let inner_width = area.width.saturating_sub(2);

    // Use ratatui's own word-wrap line count so scroll range exactly matches
    // what the Paragraph widget actually renders.
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = paragraph.line_count(inner_width) as u16;

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

    let messages = paragraph
        .block(Block::default().borders(Borders::ALL).title(title))
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
        AppMode::FingerprintVerification => {
            if app
                .pending_fingerprint
                .as_ref()
                .is_some_and(|fp| fp.needs_action())
            {
                " | Y:accept N/Esc:reject"
            } else {
                " | Press any key to continue"
            }
        }
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
///
/// Matches ratatui's `WordWrapper` with `trim: false`:
/// - Breaks at word boundaries (spaces) when a word would overflow the line
/// - Long words that exceed the line width break at the character boundary
/// - Leading whitespace is preserved on wrapped lines
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
                tokens.push((tok_start, bi, tok_width, tok_is_ws.unwrap()));
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
                    let bp = *breaks.last().unwrap();
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
                } else {
                    // Fits: add ws + word
                    line_width += tok_w + word_w;
                    i += 2;
                    continue;
                }
            } else {
                // Trailing whitespace, just add it
                line_width += tok_w;
                i += 1;
                continue;
            }
        } else {
            // Word token
            if line_width + tok_w > max_width {
                if line_width > 0 {
                    // Word overflows current line: wrap
                    breaks.push(tok_start);
                }
                // Word alone may exceed line width: break at character boundary
                if tok_w > max_width {
                    let remaining =
                        break_long_word(text, tok_start, tok_end, max_width, &mut breaks);
                    line_width = remaining;
                } else {
                    line_width = tok_w;
                }
            } else {
                line_width += tok_w;
            }
            i += 1;
        }
    }

    breaks
}

/// Break a long word that exceeds max_width at character boundaries.
/// Returns the width of the last (partial) line produced.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::tui::fingerprint_panel::FingerprintPrompt;
    use betcode_crypto::FingerprintCheck;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
}
