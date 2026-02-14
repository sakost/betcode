# Configuration Validation Rules

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [CONFIG_REFERENCE.md](./CONFIG_REFERENCE.md)

---

## Startup Validation

The daemon and relay validate all configuration on startup. Invalid configuration
causes the process to exit with a descriptive error message.

### Type Validation

| Rule | Error Message Example |
|------|----------------------|
| Integer out of range | `daemon.max_payload_bytes must be between 1048576 and 104857600, got 0` |
| Invalid enum value | `daemon.log_level must be one of: error, warn, info, debug, trace` |
| Missing required field | `relay.tls_cert_path is required but not set` |
| Invalid path | `relay.tls_cert_path: file does not exist: /etc/foo` |
| Invalid type | `daemon.max_sessions must be an integer, got string` |

### Consistency Validation

| Rule | Error Message Example |
|------|----------------------|
| Backoff max < initial | `crash_recovery.max_backoff_ms must be >= initial_backoff_ms` |
| Timeout > interval | `relay.heartbeat_timeout_seconds must be < heartbeat_interval_seconds` |
| Buffer too small | `daemon.client.event_buffer_size must be >= 128` |

### Permission Validation

| Rule | Error Message Example |
|------|----------------------|
| Auto-approve without allowlist | `auto_approve_permissions requires non-empty allowed_tools list` |
| Invalid tool pattern | `permission rule contains invalid pattern: Bash([)` |

---

## Runtime Validation

Request-level validation occurs during gRPC call processing.

| Field | Validation | Error Code |
|-------|------------|------------|
| `session_id` | UTF-8, max 128 chars | `INVALID_ARGUMENT` |
| `prompt` | UTF-8, max 1 MB | `INVALID_ARGUMENT` |
| `tool_name` | UTF-8, max 256 chars, alphanumeric + underscore | `INVALID_ARGUMENT` |
| `allowed_tools` | Max 100 entries | `INVALID_ARGUMENT` |
| `orchestration_steps` | Max 50 entries, no cycles | `INVALID_ARGUMENT` |
| `file_path` | Canonicalized, within worktree | `PERMISSION_DENIED` |
| `working_directory` | Exists, readable | `INVALID_ARGUMENT` |

---

## Validation Error Codes

| Code | gRPC Status | Description |
|------|-------------|-------------|
| `CONFIG_INVALID` | `INVALID_ARGUMENT` | Configuration value invalid |
| `CONFIG_MISSING` | `INVALID_ARGUMENT` | Required configuration missing |
| `CONFIG_CONFLICT` | `INVALID_ARGUMENT` | Conflicting configuration values |
| `CONFIG_PATH_NOT_FOUND` | `NOT_FOUND` | Configured path does not exist |
| `CONFIG_PERMISSION_DENIED` | `PERMISSION_DENIED` | Cannot read configured path |

---

## Validation Behavior

### Fail-Fast Principle

Configuration errors cause immediate process exit with:
- Non-zero exit code (1)
- Human-readable error message to stderr
- JSON-formatted error if `--output-format json` specified

### Environment Variable Priority

Environment variables override config file values but are still validated:

```
# This will fail validation even though it's from env var
BETCODE_MAX_PAYLOAD_BYTES=0 betcode-daemon
# Error: daemon.max_payload_bytes must be between 1048576 and 104857600, got 0
```

### Unknown Fields

Unknown configuration fields are:
- **Ignored** in config files (forward compatibility)
- **Logged** at WARN level for debugging
- **Not** propagated to Claude subprocess
