# ADR-003: Auto-Approve Governance Framework

**Status**: Accepted | **Date**: 2026-02-03

## Context

Subagents can be spawned with `auto_approve_permissions = true`, allowing tool execution without user confirmation.

**Position A (Security-First)**: Max 1-hour sessions, strict tool allowlists, audit everything, real-time monitoring with automatic revocation.

**Position B (Usability-First)**: Let orchestrators decide constraints, don't over-restrict, trust operators, audit logging is sufficient.

**Underlying Tension**: BetCode is infrastructure, not policy. But insecure defaults could cause harm.

## Decision

**Secure by default, configurable for advanced users:**

### Mandatory Constraints (Cannot Be Bypassed)

| Constraint | Enforcement |
|------------|-------------|
| `allowed_tools` required when `auto_approve = true` | Daemon validation |
| Audit logging for all auto-approved calls | Daemon SQLite |
| 90-day minimum audit retention | Cannot be reduced |
| Session-scoped auto-approve | Daemon enforces |

### Configurable Limits (Secure Defaults)

| Parameter | Default | Range |
|-----------|---------|-------|
| `max_auto_approve_duration_seconds` | 3600 (1h) | 60 - 86400 |
| `max_auto_approve_tool_calls` | 1000 | 10 - 10000 |
| `auto_approve_rate_limit` | 60/minute | 1 - 300 |

### Mid-Execution Revocation

`RevokeAutoApprove { subagent_id }` RPC sets `auto_approve = false`; subsequent calls require approval.

## Consequences

**Positive**: Cannot accidentally grant unlimited auto-approve; full audit trail; revocation mechanism.

**Negative**: More restrictive than some prefer; audit storage grows with usage.

## Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| No auto-approve | Eliminates valuable automation |
| Unrestricted auto-approve | Unacceptable security risk |
| Per-tool caching | Insufficient audit granularity |
