# Failure Modes and Recovery Specifications

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

---

## Table of Contents

- [Design Philosophy](#design-philosophy)
- [Failure Classification](#failure-classification)
- [Component Specifications](#component-specifications)

For detailed specifications, see:
- [FAILURE_SQLITE.md](./FAILURE_SQLITE.md) -- SQLite corruption handling
- [FAILURE_NDJSON.md](./FAILURE_NDJSON.md) -- NDJSON parse failure recovery
- [FAILURE_RATELIMIT.md](./FAILURE_RATELIMIT.md) -- API rate limit cascading
- [FAILURE_CIRCUITS.md](./FAILURE_CIRCUITS.md) -- Circuit breakers and bulkheads
- [FAILURE_PERMISSIONS.md](./FAILURE_PERMISSIONS.md) -- Permission timeout redesign
- [FAILURE_OBSERVABILITY.md](./FAILURE_OBSERVABILITY.md) -- Tracing, metrics, alerts

---

## Design Philosophy

**Everything fails. Plan for it.**

BetCode is a distributed system with multiple failure domains: local SQLite databases,
subprocess communication, network tunnels, external APIs, and user devices. Each component
will fail. The question is not *if* but *when* and *how gracefully*.

### Core Principles

1. **Fail Fast, Recover Faster**: Detect failures immediately. Do not mask them with
   retries that hide systemic issues. Surface failures to users with actionable context.

2. **Circuit Breakers Everywhere**: Stop cascading failures before they propagate. A
   failure in the Anthropic API should not take down session management.

3. **Bulkheads for Isolation**: Isolate failure domains. Each session is a bulkhead.
   One crashed subprocess does not affect others.

4. **Timeouts Must Be Reasonable**: The current 60-second permission timeout is
   **unacceptable** for mobile users. See [FAILURE_PERMISSIONS.md](./FAILURE_PERMISSIONS.md).

5. **Observability is Not Optional**: If you cannot measure it, you cannot manage it.

---

## Failure Classification

Every failure in BetCode falls into one of four categories:

### Transient Failures

**Characteristics**: Temporary, self-healing, retry-safe.

| Failure | Example | Recovery |
|---------|---------|----------|
| Network glitch | TCP reset mid-stream | Exponential backoff, resume |
| API rate limit | 429 from Anthropic | Backoff per retry-after |
| Resource contention | SQLite busy timeout | Retry with jitter |

**Strategy**: Retry with exponential backoff and jitter. Cap retries. Surface after threshold.

### Persistent Failures

**Characteristics**: Stable failure state, will not self-heal, requires intervention.

| Failure | Example | Recovery |
|---------|---------|----------|
| Invalid credentials | Expired API key | Prompt user to update |
| Missing dependency | Claude CLI not installed | Installation guidance |
| Configuration error | Invalid JSON in settings | Validation error with fix |

**Strategy**: Fail fast, provide actionable error message, do not retry automatically.

### Corruption Failures

**Characteristics**: Data integrity compromised, recovery may lose state.

| Failure | Example | Recovery |
|---------|---------|----------|
| SQLite corruption | WAL file damaged | Backup restore or rebuild |
| NDJSON malformation | Partial JSON from crash | Skip line, log, continue |

**Strategy**: Detect via integrity checks, attempt repair, fall back to known-good state.

### Cascading Failures

**Characteristics**: One failure triggers others, amplification risk.

| Failure | Example | Recovery |
|---------|---------|----------|
| API overload | Rate limit causes retry storm | Circuit breaker |
| Connection thundering herd | Relay restart causes reconnect storm | Jittered reconnection |

**Strategy**: Circuit breakers, bulkheads, load shedding, graceful degradation.

---

## Component Specifications

Detailed failure handling for each component is documented separately:

| Document | Scope |
|----------|-------|
| [FAILURE_SQLITE.md](./FAILURE_SQLITE.md) | Database corruption detection, recovery, backups |
| [FAILURE_NDJSON.md](./FAILURE_NDJSON.md) | Protocol parse errors, unknown types, sequence gaps |
| [FAILURE_RATELIMIT.md](./FAILURE_RATELIMIT.md) | Anthropic API rate limits, cascade prevention |
| [FAILURE_CIRCUITS.md](./FAILURE_CIRCUITS.md) | Circuit breaker patterns, bulkhead isolation |
| [FAILURE_PERMISSIONS.md](./FAILURE_PERMISSIONS.md) | 7-day TTL proposal, activity refresh |
| [FAILURE_OBSERVABILITY.md](./FAILURE_OBSERVABILITY.md) | Distributed tracing, metrics, alerts |

---

## References

- [Release It!](https://pragprog.com/titles/mnee2/release-it-second-edition/) by Michael Nygard
- [DAEMON.md](./DAEMON.md) -- Daemon architecture
- [PROTOCOL_L1.md](./PROTOCOL_L1.md) -- NDJSON protocol
- [SCHEMAS.md](./SCHEMAS.md) -- SQLite schemas
- [TOPOLOGY.md](./TOPOLOGY.md) -- Network architecture
