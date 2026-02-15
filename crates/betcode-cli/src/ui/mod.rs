//! TUI rendering components.

pub mod detail_panel;
mod panels;
mod render;
#[cfg(test)]
mod render_tests;
pub mod status_panel;

pub use render::{draw, format_duration_ms, format_tool_status_line};
