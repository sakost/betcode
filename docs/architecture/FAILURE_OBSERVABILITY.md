# Observability Requirements

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Parent**: [FAILURE_MODES.md](./FAILURE_MODES.md)

---

## Distributed Tracing

**Every request must be traceable end-to-end.**

```
Client Request (trace_id: abc123)
    |
    v
Relay: span "relay.route"
    |
    v
Daemon: span "daemon.handle_request"
    |
    +-- span "subprocess.spawn"
    +-- span "sqlite.insert"
    +-- span "grpc.broadcast"
    |
    v
Response
```

Trace ID propagation:
1. Client generates trace ID (or relay generates for incoming requests)
2. Trace ID included in gRPC metadata
3. Daemon includes trace ID in subprocess environment
4. All log entries include trace ID

---

## Required Metrics

### Daemon Metrics

| Metric | Type | Labels | Purpose |
|--------|------|--------|---------|
| `betcode_subprocess_spawn_duration_seconds` | Histogram | `outcome` | Startup latency |
| `betcode_subprocess_crashes_total` | Counter | `session_id`, `exit_code` | Crash tracking |
| `betcode_ndjson_parse_errors_total` | Counter | `error_type` | Protocol health |
| `betcode_permission_request_duration_seconds` | Histogram | `tool_name`, `decision` | Permission latency |
| `betcode_permission_timeout_total` | Counter | `tool_name` | Timeout frequency |
| `betcode_sqlite_operation_duration_seconds` | Histogram | `operation` | Database latency |
| `betcode_sqlite_errors_total` | Counter | `error_type` | Database health |
| `betcode_circuit_breaker_state` | Gauge | `breaker` | Circuit status |
| `betcode_circuit_breaker_rejections_total` | Counter | `breaker` | Rejected calls |

### Relay Metrics

| Metric | Type | Labels | Purpose |
|--------|------|--------|---------|
| `betcode_relay_tunnel_reconnects_total` | Counter | `machine_id`, `reason` | Tunnel stability |
| `betcode_relay_tunnel_latency_seconds` | Histogram | `machine_id` | Tunnel health |
| `betcode_relay_buffer_overflow_total` | Counter | `machine_id` | Buffer capacity |
| `betcode_relay_auth_failures_total` | Counter | `reason` | Security monitoring |

---

## Alert Thresholds

| Alert | Condition | Severity | Action |
|-------|-----------|----------|--------|
| High crash rate | >5 crashes in 5 min | P2 | Page on-call |
| SQLite errors | Any SQLITE_CORRUPT | P1 | Page immediately |
| API rate limited | Circuit breaker open | P3 | Notify team |
| Tunnel disconnected | >5 min offline | P3 | Notify user |
| Permission timeout rate | >20% of requests | P3 | Review timeout config |
| Parse error spike | >1% of messages | P2 | Investigate protocol |

---

## Log Aggregation Format

All logs must be structured JSON with consistent fields:

```json
{
  "timestamp": "2026-02-03T10:15:30.123Z",
  "level": "ERROR",
  "target": "betcode_daemon::subprocess::protocol",
  "message": "NDJSON parse error",
  "trace_id": "abc123def456",
  "session_id": "sess_01HQ5X...",
  "error_type": "invalid_json",
  "raw_line_preview": "{\"type\":\"assi..."
}
```

### Required Fields

- `timestamp` (ISO 8601 with milliseconds)
- `level` (TRACE, DEBUG, INFO, WARN, ERROR)
- `target` (module path)
- `message` (human-readable)

### Contextual Fields

- `trace_id` (distributed trace correlation)
- `session_id` (session context)
- `machine_id` (multi-machine context)
- `request_id` (request correlation)
- `error_type` (categorized error)
- `duration_ms` (for latency tracking)

---

## Chaos Engineering Recommendations

### Experiment 1: SQLite Corruption

**Method**: Stop daemon, corrupt database file, restart.
**Expected**: Detects corruption, restores from backup, logs event.

### Experiment 2: Subprocess Crash Storm

**Method**: Configure mock claude to exit non-zero repeatedly.
**Expected**: After 5 crashes in 60s, stops restarting, sends CRASHED status.

### Experiment 3: Network Partition

**Method**: Firewall mobile client, queue messages, restore.
**Expected**: All queued messages delivered in order, no duplicates.

### Experiment 4: Rate Limit Cascade

**Method**: Start 10 sessions, mock rate limit errors.
**Expected**: Circuit breaker trips, no thundering herd on recovery.

### Experiment 5: Permission Timeout Under Load

**Method**: Trigger permission, do not respond, verify other sessions work.
**Expected**: Other sessions unaffected, timeout after configured TTL.
