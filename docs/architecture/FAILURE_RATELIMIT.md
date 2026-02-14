# Anthropic API Rate Limit Cascading

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [FAILURE_MODES.md](./FAILURE_MODES.md)

---

## The Problem

When Claude Code hits Anthropic's rate limits, it returns an error result. If
multiple sessions are running, they may all hit the limit simultaneously,
creating a retry storm that makes the situation worse.

---

## Rate Limit Sources

| Source | Limit Type | Typical Values |
|--------|-----------|----------------|
| Anthropic API | Requests per minute | 60 RPM (varies by tier) |
| Anthropic API | Tokens per minute | 100k TPM (varies) |
| Anthropic API | Concurrent requests | 10-50 |
| Relay (BetCode) | Sessions per hour | 20 per user |

---

## Detection

Claude Code reports rate limits in `result` messages:

```json
{
  "type": "result",
  "subtype": "error",
  "error": "Rate limit exceeded",
  "error_details": {
    "retry_after_seconds": 60,
    "limit_type": "requests_per_minute"
  }
}
```

The daemon monitors these to track API health.

---

## Cascade Prevention

```
         Daemon-level Rate Limiter
                   |
    +--------------+--------------+
    |              |              |
Session 1     Session 2     Session 3
    |              |              |
    v              v              v
         Anthropic API

When rate limit detected:
1. Circuit breaker opens for Anthropic API
2. All sessions see "API temporarily unavailable"
3. New subprocess spawns are queued (not blocked)
4. After cooldown, circuit half-opens
5. One session tests; if success, circuit closes
```

---

## Circuit Breaker Configuration

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Failure threshold | 3 rate limits in 60s | Detect sustained limits |
| Open timeout | 60s (or retry-after) | Respect API guidance |
| Half-open probes | 1 session | Minimize probe cost |
| Success threshold | 1 | Quick recovery |

---

## User Experience

When circuit is open, clients see:

```
API temporarily unavailable due to rate limits.
Automatic retry in 45 seconds.
```

Sessions are NOT terminated. They wait for the circuit to close, then resume
automatically.
