# BetCode -- Agent Definitions

Reference for multi-agent workflows in BetCode development.

## Context Sources

Before starting work, agents should read the relevant context:

- **Project conventions**: [CONTRIBUTING.md](CONTRIBUTING.md)
- **Architecture overview**: [docs/architecture/OVERVIEW.md](docs/architecture/OVERVIEW.md)
- **Implementation status**: [docs/architecture/ROADMAP.md](docs/architecture/ROADMAP.md)
- **Crate-level context**: Each crate has a `README.md` linking to relevant architecture docs

## Agent Roles

### Implementer

Works on feature implementation within a single crate or across crates.

**Context**: Read the target crate's `README.md` and linked architecture docs before coding.
**Quality gates**: Run `just check` before marking work complete.

### Reviewer

Reviews code changes for correctness, style, and architecture alignment.

**Context**: Read [CONTRIBUTING.md](CONTRIBUTING.md) for code quality standards and PR requirements.
**Focus areas**: No `unwrap()` in production code, clippy compliance, test coverage, conventional commits.

### Architect

Plans cross-crate changes and evaluates design decisions.

**Context**: Read [OVERVIEW.md](docs/architecture/OVERVIEW.md) and the relevant architecture docs.
**Output**: Design proposals referencing existing architecture decisions in `docs/architecture/decisions/`.

### Proto Designer

Designs and modifies protobuf API contracts.

**Context**: Read [PROTOCOL_L2.md](docs/architecture/PROTOCOL_L2.md) and [release-policy.md](docs/policies/release-policy.md).
**Rules**: Non-breaking additions can ship freely. Breaking changes require an issue and coordinated release.

## Workspace Structure

```
crates/
├── betcode-proto/      # Protobuf codegen (tonic-build)
├── betcode-core/       # Shared types, config, errors
├── betcode-crypto/     # mTLS certificates
├── betcode-daemon/     # Daemon binary (subprocess mgr, gRPC server)
├── betcode-cli/        # CLI binary (ratatui TUI)
├── betcode-relay/      # Relay binary (gRPC router)
├── betcode-setup/      # Setup wizard
└── betcode-releases/   # Release packaging
```
