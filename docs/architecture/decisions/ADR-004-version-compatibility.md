# ADR-004: Claude Code Version Compatibility Strategy

**Status**: Accepted | **Date**: 2026-02-03

## Context

BetCode wraps Claude Code as a subprocess. Claude Code's SDK protocol may change without notice.

**Position A (Strict)**: Refuse to start if version unknown, maintain explicit version-to-protocol mapping, fail fast.

**Position B (Lenient)**: Best-effort compatibility, log warnings but continue, trust backward compatibility.

**Underlying Tension**: Strict provides predictability but creates upgrade friction. Lenient provides flexibility but risks silent failures.

## Decision

**Different modes for different environments:**

### Production Mode (Default)

| Version Status | Action |
|----------------|--------|
| Known compatible | Start normally |
| Unknown version | Start with warning |
| Known incompatible | Refuse to start |

### Development Mode (`--dev`)

| Version Status | Action |
|----------------|--------|
| Any version | Start with warning if unknown |
| Protocol errors | Log and continue where possible |

### Compatibility Matrix

Maintained in `COMPATIBILITY.md`:

| BetCode | Claude Code Min | Claude Code Max |
|---------|-----------------|-----------------|
| 0.2.x | 1.0.15 | 1.2.x |

### Adapter Layer

```rust
trait ClaudeProtocolAdapter {
    fn parse_message(&self, line: &str) -> Result<ClaudeMessage>;
    fn format_response(&self, response: ControlResponse) -> String;
}
```

Unknown versions use latest adapter with warning.

## Consequences

**Positive**: Production protected from untested versions; dev can experiment freely; clear error messages.

**Negative**: Must maintain compatibility matrix; may lag behind releases.

## Alternatives Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Always strict | Too much friction for minor bumps |
| Always lenient | Silent failures in production |
| Pin exact version | Incompatible with npm update |
