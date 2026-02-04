//! Relay module: bridges subprocess I/O with gRPC event streams.
//!
//! This is the "glue" that connects:
//! - SubprocessManager (spawn, stdin/stdout channels)
//! - EventBridge (NDJSON → AgentEvent conversion)
//! - SessionMultiplexer (multi-client event broadcast)

mod pipeline;
mod types;

pub use pipeline::SessionRelay;
pub use types::*;
