# BetCode Configuration Reference

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Status**: Design Phase

This document provides the authoritative reference for all BetCode configuration
parameters, including defaults, valid ranges, environment variable mappings, and
validation rules.

---

## Configuration Hierarchy

Configuration is resolved in priority order (highest to lowest):

1. **Command-line flags** - Explicit overrides
2. **Environment variables** - Runtime configuration
3. **Project local settings** - `.claude/settings.local.json` (gitignored)
4. **Project settings** - `.claude/settings.json` (committed)
5. **User settings** - `$BETCODE_CONFIG_DIR/settings.json`
6. **Built-in defaults** - Hardcoded sensible values

**Important**: Higher priority values completely override lower priority values
for the same key (no merging of nested objects).

---

## Configuration Loading

### Config Directory Resolution

| Platform | Default Path | Override |
|----------|--------------|----------|
| Linux | `$XDG_CONFIG_HOME/betcode` (default: `~/.config/betcode`) | `$BETCODE_CONFIG_DIR` |
| macOS | `~/Library/Application Support/betcode` | `$BETCODE_CONFIG_DIR` |
| Windows | `%USERPROFILE%\.betcode` | `$BETCODE_CONFIG_DIR` |

### File Locations

```
$BETCODE_CONFIG_DIR/
  settings.json           # User-level settings
  rules/*.md              # User permission rules (copied from ~/.claude/)
  certs/
    client.key            # mTLS private key (600 permissions)
    client.crt            # mTLS certificate
    ca.crt                # Relay CA certificate
  auth.json               # JWT tokens (fallback from OS keyring)
  daemon.db               # SQLite database
```

---

## Document Index

Configuration is split across focused documents:

| Document | Contents |
|----------|----------|
| [CONFIG_DAEMON.md](./CONFIG_DAEMON.md) | Daemon settings (subprocess, session, subagent, permissions) |
| [CONFIG_RELAY.md](./CONFIG_RELAY.md) | Relay settings (auth, buffer, rate limits, push) |
| [CONFIG_CLIENTS.md](./CONFIG_CLIENTS.md) | CLI and Flutter client settings |
| [CONFIG_EXAMPLES.md](./CONFIG_EXAMPLES.md) | Minimal, development, and production examples |
| [CONFIG_VALIDATION.md](./CONFIG_VALIDATION.md) | Validation rules and error messages |
| [CONFIG_ENV_VARS.md](./CONFIG_ENV_VARS.md) | Complete environment variable reference |

---

## Quick Reference: Key Parameters

### Daemon (Most Common)

| Parameter | Default | Range | Env Override |
|-----------|---------|-------|--------------|
| `daemon.max_payload_bytes` | 10 MB | 1-100 MB | `BETCODE_MAX_PAYLOAD_BYTES` |
| `daemon.subprocess.max_concurrent` | 5 | 1-20 | `BETCODE_MAX_SUBPROCESSES` |
| `daemon.subagent.max_concurrent` | 5 | 1-20 | `BETCODE_MAX_SUBAGENTS` |
| `daemon.permission.request_timeout_seconds` | 60 | 10-300 | - |
| `daemon.relay.url` | null | - | `BETCODE_RELAY_URL` |

### Relay (Most Common)

| Parameter | Default | Range | Env Override |
|-----------|---------|-------|--------------|
| `relay.auth.access_token_ttl_seconds` | 900 | 300-3600 | - |
| `relay.buffer.ttl_hours` | 168 | 1-720 | - |
| `relay.buffer.max_per_machine` | 1000 | 100-10000 | - |
| `relay.rate_limit.new_session_per_hour` | 20 | 5-200 | - |

---

## Terminology Standards

To resolve terminology inconsistencies (Issue #11):

### Process Terminology

| Term | Definition | Usage |
|------|------------|-------|
| **subprocess** | A child process spawned by the daemon (`claude` CLI) | "The daemon spawns a subprocess for each session" |
| **subagent** | An independent Claude subprocess for parallel task execution | "Spawn a subagent to work on documentation" |
| **daemon** | The betcode-daemon process itself | "Start the daemon" |

**Avoid**: "process" alone (ambiguous), "claude_process" (redundant)

### Time-Related Terminology

| Term | Definition | Standard Suffix |
|------|------------|-----------------|
| **timeout** | Max wait time before operation fails | `*_timeout_seconds` |
| **ttl** | Time To Live - duration until expiration | `*_ttl_hours`, `*_ttl_days` |
| **interval** | Time between recurring operations | `*_interval_seconds` |
| **backoff** | Delay between retry attempts | `*_backoff_ms` |

### Size-Related Terminology

| Term | Unit | Standard Suffix |
|------|------|-----------------|
| Bytes | Bytes | `*_bytes` |
| Count | Count | `*_size`, `*_count` |

---

## Related Documents

| Document | Description |
|----------|-------------|
| [DAEMON.md](./DAEMON.md) | Daemon architecture, subprocess management |
| [TOPOLOGY.md](./TOPOLOGY.md) | Relay architecture, connection modes |
| [SECURITY.md](./SECURITY.md) | Auth, rate limiting, audit logging |
| [GLOSSARY.md](./GLOSSARY.md) | Terminology definitions |
