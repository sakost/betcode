//! Completion trigger detection.
//!
//! Analyzes the current input text and cursor position to determine
//! which type of completion (if any) should be activated.

/// The type of completion trigger detected from user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Slash command: `/help`, `/reload-commands`
    Command { query: String },
    /// Agent mention: `@researcher`, `@@forced-agent`
    Agent { query: String },
    /// File reference: `@/src/main.rs`, `@./file`, `@README.md`
    File { query: String },
    /// Bash shortcut: `!ls -la`
    Bash { cmd: String },
}

/// Detect a completion trigger from the input text at the given cursor position.
///
/// Extracts the token at/before the cursor and classifies it:
/// - `/...` -> Command
/// - `@@...` -> Agent (forced)
/// - `@/...` or `@./...` or `@../...` -> File (forced)
/// - `@text` with path chars (contains `/` or `.ext` pattern) -> File
/// - `@text` without path chars -> Agent
/// - `!...` -> Bash
/// - Otherwise -> None
pub fn detect_trigger(input: &str, cursor_pos: usize) -> Option<CompletionTrigger> {
    let pos = cursor_pos.min(input.len());
    let before_cursor = &input[..pos];

    // Check for bang command first: `!` at the start of input captures everything after it.
    // This is special because bash commands contain spaces.
    let trimmed = before_cursor.trim_start();
    if let Some(cmd) = trimmed.strip_prefix('!') {
        return Some(CompletionTrigger::Bash {
            cmd: cmd.to_string(),
        });
    }

    // Find the start of the current token by scanning backwards to whitespace
    let token_start = before_cursor
        .rfind(char::is_whitespace)
        .map(|i| {
            i + before_cursor[i..]
                .chars()
                .next()
                .expect("rfind returned a valid char index")
                .len_utf8()
        })
        .unwrap_or(0);

    let token = &before_cursor[token_start..];
    if token.is_empty() {
        return None;
    }

    if let Some(query) = token.strip_prefix('/') {
        return Some(CompletionTrigger::Command {
            query: query.to_string(),
        });
    }

    if let Some(after_at) = token.strip_prefix('@') {
        // `@@...` -> forced Agent
        if let Some(rest) = after_at.strip_prefix('@') {
            return Some(CompletionTrigger::Agent {
                query: rest.to_string(),
            });
        }

        // `@/...` or `@./...` or `@../...` -> forced File
        if after_at.starts_with('/') || after_at.starts_with("./") || after_at.starts_with("../") {
            return Some(CompletionTrigger::File {
                query: after_at.to_string(),
            });
        }

        // Disambiguation: path-like text -> File, otherwise -> Agent
        if looks_like_path(after_at) {
            return Some(CompletionTrigger::File {
                query: after_at.to_string(),
            });
        }

        return Some(CompletionTrigger::Agent {
            query: after_at.to_string(),
        });
    }

    None
}

/// Check whether a text string looks like a filesystem path.
///
/// Returns true if the text:
/// - contains a `/` separator
/// - matches a `*.ext` pattern (contains a `.` followed by letters)
/// - starts with `./` or `../`
pub fn looks_like_path(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    if text.starts_with("./") || text.starts_with("../") {
        return true;
    }

    if text.contains('/') {
        return true;
    }

    // Check for file extension pattern: something.ext where ext is alphanumeric
    if let Some(dot_pos) = text.rfind('.') {
        let ext = &text[dot_pos + 1..];
        if !ext.is_empty() && ext.chars().all(|c| c.is_alphanumeric()) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_trigger_slash() {
        let trigger = detect_trigger("/hel", 4);
        assert_eq!(
            trigger,
            Some(CompletionTrigger::Command {
                query: "hel".to_string()
            })
        );
    }

    #[test]
    fn test_detect_trigger_at_agent() {
        let trigger = detect_trigger("@res", 4);
        assert_eq!(
            trigger,
            Some(CompletionTrigger::Agent {
                query: "res".to_string()
            })
        );
    }

    #[test]
    fn test_detect_trigger_at_file_explicit() {
        let trigger = detect_trigger("@/src/main", 10);
        assert_eq!(
            trigger,
            Some(CompletionTrigger::File {
                query: "/src/main".to_string()
            })
        );
    }

    #[test]
    fn test_detect_trigger_at_file_implicit() {
        let trigger = detect_trigger("@README.md", 10);
        assert_eq!(
            trigger,
            Some(CompletionTrigger::File {
                query: "README.md".to_string()
            })
        );
    }

    #[test]
    fn test_detect_trigger_at_force_agent() {
        let trigger = detect_trigger("@@res", 5);
        assert_eq!(
            trigger,
            Some(CompletionTrigger::Agent {
                query: "res".to_string()
            })
        );
    }

    #[test]
    fn test_detect_trigger_bang() {
        let trigger = detect_trigger("!ls -la", 7);
        assert_eq!(
            trigger,
            Some(CompletionTrigger::Bash {
                cmd: "ls -la".to_string()
            })
        );
    }

    #[test]
    fn test_detect_trigger_none() {
        let trigger = detect_trigger("hello world", 11);
        assert_eq!(trigger, None);
    }

    #[test]
    fn test_at_path_detection() {
        assert!(looks_like_path("src/main.rs"));
        assert!(looks_like_path("README.md"));
        assert!(looks_like_path("./file"));
        assert!(looks_like_path("../file"));
        assert!(!looks_like_path("researcher"));
        assert!(!looks_like_path("team-lead"));
    }
}
