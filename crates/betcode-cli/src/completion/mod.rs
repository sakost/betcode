//! Completion system for the CLI input.
//!
//! Provides trigger detection, ghost text preview, and popup widget
//! for inline autocompletion of commands, agents, files, and bash shortcuts.

pub mod controller;
pub mod ghost;
pub mod popup;
