# Non-Functional Requirements & SLOs

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Status**: Design Phase

## Overview

This document defines measurable non-functional requirements for BetCode: latency SLOs,
throughput limits, resource constraints, availability targets, and message ordering
guarantees. These requirements drive architectural decisions and provide acceptance
criteria for performance validation.

**Philosophy**: You cannot improve what you cannot measure. Every target in this document
must be instrumentable via the observability stack defined in [DAEMON.md](./DAEMON.md).

Related: [CAPACITY_PLANNING.md](./CAPACITY_PLANNING.md) for detailed scaling formulas.

---

## Latency SLOs

### Latency Budget Breakdown

```
User Input -> First Response Token (excluding AI inference)

+----------------+     +----------------+     +----------------+     +----------------+
|   Client UI    | --> |     Relay      | --> |     Daemon     | --> |  Claude Code   |
|   Processing   |     |    Routing     |     |   Processing   |     |   Subprocess   |
+----------------+     +----------------+     +----------------+     +----------------+
      P50: 5ms              P50: 3ms              P50: 10ms             P50: 50ms
      P95: 15ms             P95: 8ms              P95: 25ms             P95: 100ms
      P99: 30ms             P99: 15ms             P99: 50ms             P99: 200ms
```

### Per-Operation Latency Targets

| Operation | P50 | P95 | P99 | Notes |
|-----------|-----|-----|-----|-------|
| **Client Layer** |||||
| UI input processing | 5ms | 15ms | 30ms | Keyboard-to-gRPC serialization |
| UI render (TUI/Flutter) | 8ms | 16ms | 33ms | Target 60fps, 30fps minimum |
| Offline queue write | 2ms | 5ms | 10ms | Local SQLite insert |
| **Relay Layer** |||||
| JWT validation | 0.5ms | 1ms | 2ms | Signature + revocation check |
| Tunnel routing | 1ms | 3ms | 5ms | Registry lookup + frame forward |
| Message buffer write | 2ms | 5ms | 10ms | SQLite insert for offline daemon |
| **Daemon Layer** |||||
| Permission engine eval | 1ms | 3ms | 8ms | Rule matching + DB lookup |
| Session lookup | 0.5ms | 1ms | 2ms | In-memory + SQLite fallback |
| SQLite message insert | 2ms | 5ms | 10ms | WAL mode, single writer |
| gRPC event broadcast | 1ms | 3ms | 5ms | Fan-out to N clients |
| **Subprocess Layer** |||||
| Claude spawn (cold) | 500ms | 800ms | 1200ms | Node.js + npm startup |
| Claude spawn (warm) | 50ms | 100ms | 200ms | Process pool pre-warmed |

### End-to-End Latency Targets

| Path | P50 | P95 | P99 | Notes |
|------|-----|-----|-----|-------|
| Local CLI -> Daemon -> First Token | 60ms | 150ms | 300ms | Excludes AI inference |
| Flutter -> Relay -> Daemon -> First Token | 110ms | 200ms | 400ms | Includes network RTT |
| Permission prompt -> User sees dialog | 80ms | 150ms | 300ms | Local path |
| Reconnection -> Stream resume | 200ms | 500ms | 1000ms | History replay capped |

---

## Throughput Limits

### Per-Component Throughput

| Component | Metric | Target | Burst |
|-----------|--------|--------|-------|
| **Daemon** ||||
| Concurrent sessions | 20 | 50 | Per-machine default |
| Concurrent subagents | 5 | 10 | Subprocess pool size |
| Connected clients | 100 | 200 | gRPC streams |
| Messages/second (ingest) | 1000 | 2000 | NDJSON lines from all sessions |
| Events/second (broadcast) | 5000 | 10000 | gRPC events to all clients |
| **Relay** ||||
| Concurrent tunnels | 10000 | 20000 | Per-relay instance |
| Concurrent clients | 50000 | 100000 | Per-relay instance |
| Requests/second | 100000 | 200000 | Excludes payload processing |

### Rate Limits

| Endpoint | Limit | Window | Scope |
|----------|-------|--------|-------|
| `Converse` (new session) | 20 | 1 hour | per user |
| `UserMessage` | 60 | 1 minute | per session |
| `SpawnSubagent` | 50 | 1 hour | per parent session |
| Token refresh | 30 | 1 minute | per user |
| Registration/login | 10 | 1 minute | per IP |

---

## Resource Constraints

### Daemon Resource Limits

