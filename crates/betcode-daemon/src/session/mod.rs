//! Session management and multiplexing.
//!
//! Handles multiple client connections to a single session with event fan-out.

mod multiplexer;
mod state;
mod types;

pub use multiplexer::SessionMultiplexer;
pub use types::{
    ClientHandle, InputLockResult, MultiplexerConfig, MultiplexerError, MultiplexerStats,
};
