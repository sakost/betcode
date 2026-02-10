//! Floating overlay popup widget for completion items.
//!
//! Provides the data model and navigation logic for the completion popup,
//! including virtualized scrolling for large item lists.

/// Category of a completion item, used for styling and grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionCategory {
    Command,
    Agent,
    File,
    Plugin,
}

/// A single completion item displayed in the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    /// The completion text to insert.
    pub text: String,
    /// A short description shown alongside the text.
    pub description: String,
    /// The category for styling/grouping.
    pub category: CompletionCategory,
    /// Badge indicating the source (e.g. `[bc]`, `[cc]`).
    pub source_badge: String,
    /// Byte positions in `text` that matched the fuzzy query.
    pub match_positions: Vec<usize>,
}

impl CompletionItem {
    /// Convenience constructor with empty source_badge and match_positions.
    pub fn simple(text: &str, description: &str, category: CompletionCategory) -> Self {
        Self {
            text: text.to_string(),
            description: description.to_string(),
            category,
            source_badge: String::new(),
            match_positions: Vec::new(),
        }
    }
}

/// State for the completion popup, managing selection and scroll window.
#[derive(Debug, Clone)]
pub struct PopupState {
    items: Vec<CompletionItem>,
    selected: usize,
    visible_count: usize,
    scroll_offset: usize,
}

impl PopupState {
    /// Create a new popup state with the given items and visible window size.
    pub fn new(items: Vec<CompletionItem>, visible_count: usize) -> Self {
        Self {
            items,
            selected: 0,
            visible_count,
            scroll_offset: 0,
        }
    }

    /// The index of the currently selected item.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Move the selection up by one, wrapping to the end.
    pub fn move_up(&mut self) {
        if self.items.is_empty() {
            return;
        }
        if self.selected == 0 {
            self.selected = self.items.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.adjust_scroll();
    }

    /// Move the selection down by one, wrapping to the start.
    pub fn move_down(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.items.len();
        self.adjust_scroll();
    }

    /// Return a slice of items currently visible in the scroll window.
    pub fn visible_items(&self) -> &[CompletionItem] {
        let end = (self.scroll_offset + self.visible_count).min(self.items.len());
        &self.items[self.scroll_offset..end]
    }

    /// Accept the currently selected item, returning a clone of it.
    pub fn accept(&self) -> CompletionItem {
        self.items[self.selected].clone()
    }

    /// Adjust the scroll offset so the selected item is visible.
    fn adjust_scroll(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.visible_count {
            self.scroll_offset = self.selected + 1 - self.visible_count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_item_display() {
        let item = CompletionItem {
            text: "reload-commands".to_string(),
            description: "Re-discover all commands".to_string(),
            category: CompletionCategory::Command,
            source_badge: "[bc]".to_string(),
            match_positions: vec![0, 7],
        };
        assert_eq!(item.text, "reload-commands");
    }

    #[test]
    fn test_popup_state_navigation() {
        let items = vec![
            CompletionItem::simple("cd", "Change dir", CompletionCategory::Command),
            CompletionItem::simple("pwd", "Print dir", CompletionCategory::Command),
            CompletionItem::simple("exit", "Exit CLI", CompletionCategory::Command),
        ];
        let mut state = PopupState::new(items, 8);
        assert_eq!(state.selected_index(), 0);
        state.move_down();
        assert_eq!(state.selected_index(), 1);
        state.move_down();
        assert_eq!(state.selected_index(), 2);
        state.move_down(); // wrap around
        assert_eq!(state.selected_index(), 0);
        state.move_up(); // wrap to end
        assert_eq!(state.selected_index(), 2);
    }

    #[test]
    fn test_popup_visible_window() {
        let items: Vec<CompletionItem> = (0..20)
            .map(|i| CompletionItem::simple(&format!("item-{}", i), "", CompletionCategory::Command))
            .collect();
        let state = PopupState::new(items, 5);
        let visible = state.visible_items();
        assert_eq!(visible.len(), 5);
    }

    #[test]
    fn test_popup_accept_returns_selected() {
        let items = vec![
            CompletionItem::simple("cd", "", CompletionCategory::Command),
            CompletionItem::simple("pwd", "", CompletionCategory::Command),
        ];
        let mut state = PopupState::new(items, 8);
        state.move_down();
        let accepted = state.accept();
        assert_eq!(accepted.text, "pwd");
    }
}
