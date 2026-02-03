//! NDJSON parser for Claude Code stream-json protocol.
//!
//! This module parses newline-delimited JSON from Claude's stdout into
//! canonical message types, implementing a tolerant reader pattern.

mod parser;
mod types;

pub use parser::{parse_line, parse_value};
pub use types::*;
