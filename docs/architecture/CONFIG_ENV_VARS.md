# Environment Variable Reference

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [CONFIG_REFERENCE.md](./CONFIG_REFERENCE.md)

---

## Daemon Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `BETCODE_CONFIG_DIR` | (platform-specific) | Override config directory |
| `BETCODE_LOG_LEVEL` | "info" | Log level: error, warn, info, debug, trace |
| `BETCODE_MAX_PAYLOAD_BYTES` | 10485760 | Max NDJSON payload (bytes) |
| `BETCODE_MAX_SESSIONS` | 100 | Max concurrent sessions |
| `BETCODE_MAX_SUBPROCESSES` | 5 | Max Claude subprocesses |
| `BETCODE_MAX_SUBAGENTS` | 5 | Max subagent subprocesses |
| `BETCODE_SUBPROCESS_TIMEOUT` | 300 | Subprocess idle timeout (seconds) |
| `BETCODE_SUBPROCESS_QUEUE` | 50 | Queue size when pool full |
| `BETCODE_SOCKET_PATH` | (platform-specific) | Local IPC socket/pipe path |
| `BETCODE_RELAY_URL` | (none) | Relay server URL |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | (none) | OpenTelemetry collector endpoint |

---

## Relay Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `BETCODE_RELAY_LISTEN` | "0.0.0.0:443" | gRPC listen address |
| `BETCODE_RELAY_TLS_CERT` | (required) | TLS certificate path |
| `BETCODE_RELAY_TLS_KEY` | (required) | TLS private key path |
| `BETCODE_RELAY_LOG_LEVEL` | "info" | Log level |
| `BETCODE_RELAY_DB` | "/var/lib/betcode-relay/relay.db" | SQLite database path |
| `BETCODE_JWT_SECRET` | (required) | JWT signing key |
| `BETCODE_FCM_CREDENTIALS` | (none) | Firebase credentials JSON path |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | (none) | OpenTelemetry collector endpoint |

---

## Claude Code Passthrough Variables

The daemon passes these to Claude subprocesses:

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | Yes | Anthropic API key |
| `ANTHROPIC_BASE_URL` | No | Custom API endpoint (proxies, Bedrock, Vertex) |
| `PATH` | Yes | System PATH for tool execution |
| `HOME` / `USERPROFILE` | Yes | Home directory |
| `TERM` | No | Terminal type |
| `LANG` | No | Locale settings |
| `TZ` | No | Timezone |

---

## Prohibited Environment Overrides

These patterns cannot be passed via `SpawnSubagentRequest.env`:

| Pattern | Reason |
|---------|--------|
| `ANTHROPIC_*` | Prevents API key injection/override |
| `BETCODE_*` | Prevents daemon config manipulation |
| `CLAUDE_*` | Prevents Claude behavior manipulation |
| `LD_PRELOAD` | Security risk |
| `DYLD_*` | Security risk (macOS) |

---

## Platform-Specific Defaults

### Linux

| Variable | Default |
|----------|---------|
| `BETCODE_CONFIG_DIR` | `$XDG_CONFIG_HOME/betcode` or `~/.config/betcode` |
| `BETCODE_SOCKET_PATH` | `/run/user/$UID/betcode/daemon.sock` |

### macOS

| Variable | Default |
|----------|---------|
| `BETCODE_CONFIG_DIR` | `~/Library/Application Support/betcode` |
| `BETCODE_SOCKET_PATH` | `/run/user/$UID/betcode/daemon.sock` |

### Windows

| Variable | Default |
|----------|---------|
| `BETCODE_CONFIG_DIR` | `%USERPROFILE%\.betcode` |
| `BETCODE_SOCKET_PATH` | `\\.\pipe\betcode-daemon-$USERNAME` |

---

## Example: Environment-Based Configuration

```bash
# Development setup
export BETCODE_LOG_LEVEL=debug
export BETCODE_MAX_SUBPROCESSES=3
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# Production relay
export BETCODE_RELAY_LISTEN=0.0.0.0:443
export BETCODE_RELAY_TLS_CERT=/etc/ssl/betcode/server.crt
export BETCODE_RELAY_TLS_KEY=/etc/ssl/betcode/server.key
export BETCODE_JWT_SECRET=$(cat /run/secrets/jwt_secret)
export BETCODE_RELAY_DB=/var/lib/betcode-relay/relay.db
```
