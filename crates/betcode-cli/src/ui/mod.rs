//! TUI rendering components.

mod panels;
mod render;
#[cfg(test)]
mod render_tests;

pub use render::draw;
