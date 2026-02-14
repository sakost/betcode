# Circuit Breakers and Bulkhead Isolation

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [FAILURE_MODES.md](./FAILURE_MODES.md)

---

## Circuit Breaker State Machine

```
     +---------+
     |         |
     v         | Success / Failure below threshold
  CLOSED ------+
     |
     | Failure threshold exceeded
     v
   OPEN -------> (Timeout elapsed) -------> HALF-OPEN
     ^                                          |
     |                                          |
     +---- Probe fails <------------------------+
                                                |
                              Probe succeeds ---+---> CLOSED
```

---

## Circuit Breakers by Dependency

| Dependency | Failure Threshold | Timeout | Probe Strategy |
|------------|-------------------|---------|----------------|
| Anthropic API | 3 rate limits in 60s | 60s (or retry-after) | Single session probe |
| Relay Tunnel | 5 reconnect failures | 30s doubling, max 5m | Reconnect attempt |
| SQLite | 3 write failures | 10s | Health check query |
| Claude Subprocess | 5 crashes in 60s | 30s | Spawn with --version |

---

## Implementation Pattern

```rust
pub enum CircuitState {
    Closed,
    Open { until: Instant },
    HalfOpen,
}

impl CircuitBreaker {
    pub async fn call<F, T, E>(&self, f: F) -> Result<T, CircuitError<E>>
    where F: FnOnce() -> Future<Output = Result<T, E>>
    {
        match self.state() {
            CircuitState::Open { until } if Instant::now() < until => {
                return Err(CircuitError::Open { remaining: until - Instant::now() });
            }
            CircuitState::Open { .. } => {
                self.set_state(CircuitState::HalfOpen);
            }
            _ => {}
        }

        match f().await {
            Ok(v) => { self.record_success(); Ok(v) }
            Err(e) => { self.record_failure(); Err(CircuitError::Inner(e)) }
        }
    }
}
```

---

## Bulkhead Isolation

Each session is an isolated failure domain:

```
+------------------+  +------------------+  +------------------+
|   Session 1      |  |   Session 2      |  |   Session 3      |
|   Bulkhead       |  |   Bulkhead       |  |   Bulkhead       |
|                  |  |                  |  |                  |
| [Subprocess]     |  | [Subprocess]     |  | [Subprocess]     |
|                  |  |                  |  |                  |
| Memory: 512MB    |  | Memory: 512MB    |  | Memory: 512MB    |
| Timeout: 10min   |  | Timeout: 10min   |  | Timeout: 10min   |
|                  |  |                  |  |                  |
| State: ACTIVE    |  | State: CRASHED   |  | State: IDLE      |
+------------------+  +------------------+  +------------------+

Session 2 crash does NOT affect Sessions 1 or 3
```

---

## Resource Limits per Bulkhead

| Resource | Per-Session Limit | Daemon Total | Enforcement |
|----------|-------------------|--------------|-------------|
| Memory | 512 MB | 80% of system | Process limits |
| File descriptors | 256 | 4096 | ulimit |
| Subprocess timeout | 10 minutes | N/A | Kill after inactivity |
| gRPC clients | 10 | 100 | Connection limit |

---

## Subprocess Pool

```rust
struct SubprocessPool {
    max_size: usize,
    running: HashMap<SessionId, Subprocess>,
    waiting: VecDeque<WaitingSession>,
    semaphore: Arc<Semaphore>,
}

impl SubprocessPool {
    pub async fn spawn(&self, session: &Session) -> Result<Subprocess, PoolError> {
        // Acquire slot with 30s timeout
        let permit = tokio::time::timeout(
            Duration::from_secs(30),
            self.semaphore.clone().acquire_owned()
        ).await.map_err(|_| PoolError::Timeout)?;

        let subprocess = Subprocess::spawn(session, SubprocessLimits {
            memory_mb: 512,
            timeout: Duration::from_secs(600),
        })?;

        subprocess.attach_permit(permit);
        Ok(subprocess)
    }
}
```
