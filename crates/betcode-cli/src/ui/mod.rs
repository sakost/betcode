//! TUI rendering components.

pub mod detail_panel;
mod panels;
mod render;
#[cfg(test)]
mod render_tests;
pub mod status_panel;

pub use render::{draw, format_duration_ms, format_tool_status_line};

#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::app::{ToolCallEntry, ToolCallStatus};

    pub fn make_tool_entry(
        name: &str,
        desc: &str,
        status: ToolCallStatus,
        duration_ms: Option<u32>,
        output: Option<&str>,
    ) -> ToolCallEntry {
        ToolCallEntry {
            tool_id: "t1".to_string(),
            tool_name: name.to_string(),
            description: desc.to_string(),
            input_json: None,
            output: output.map(String::from),
            status,
            duration_ms,
            finished_at: None,
            message_index: 0,
        }
    }
}