| Resource | Default | Max | Config Path |
|----------|---------|-----|-------------|
| Memory (daemon) | 512 MB | 2 GB | `daemon.memory_limit_mb` |
| Disk (SQLite) | 1 GB | 10 GB | `daemon.max_db_size_gb` |
| Disk (per session) | 50 MB | 200 MB | `daemon.max_session_size_mb` |
| Network connections | 200 | 1000 | `daemon.max_connections` |

### Relay Resource Limits

| Resource | Default | Max | Notes |
|----------|---------|-----|-------|
| Memory | 1 GB | 8 GB | Scales with connection count |
| Disk (SQLite) | 10 GB | 100 GB | Message buffer + auth DB |
| Network connections | 100000 | 500000 | Tune with ulimit |

---

## Availability Targets

### Service Level Objectives

| Component | Target | Measurement |
|-----------|--------|-------------|
| Daemon (local) | 99.9% | Process uptime |
| Relay | 99.95% | Health endpoint success |
| End-to-end | 99.5% | Request completion rate |

### Failure Recovery Times

| Failure | Target Recovery |
|---------|-----------------|
| Claude subprocess crash | 5 seconds (auto-restart) |
| Daemon crash | 10 seconds (systemd restart) |
| Relay crash | 30 seconds (LB failover) |
| Network partition | 60 seconds (reconnect backoff) |

---

## Message Ordering Guarantees

### Within-Session: Total Order

Every `AgentEvent` within a session carries a monotonic `sequence` number:
- Assigned at write time by daemon (single writer)
- Unique within session (SQLite UNIQUE constraint)
- Clients process in sequence order; buffer out-of-order events

```
Session S1: seq=1 -> seq=2 -> seq=3 -> seq=4 (strict ordering)
```

### Cross-Session: No Guarantee

Sessions are independent. Events from S1 and S2 may interleave arbitrarily.
Each session's events are individually ordered, but no global order exists.

### Permission Responses: First Wins, Idempotent Duplicates

When multiple responses arrive for the same `request_id`:
1. First valid response: process normally, write to Claude stdin
2. Subsequent responses: log duplicate, return success (idempotent)
3. Unknown request_id: log warning, return error
4. Timeout (60s): auto-deny, remove from pending map

```
T0: Claude emits control_request(req_001)
T1: Client A responds ALLOW - processed
T2: Tunnel drops, client reconnects
T3: Daemon replays pending request (is_replay=true)
T4: Client A responds ALLOW again - ignored (idempotent)
```

### Tunnel Frames: FIFO Within Stream

Frames within a tunnel stream are FIFO ordered. Multiple logical requests
multiplex on the tunnel; responses correlate via `request_id`, not order.

---

## Message Buffer TTL Trade-offs

### Current: 24-Hour Fixed TTL

Chosen to cover overnight disconnects for typical developer workflows.

### Argument: TTL Should Be Configurable

**The 24-hour TTL is arbitrary**:
- Too long: 20-hour-old messages may reference stale context
- Too short: Weekend disconnects exceed 24 hours
- No user control: Different users have different staleness tolerance

### Proposed: Configurable TTL (1 hour to 7 days)

| TTL | Storage Impact | Staleness Risk | Use Case |
|-----|----------------|----------------|----------|
| 1 hour | Minimal | Low | Real-time only workflows |
| 24 hours | Moderate | Medium | Overnight (default) |
| 72 hours | High | Medium-High | Weekend coverage |
| 168 hours | Very High | High | Vacation, critical workflows |

**Recommendation**: Configurable TTL with 24-hour default. Add staleness
warning in UI when delivering messages older than 4 hours.

---

## Monitoring Thresholds

### SLO-Based Alerts

| Metric | Warning | Critical |
|--------|---------|----------|
| Latency P95 | > 150ms | > 200ms |
| Latency P99 | > 400ms | > 500ms |
| Error rate | > 0.5% | > 1% |
| Availability | < 99.7% | < 99.5% |

### Resource-Based Alerts

| Metric | Warning | Critical |
|--------|---------|----------|
| CPU usage | > 70% (5 min) | > 90% (2 min) |
| Memory usage | > 70% | > 85% |
| Disk usage | > 70% | > 90% |
| Connections | > 80% limit | > 95% limit |

---

## Related Documents

| Document | Description |
|----------|-------------|
| [CAPACITY_PLANNING.md](./CAPACITY_PLANNING.md) | Scaling formulas, resource estimation |
| [DAEMON.md](./DAEMON.md) | Observability metrics |
| [TOPOLOGY.md](./TOPOLOGY.md) | Relay architecture |
| [PROTOCOL_L2.md](./PROTOCOL_L2.md) | Sequence numbers, gRPC API |
