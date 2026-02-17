//! Semaphore-based subprocess pool for subagent concurrency control.
//!
//! The [`SubprocessPool`] limits the number of concurrent subagent processes
//! to avoid overwhelming the system.  It issues permits via a Tokio semaphore
//! and tracks active subprocess handles so they can be enumerated or cancelled.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore, mpsc};
use tracing::{debug, info};

/// Default maximum number of concurrent subagent subprocesses.
const DEFAULT_MAX_CONCURRENCY: usize = 5;

/// A permit granting the right to run one subagent subprocess.
///
/// When dropped, the permit is automatically returned to the pool.
pub struct PoolPermit {
    _permit: OwnedSemaphorePermit,
}

/// Metadata for a tracked subprocess in the pool.
#[derive(Debug, Clone)]
pub struct PoolEntry {
    /// The subagent ID that owns this slot.
    pub subagent_id: String,
    /// Channel to write to the subprocess stdin.
    pub stdin_tx: mpsc::Sender<String>,
}

/// Semaphore-based concurrency pool for subagent subprocesses.
pub struct SubprocessPool {
    semaphore: Arc<Semaphore>,
    max_concurrency: usize,
    /// Active entries keyed by subagent ID.
    entries: Arc<RwLock<HashMap<String, PoolEntry>>>,
}

impl SubprocessPool {
    /// Create a new pool with the given concurrency limit.
    pub fn new(max_concurrency: usize) -> Self {
        let limit = if max_concurrency == 0 {
            DEFAULT_MAX_CONCURRENCY
        } else {
            max_concurrency
        };

        info!(max_concurrency = limit, "SubprocessPool created");

        Self {
            semaphore: Arc::new(Semaphore::new(limit)),
            max_concurrency: limit,
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Acquire a permit to spawn a subagent subprocess.
    ///
    /// Returns `None` if a permit cannot be acquired immediately (pool full).
    pub fn try_acquire(&self) -> Option<PoolPermit> {
        let permit = Arc::clone(&self.semaphore).try_acquire_owned().ok()?;
        Some(PoolPermit { _permit: permit })
    }

    /// Acquire a permit, waiting until one becomes available.
    pub async fn acquire(&self) -> Result<PoolPermit, PoolError> {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| PoolError::Closed)?;
        Ok(PoolPermit { _permit: permit })
    }

    /// Register a subagent entry in the pool.
    pub async fn register(&self, entry: PoolEntry) {
        let id = entry.subagent_id.clone();
        debug!(subagent_id = %id, "Registering subagent in pool");
        self.entries.write().await.insert(id, entry);
    }

    /// Remove a subagent entry from the pool.
    pub async fn unregister(&self, subagent_id: &str) -> Option<PoolEntry> {
        debug!(subagent_id, "Unregistering subagent from pool");
        self.entries.write().await.remove(subagent_id)
    }

    /// Get a clone of the pool entry for the given subagent.
    pub async fn get(&self, subagent_id: &str) -> Option<PoolEntry> {
        self.entries.read().await.get(subagent_id).cloned()
    }

    /// Return a list of all active subagent IDs in the pool.
    pub async fn active_ids(&self) -> Vec<String> {
        self.entries.read().await.keys().cloned().collect()
    }

    /// Number of currently active subagents registered in the pool.
    pub async fn active_count(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Maximum concurrency limit.
    pub const fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Number of available permits (slots) remaining.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

impl Default for SubprocessPool {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENCY)
    }
}

/// Errors from the subprocess pool.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// The semaphore was closed (pool shut down).
    #[error("Subprocess pool has been closed")]
    Closed,
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pool_default_concurrency() {
        let pool = SubprocessPool::default();
        assert_eq!(pool.max_concurrency(), DEFAULT_MAX_CONCURRENCY);
        assert_eq!(pool.available_permits(), DEFAULT_MAX_CONCURRENCY);
    }

    #[tokio::test]
    async fn pool_custom_concurrency() {
        let pool = SubprocessPool::new(3);
        assert_eq!(pool.max_concurrency(), 3);
        assert_eq!(pool.available_permits(), 3);
    }

    #[tokio::test]
    async fn pool_zero_uses_default() {
        let pool = SubprocessPool::new(0);
        assert_eq!(pool.max_concurrency(), DEFAULT_MAX_CONCURRENCY);
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)]
    async fn try_acquire_succeeds_when_available() {
        let pool = SubprocessPool::new(2);
        let permit1 = pool.try_acquire();
        assert!(permit1.is_some());
        assert_eq!(pool.available_permits(), 1);

        let permit2 = pool.try_acquire();
        assert!(permit2.is_some());
        assert_eq!(pool.available_permits(), 0);

        // Third should fail
        let permit3 = pool.try_acquire();
        assert!(permit3.is_none());
        drop((permit1, permit2, permit3));
    }

    #[tokio::test]
    async fn permit_returned_on_drop() {
        let pool = SubprocessPool::new(1);

        {
            let _permit = pool.try_acquire().unwrap();
            assert_eq!(pool.available_permits(), 0);
        }
        // Permit dropped
        assert_eq!(pool.available_permits(), 1);
    }

    #[tokio::test]
    async fn acquire_waits_for_permit() {
        let pool = Arc::new(SubprocessPool::new(1));

        let permit = pool.try_acquire().unwrap();
        assert_eq!(pool.available_permits(), 0);

        let pool_clone = Arc::clone(&pool);
        let handle = tokio::spawn(async move {
            // This should wait until the permit is released
            pool_clone.acquire().await.unwrap();
        });

        // Drop permit to unblock
        drop(permit);

        // The spawned task should complete
        tokio::time::timeout(std::time::Duration::from_millis(100), handle)
            .await
            .expect("acquire should complete after permit released")
            .unwrap();
    }

    #[tokio::test]
    async fn register_and_unregister() {
        let pool = SubprocessPool::new(5);
        let (tx, _rx) = mpsc::channel(1);

        pool.register(PoolEntry {
            subagent_id: "sa-1".to_string(),
            stdin_tx: tx.clone(),
        })
        .await;

        assert_eq!(pool.active_count().await, 1);
        assert!(pool.get("sa-1").await.is_some());

        let entry = pool.unregister("sa-1").await;
        assert!(entry.is_some());
        assert_eq!(pool.active_count().await, 0);
    }

    #[tokio::test]
    async fn active_ids_lists_registered() {
        let pool = SubprocessPool::new(5);
        let (tx, _rx) = mpsc::channel(1);

        pool.register(PoolEntry {
            subagent_id: "sa-a".to_string(),
            stdin_tx: tx.clone(),
        })
        .await;
        pool.register(PoolEntry {
            subagent_id: "sa-b".to_string(),
            stdin_tx: tx,
        })
        .await;

        let mut ids = pool.active_ids().await;
        ids.sort();
        assert_eq!(ids, vec!["sa-a", "sa-b"]);
    }

    #[tokio::test]
    async fn unregister_nonexistent_returns_none() {
        let pool = SubprocessPool::new(5);
        assert!(pool.unregister("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let pool = SubprocessPool::new(5);
        assert!(pool.get("nonexistent").await.is_none());
    }
}
