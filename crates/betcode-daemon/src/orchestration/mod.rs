//! Subagent orchestration: subprocess pool, manager, and DAG scheduler.
//!
//! This module provides the core infrastructure for spawning and managing
//! Claude Code subagent subprocesses under a parent session, including:
//!
//! - [`SubprocessPool`]: Semaphore-based concurrency limiter for subagent processes.
//! - [`SubagentManager`]: High-level lifecycle manager that spawns, monitors,
//!   times out, and cancels subagent subprocesses.
//! - [`DagScheduler`]: DAG-based step scheduler for multi-step orchestrations.

pub mod manager;
pub mod pool;
pub mod scheduler;

pub use manager::SubagentManager;
pub use pool::SubprocessPool;
pub use scheduler::DagScheduler;
