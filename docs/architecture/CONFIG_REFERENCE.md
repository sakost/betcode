# BetCode Configuration Reference

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

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

## Quick Reference

For parameter details, defaults, ranges, and environment variable overrides, see the individual configuration documents listed in the [Document Index](#document-index) above.

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
