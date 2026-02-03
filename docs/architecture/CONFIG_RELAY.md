# Relay Configuration Reference

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Parent**: [CONFIG_REFERENCE.md](./CONFIG_REFERENCE.md)

Relay settings live in `$RELAY_CONFIG_DIR/settings.json` (typically `/etc/betcode-relay/settings.json`).

---

## Server Settings

| Parameter | Type | Default | Env Override |
|-----------|------|---------|--------------|
| `relay.listen_address` | string | "0.0.0.0:443" | `BETCODE_RELAY_LISTEN` |
| `relay.tls_cert_path` | string | (required) | `BETCODE_RELAY_TLS_CERT` |
| `relay.tls_key_path` | string | (required) | `BETCODE_RELAY_TLS_KEY` |
| `relay.log_level` | string | "info" | `BETCODE_RELAY_LOG_LEVEL` |

---

## Authentication Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `relay.auth.jwt_secret` | string | (required) | - | - | `BETCODE_JWT_SECRET` |
| `relay.auth.jwt_issuer` | string | "betcode-relay" | - | - | - |
| `relay.auth.jwt_audience` | string | "betcode" | - | - | - |
| `relay.auth.access_token_ttl_seconds` | integer | 900 | 300 | 3600 | - |
| `relay.auth.refresh_token_ttl_days` | integer | 7 | 1 | 30 | - |

**Security note**: Prefer setting `jwt_secret` via environment variable.

---

## Certificate Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `relay.cert.ca_cert_path` | string | (required) | - | - |
| `relay.cert.ca_key_path` | string | (required) | - | - |
| `relay.cert.validity_days` | integer | 365 | 30 | 730 |
| `relay.cert.renewal_threshold_days` | integer | 30 | 7 | 90 |

---

## Message Buffer Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `relay.buffer.ttl_hours` | integer | 168 | 1 | 720 |
| `relay.buffer.max_per_machine` | integer | 1000 | 100 | 10000 |
| `relay.buffer.max_message_bytes` | integer | 1048576 | 65536 | 10485760 |
| `relay.buffer.purge_interval_minutes` | integer | 60 | 10 | 360 |

---

## Rate Limiting Settings

| Parameter | Type | Default | Min | Max |
|-----------|------|---------|-----|-----|
| `relay.rate_limit.registration_per_minute` | integer | 10 | 1 | 100 |
| `relay.rate_limit.token_refresh_per_minute` | integer | 30 | 5 | 100 |
| `relay.rate_limit.new_session_per_hour` | integer | 20 | 5 | 200 |
| `relay.rate_limit.subagent_per_hour` | integer | 50 | 10 | 500 |
| `relay.rate_limit.tunnel_registration_per_minute` | integer | 5 | 1 | 20 |

---

## Push Notification Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `relay.push.fcm_credentials_path` | string | null | - | - | `BETCODE_FCM_CREDENTIALS` |
| `relay.push.apns_key_path` | string | null | - | - | - |
| `relay.push.apns_key_id` | string | null | - | - | - |
| `relay.push.apns_team_id` | string | null | - | - | - |
| `relay.push.retry_max_attempts` | integer | 5 | 1 | 10 | - |
| `relay.push.retry_backoff_ms` | integer | 1000 | 500 | 5000 | - |
| `relay.push.retry_max_backoff_ms` | integer | 30000 | 5000 | 60000 | - |

---

## Observability Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `relay.observability.otlp_endpoint` | string | null | - | - | `OTEL_EXPORTER_OTLP_ENDPOINT` |
| `relay.observability.metrics_enabled` | boolean | false | - | - | - |
| `relay.observability.metrics_port` | integer | 9091 | 1024 | 65535 | - |

---

## SQLite Settings

| Parameter | Type | Default | Min | Max | Env Override |
|-----------|------|---------|-----|-----|--------------|
| `relay.sqlite.path` | string | "/var/lib/betcode-relay/relay.db" | - | - | `BETCODE_RELAY_DB` |
| `relay.sqlite.busy_timeout_ms` | integer | 5000 | 1000 | 30000 | - |
| `relay.sqlite.max_connections` | integer | 10 | 1 | 50 | - |
