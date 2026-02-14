# Architecture Decision Records

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

This document indexes key architectural decisions for BetCode, synthesizing competing viewpoints into coherent policies. Each ADR follows the standard format: Context, Decision, Consequences, and Alternatives Considered.

---

## Decision Index

| ADR | Title | Status | Key Trade-off |
|-----|-------|--------|---------------|
| [ADR-001](./decisions/ADR-001-permission-timeout.md) | Permission Request Timeout Policy | Accepted | Responsiveness vs. mobile flexibility |
| [ADR-002](./decisions/ADR-002-buffer-retention.md) | Message Buffer Retention Strategy | Accepted | Storage cost vs. offline support |
| [ADR-003](./decisions/ADR-003-auto-approve-governance.md) | Auto-Approve Governance Framework | Accepted | Security friction vs. automation power |
| [ADR-004](./decisions/ADR-004-version-compatibility.md) | Claude Code Version Compatibility | Accepted | Stability vs. flexibility |
| [ADR-005](./decisions/ADR-005-diagram-standard.md) | Diagram Standard (Mermaid) | Accepted | Consistency vs. migration effort |

---

## Decision Principles

These ADRs were guided by the following principles:

1. **No false dichotomies**: Most debates have valid points on both sides. Look for tiered or configurable solutions.

2. **Secure by default, configurable for advanced users**: Don't force all users into the most restrictive mode, but don't expose them to risk without explicit opt-in.

3. **Document the WHY**: Future maintainers need to understand the reasoning, not just the decision.

4. **Prefer reversible decisions**: Where possible, choose options that can be changed later without breaking existing deployments.

5. **Daemon is source of truth**: When in doubt about state management, the daemon's SQLite database is authoritative.

---

## Summary

### ADR-001: Permission Timeout Policy
**Decision**: Tiered timeout - 60 seconds when client connected, 7 days when disconnected.
**Rationale**: Interactive CLI sessions need fast-fail behavior; mobile users need extended windows.

### ADR-002: Buffer Retention Strategy
**Decision**: Configurable retention - 24h default, 7d available, priority-based purging.
**Rationale**: Balance storage costs against legitimate offline use cases.

### ADR-003: Auto-Approve Governance
**Decision**: Mandatory constraints (non-empty allowed_tools, 90-day audit retention) plus configurable limits.
**Rationale**: Prevent accidental unrestricted auto-approve while enabling automation.

### ADR-004: Version Compatibility
**Decision**: Strict in production (refuse known-incompatible), lenient in development (warn on unknown).
**Rationale**: Protect production while allowing experimentation.

### ADR-005: Diagram Standard
**Decision**: Mermaid diagrams for all architecture documentation.
**Rationale**: Git-friendly, renders in GitHub/GitLab, lower maintenance than ASCII.

---

## References

- [DAEMON.md](./DAEMON.md) - Daemon architecture
- [SECURITY.md](./SECURITY.md) - Security model
- [SUBAGENTS.md](./SUBAGENTS.md) - Multi-agent orchestration
- [TOPOLOGY.md](./TOPOLOGY.md) - Network architecture
- [SCHEMAS.md](./SCHEMAS.md) - Database schemas
