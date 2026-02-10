//! Ghost text rendering for inline completion preview.
//!
//! Provides functions to compute the ghost text suffix (the dimmed
//! continuation of a completion) and to build styled spans for display.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Extract the untyped suffix of a completion.
///
/// If `completion` starts with `typed` (case-insensitive), returns the remaining
/// suffix. Returns `None` if they are an exact match or if `completion` does not
/// start with `typed`.
pub fn ghost_suffix<'a>(typed: &str, completion: &'a str) -> Option<&'a str> {
    if typed.is_empty() && !completion.is_empty() {
        return Some(completion);
    }

    if typed.len() >= completion.len() {
        return None;
    }

    let prefix = &completion[..typed.len()];
    if prefix.eq_ignore_ascii_case(typed) {
        Some(&completion[typed.len()..])
    } else {
        None
    }
}

/// Build styled spans for the input line with an optional ghost text suffix.
///
/// Returns:
/// - If no completion or exact match: a single span with the typed text
/// - Otherwise: the typed text span (normal style) followed by the suffix span (DarkGray)
pub fn ghost_text_spans<'a>(typed: &'a str, completion: Option<&str>) -> Vec<Span<'a>> {
    let suffix = completion.and_then(|c| ghost_suffix(typed, c));

    match suffix {
        Some(s) => vec![
            Span::raw(typed),
            Span::styled(s.to_string(), Style::default().fg(Color::DarkGray)),
        ],
        None => vec![Span::raw(typed)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn test_ghost_spans_basic() {
        let spans = ghost_text_spans("hel", Some("help"));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "p");
        assert_eq!(spans[1].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_ghost_spans_no_completion() {
        let spans = ghost_text_spans("hello", None);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn test_ghost_spans_exact_match() {
        let spans = ghost_text_spans("help", Some("help"));
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn test_ghost_completion_extraction() {
        assert_eq!(ghost_suffix("hel", "help"), Some("p"));
        assert_eq!(ghost_suffix("/cd", "/cd"), None);
        assert_eq!(
            ghost_suffix("/re", "/reload-commands"),
            Some("load-commands")
        );
    }
}
