# Configuration Examples

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Parent**: [CONFIG_REFERENCE.md](./CONFIG_REFERENCE.md)

---

## Minimal Configuration (All Defaults)

```json
{
  "daemon": {},
  "cli": {}
}
```

With this configuration, all parameters use their defaults. The daemon:
- Accepts 10 MB max payload
- Allows 5 concurrent Claude subprocesses
- Listens on platform-default socket
- Does not connect to a relay

---

## Development Configuration

```json
{
  "daemon": {
    "log_level": "debug",
    "max_payload_bytes": 20971520,
    "subprocess": {
      "max_concurrent": 3
    },
    "crash_recovery": {
      "max_crashes_per_window": 10,
      "window_seconds": 120
    },
    "permission": {
      "request_timeout_seconds": 120
    },
    "observability": {
      "metrics_enabled": true,
      "metrics_port": 9090
    }
  },
  "cli": {
    "theme": "dark",
    "show_timestamps": true,
    "headless": {
      "output_format": "stream-json"
    }
  }
}
```

---

## Production Daemon Configuration

```json
{
  "daemon": {
    "log_level": "info",
    "max_payload_bytes": 10485760,
    "max_sessions": 200,
    "subprocess": {
      "max_concurrent": 10,
      "queue_size": 100
    },
    "subprocess_timeout_seconds": 600,
    "crash_recovery": {
      "initial_backoff_ms": 1000,
      "max_backoff_ms": 60000,
      "max_crashes_per_window": 3,
      "window_seconds": 300
    },
    "session": {
      "auto_compact_threshold": 100000,
      "history_retention_days": 90
    },
    "subagent": {
      "max_concurrent": 10,
      "max_per_session": 50,
      "timeout_minutes": 60
    },
    "permission": {
      "request_timeout_seconds": 60,
      "auto_deny_no_client": true
    },
    "client": {
      "heartbeat_timeout_seconds": 60,
      "event_buffer_size": 2048
    },
    "relay": {
      "url": "https://relay.example.com:443",
      "heartbeat_interval_seconds": 15,
      "heartbeat_timeout_seconds": 10
    },
    "observability": {
      "otlp_endpoint": "http://otel-collector:4317",
      "metrics_enabled": true
    }
  }
}
```

---

## Production Relay Configuration

```json
{
  "relay": {
    "listen_address": "0.0.0.0:443",
    "tls_cert_path": "/etc/betcode-relay/tls/server.crt",
    "tls_key_path": "/etc/betcode-relay/tls/server.key",
    "log_level": "info",
    "auth": {
      "jwt_issuer": "betcode-relay",
      "jwt_audience": "betcode",
      "access_token_ttl_seconds": 900,
      "refresh_token_ttl_days": 7
    },
    "cert": {
      "ca_cert_path": "/etc/betcode-relay/ca/ca.crt",
      "ca_key_path": "/etc/betcode-relay/ca/ca.key",
      "validity_days": 365,
      "renewal_threshold_days": 30
    },
    "buffer": {
      "ttl_hours": 24,
      "max_per_machine": 1000,
      "max_message_bytes": 1048576,
      "purge_interval_minutes": 60
    },
    "rate_limit": {
      "registration_per_minute": 10,
      "token_refresh_per_minute": 30,
      "new_session_per_hour": 20,
      "subagent_per_hour": 50
    },
    "observability": {
      "otlp_endpoint": "http://otel-collector:4317",
      "metrics_enabled": true,
      "metrics_port": 9091
    },
    "sqlite": {
      "path": "/var/lib/betcode-relay/relay.db",
      "max_connections": 20
    }
  }
}
```

---

## High-Throughput Subagent Configuration

For orchestrators spawning many parallel subagents:

```json
{
  "daemon": {
    "subprocess": {
      "max_concurrent": 15,
      "queue_size": 150
    },
    "subagent": {
      "max_concurrent": 15,
      "max_per_session": 100,
      "timeout_minutes": 90,
      "default_max_turns": 100
    },
    "session": {
      "auto_compact_threshold": 50000
    },
    "sqlite": {
      "max_connections": 10
    }
  }
}
```
