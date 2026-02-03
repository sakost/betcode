# ADR-001: Permission Request Timeout Policy

**Status**: Accepted | **Date**: 2026-02-03

## Context

Permission requests occur when Claude Code wants to execute a tool that requires user approval. The daemon forwards these to the client, which must respond within a timeout period.

**Position A (60 seconds)**: Keeps agent responsive, prevents indefinite blocking, aligns with interactive terminal workflows, fails fast when users are unavailable.

**Position B (7 days)**: Mobile users may be offline for extended periods, permission requests should persist until user can respond, 60 seconds is too aggressive for asynchronous mobile workflows.

**Underlying Tension**: Synchronous terminal workflows (60s is generous) vs. asynchronous mobile workflows (60s is impossibly short).

## Decision

**Implement a tiered timeout policy:**

| Tier | Timeout | Condition |
|------|---------|-----------|
| Client-Connected | 60 seconds | Client holds input lock |
| Client-Disconnected | 7 days | No client connected |
| Session Override | Configurable | Per-session settings |

**Behavior**:
- When client connected: 60s timeout, auto-deny on expiry
- When disconnected: persist in `pending_permissions`, 7-day TTL, push notification sent
- Timeout does NOT reset on reconnection (prevents indefinite extension)

**Configuration** (`settings.json`):
```json
{
  "permissions": {
    "connected_timeout_seconds": 60,
    "disconnected_timeout_seconds": 604800
  }
}
```

## Consequences

**Positive**: Mobile users get reasonable response window; CLI users retain fast-fail behavior; configurable for different workflows.

**Negative**: Daemon must persist permissions across restarts; increased complexity; 7-day old requests may be stale.

## Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Always 60s | Hostile to mobile/async workflows |
| Always 7 days | Blocks interactive sessions unnecessarily |
| Infinite timeout | Unbounded resource consumption |
