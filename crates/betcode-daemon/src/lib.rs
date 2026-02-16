//! `BetCode` Daemon Library
//!
//! Core functionality for the `BetCode` daemon:
//! - Subprocess management for Claude Code processes
//! - `SQLite` storage for sessions and messages
//! - gRPC server for client connections
//! - Session multiplexing for multi-client support
//! - Permission bridge for tool authorization

pub mod commands;
pub mod completion;
pub mod gitlab;
pub mod permission;
pub mod plugin;
pub mod relay;
pub mod server;
pub mod session;
pub mod storage;
pub mod subprocess;
pub mod tunnel;
pub mod worktree;

/// Test infrastructure shared between unit and integration tests.
///
/// Not part of the public API -- hidden from docs and only useful for testing.
#[doc(hidden)]
#[allow(clippy::unwrap_used)]
pub mod testutil {
    use std::sync::Arc;

    use tokio::sync::RwLock;

    use crate::commands::CommandRegistry;
    use crate::relay::SessionRelay;
    use crate::session::SessionMultiplexer;
    use crate::storage::Database;
    use crate::subprocess::SubprocessManager;

    /// Core test components backed by an in-memory database.
    pub struct TestComponents {
        pub db: Database,
        pub relay: Arc<SessionRelay>,
        pub multiplexer: Arc<SessionMultiplexer>,
    }

    /// Build a `TestComponents` with an in-memory DB, capped subprocess manager,
    /// and default multiplexer.
    ///
    /// # Panics
    ///
    /// Panics if the in-memory database cannot be opened.
    pub async fn test_components() -> TestComponents {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let command_registry = Arc::new(RwLock::new(CommandRegistry::new()));
        let relay = Arc::new(SessionRelay::new(
            subprocess_mgr,
            Arc::clone(&multiplexer),
            db.clone(),
            command_registry,
        ));
        TestComponents {
            db,
            relay,
            multiplexer,
        }
    }
}
