# ADR-002: Message Buffer Retention Strategy

**Status**: Accepted | **Date**: 2026-02-03

## Context

The relay buffers messages for offline daemons, enabling asynchronous workflows.

**Position A (24 hours)**: Limits storage costs, prevents unbounded growth, messages older than 24h are likely stale.

**Position B (7 days)**: Better offline support for weekend/vacation scenarios, storage is cheap, matches permission timeout for consistency.

**Underlying Tension**: Relay is designed as lightweight router; extended buffering transforms it toward message queue.

## Decision

**Configurable retention with tiered defaults:**

| Tier | Default TTL | Max Messages |
|------|-------------|--------------|
| Standard | 24 hours | 1000 |
| Extended | 7 days | 5000 |
| Self-hosted | 1h - 30 days | 100 - 10000 |

**Priority-based purging**:
- Permission requests: max TTL (critical)
- User questions: max TTL
- Session updates: default TTL
- Heartbeats: 1 hour

**Configuration** (`relay.toml`):
```toml
[buffer]
default_ttl_hours = 24
max_ttl_hours = 168
max_messages_per_machine = 1000
```

## Consequences

**Positive**: Sensible defaults for most users; extended option available; priority system protects critical messages.

**Negative**: Increased storage for extended retention; complexity in priority-based purging.

## Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Always 24h | Poor mobile/vacation UX |
| Always 7d | Unnecessary storage for always-on daemons |
| Unlimited | Unbounded growth |
