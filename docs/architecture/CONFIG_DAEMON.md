# Daemon Configuration Reference

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [CONFIG_REFERENCE.md](./CONFIG_REFERENCE.md)

All daemon settings live in `$BETCODE_CONFIG_DIR/settings.json` under the `daemon` object.

---

## Core Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `daemon.max_payload_bytes` | integer | 10485760 | 1048576 | 104857600 | `BETCODE_MAX_PAYLOAD_BYTES` |
| `daemon.subprocess_timeout_seconds` | integer | 300 | 60 | 7200 | `BETCODE_SUBPROCESS_TIMEOUT` |
| `daemon.max_sessions` | integer | 100 | 1 | 1000 | `BETCODE_MAX_SESSIONS` |
| `daemon.log_level` | string | "info" | - | - | `BETCODE_LOG_LEVEL` |
| `daemon.socket_path` | string | (platform) | - | - | `BETCODE_SOCKET_PATH` |

**Default socket paths**:
- Linux/macOS: `/run/user/$UID/betcode/daemon.sock`
- Windows: `\\.\pipe\betcode-daemon-$USERNAME`

---

## Subprocess Pool Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `daemon.subprocess.max_concurrent` | integer | 5 | 1 | 20 | `BETCODE_MAX_SUBPROCESSES` |
| `daemon.subprocess.queue_size` | integer | 50 | 0 | 200 | `BETCODE_SUBPROCESS_QUEUE` |

When `queue_size` is 0, requests are rejected immediately when pool is full.

---

## Crash Recovery Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `daemon.crash_recovery.initial_backoff_ms` | integer | 500 | 100 | 5000 |
| `daemon.crash_recovery.max_backoff_ms` | integer | 30000 | 1000 | 300000 |
| `daemon.crash_recovery.backoff_multiplier` | float | 2.0 | 1.5 | 4.0 |
| `daemon.crash_recovery.max_crashes_per_window` | integer | 5 | 1 | 20 |
| `daemon.crash_recovery.window_seconds` | integer | 60 | 30 | 600 |

**Backoff formula**: `delay = min(initial_backoff_ms * (multiplier ^ attempt), max_backoff_ms)`

---

## Session Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `daemon.session.auto_compact_threshold` | integer | 0 | 0 | 1000000 |
| `daemon.session.idle_timeout_seconds` | integer | 0 | 0 | 86400 |
| `daemon.session.history_retention_days` | integer | 30 | 1 | 365 |

Set `auto_compact_threshold` to 0 to disable automatic compaction.

---

## Subagent Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `daemon.subagent.max_concurrent` | integer | 5 | 1 | 20 | `BETCODE_MAX_SUBAGENTS` |
| `daemon.subagent.max_per_session` | integer | 20 | 1 | 100 | - |
| `daemon.subagent.timeout_minutes` | integer | 30 | 1 | 120 | - |
| `daemon.subagent.default_max_turns` | integer | 50 | 1 | 200 | - |

---

## Permission Settings

**Tiered Timeout Policy** (see [ADR-001](./decisions/ADR-001-permission-timeout.md)):

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `daemon.permission.connected_timeout_seconds` | integer | 60 | 10 | 300 |
| `daemon.permission.disconnected_timeout_seconds` | integer | 604800 | 3600 | 2592000 |
| `daemon.permission.extend_on_activity` | boolean | true | - | - |
| `daemon.permission.auto_deny_no_client` | boolean | false | - | - |

**Behavior**:
- When client connected: `connected_timeout_seconds` applies (default 60s)
- When client disconnected: `disconnected_timeout_seconds` applies (default 7 days)
- When `extend_on_activity` is true, any client activity resets the disconnected timer

---

## Input Lock Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `daemon.input_lock.transfer_timeout_seconds` | integer | 10 | 5 | 60 |
| `daemon.input_lock.idle_transfer_seconds` | integer | 300 | 60 | 3600 |

---

## Client Tracking Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `daemon.client.heartbeat_timeout_seconds` | integer | 30 | 10 | 120 |
| `daemon.client.event_buffer_size` | integer | 1024 | 128 | 8192 |

---

## Relay Tunnel Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `daemon.relay.url` | string | null | - | - | `BETCODE_RELAY_URL` |
| `daemon.relay.reconnect_initial_ms` | integer | 1000 | 500 | 10000 | - |
| `daemon.relay.reconnect_max_ms` | integer | 60000 | 10000 | 300000 | - |
| `daemon.relay.reconnect_multiplier` | float | 2.0 | 1.5 | 4.0 | - |
| `daemon.relay.heartbeat_interval_seconds` | integer | 20 | 10 | 60 | - |
| `daemon.relay.heartbeat_timeout_seconds` | integer | 15 | 5 | 30 | - |

---

## Observability Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `daemon.observability.otlp_endpoint` | string | null | - | - | `OTEL_EXPORTER_OTLP_ENDPOINT` |
| `daemon.observability.metrics_enabled` | boolean | false | - | - | - |
| `daemon.observability.metrics_port` | integer | 9090 | 1024 | 65535 | - |

---

## SQLite Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `daemon.sqlite.busy_timeout_ms` | integer | 5000 | 1000 | 30000 |
| `daemon.sqlite.max_connections` | integer | 5 | 1 | 20 |
