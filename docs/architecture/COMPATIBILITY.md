# Claude Code Compatibility

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

---

## Overview

BetCode wraps Claude Code as a subprocess and depends on its `--output-format stream-json`
NDJSON protocol. This document tracks version compatibility between BetCode releases and
Claude Code versions.

---

## Compatibility Matrix

| BetCode Version | Claude Code Version | Status | Notes |
|-----------------|---------------------|--------|-------|
| 0.1.0-alpha.1 | TBD | Pre-release | Initial implementation; exact version range to be determined after broader testing |

This matrix will be updated as BetCode releases are tested against specific Claude Code versions.

---

## Version Detection

BetCode detects the installed Claude Code version at daemon startup:

1. Runs `claude --version` and parses the output
2. Compares against the supported version range for the current BetCode release
3. Logs a warning if the version is outside the tested range
4. Refuses to start if the version is below the minimum supported version

The minimum supported version and tested version range are compiled into each BetCode release.

---

## NDJSON Protocol Stability

BetCode depends on the following Claude Code interfaces:

| Interface | Flag | Risk Level |
|-----------|------|------------|
| NDJSON output | `--output-format stream-json` | Medium -- documented public interface |
| JSON input | `--input-format stream-json` | Medium -- documented public interface |
| Permission prompts | `--permission-prompt-tool stdio` | Medium -- documented public interface |
| Session resume | `--resume` | Low -- stable CLI flag |

Changes to these interfaces would require a BetCode update. See the
[Risk Register](./ROADMAP.md#risk-register) for mitigation strategies.

---

## Related Documents

| Document | Description |
|----------|-------------|
| [ADR-004: Version Compatibility](./decisions/ADR-004-version-compatibility.md) | Decision record for version detection strategy |
| [PROTOCOL_L1.md](./PROTOCOL_L1.md) | Claude SDK NDJSON protocol details |
| [ROADMAP.md](./ROADMAP.md) | Risk register with NDJSON stability risks |
