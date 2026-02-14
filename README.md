# BetCode

**Multi-client, multi-machine infrastructure for Claude Code.**

BetCode wraps [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (Anthropic's official CLI) as a subprocess and builds orchestration, transport, persistence, and UI layers around it. Access your coding agent from a terminal TUI, a mobile app, or across machines via a self-hosted relay -- all with full agent fidelity.

> **Disclaimer**: BetCode is an independent, community-driven project. It is **not** affiliated with, endorsed by, or sponsored by Anthropic or the Claude team. Claude and Claude Code are products of Anthropic. BetCode simply wraps the publicly available Claude Code CLI.

---

## Status

**v0.1.0-alpha.1** -- Pre-release. Phase 1 (foundation crates) is implemented and passing CI. See [CHANGELOG.md](CHANGELOG.md) and [ROADMAP.md](docs/architecture/ROADMAP.md) for details.

---

## Why BetCode?

Claude Code is a powerful coding agent, but it runs as a single-user CLI on one machine. BetCode adds the missing infrastructure:

| Capability | Claude Code | BetCode |
|---|---|---|
| Agent intelligence | Native | Inherited (wrapper) |
| Mobile client | No | Flutter app ([separate repo](https://github.com/sakost/betcode_app)) |
| Multi-machine access | No | Self-hosted relay + mTLS tunnel |
| Git worktree management | No | First-class |
| GitLab integration | GitHub only | GitLab-native |
| Offline queueing | No | Client-side SQLite |
| Session multiplexing | Single user | Multi-client |
| Self-hosted relay | No | Built-in |

Because BetCode runs Claude Code as a subprocess, **every tool, MCP server, hook, skill, and prompt that works with `claude` works identically with BetCode**. Updates to Claude Code are automatically available.

---

## Architecture

BetCode treats Claude Code as an opaque subprocess. The daemon spawns `claude` processes, bridges NDJSON events to gRPC, and multiplexes sessions across clients. See [OVERVIEW.md](docs/architecture/OVERVIEW.md) for C4 diagrams, design decisions, and the full tech stack.

- **betcode-daemon** -- Spawns Claude Code subprocesses, bridges NDJSON to gRPC, multiplexes sessions, manages worktrees
- **betcode-relay** -- Public gRPC router with JWT + mTLS auth, routes traffic to machines
- **betcode-cli** -- Terminal TUI client (ratatui), streaming markdown, permission prompts
- **betcode-proto** -- Shared protobuf definitions and generated gRPC code
- **betcode-core** -- Shared types, config parsing, error types
- **betcode-crypto** -- mTLS certificate generation and management
- **betcode-setup** -- First-run setup wizard
- **betcode-releases** -- Release artifact packaging
- **[betcode_app](https://github.com/sakost/betcode_app)** -- Flutter mobile client (separate repo)

---

## Workspace Structure

```
betcode/
├── Cargo.toml                    # Workspace root (edition 2024)
├── proto/betcode/v1/             # Shared protobuf definitions (git submodule)
├── crates/
│   ├── betcode-proto/            # Generated protobuf code (tonic-build)
│   ├── betcode-core/             # Shared types, config parsing, errors
│   ├── betcode-crypto/           # mTLS certificate generation
│   ├── betcode-daemon/           # Daemon binary
│   ├── betcode-cli/              # CLI client binary
│   ├── betcode-relay/            # Relay server binary
│   ├── betcode-setup/            # First-run setup wizard
│   └── betcode-releases/         # Release artifact packaging
└── docs/architecture/            # Architecture documentation
```

---

## Prerequisites

- **Claude Code** must be installed on each machine that runs the daemon.
- **Rust** (stable, edition 2024) -- install via [rustup](https://rustup.rs/)
- **protoc** (protobuf compiler) -- `apt install protobuf-compiler` or `brew install protobuf`
- **just** (command runner) -- `cargo install just` or see [installation](https://github.com/casey/just#installation)
- **Node.js 20+** -- for duplicate code detection (`jscpd`)

---

## Quick Start

```bash
git clone --recurse-submodules https://github.com/sakost/betcode.git
cd betcode
cargo build --workspace
just check  # runs all quality gates (fmt, clippy, test, deny, machete, jscpd)
```

If you forgot `--recurse-submodules`:

```bash
git submodule update --init --recursive
```

---

## Documentation

Detailed architecture documentation lives in [`docs/architecture/`](docs/architecture/):

| Document | Description |
|---|---|
| [OVERVIEW.md](docs/architecture/OVERVIEW.md) | System overview, C4 diagrams, tech stack |
| [DAEMON.md](docs/architecture/DAEMON.md) | Daemon internals, subprocess management |
| [PROTOCOL.md](docs/architecture/PROTOCOL.md) | Protocol layer reference |
| [PROTOCOL_L1.md](docs/architecture/PROTOCOL_L1.md) | Claude SDK NDJSON protocol |
| [PROTOCOL_L2.md](docs/architecture/PROTOCOL_L2.md) | BetCode gRPC API (proto definitions) |
| [PROTOCOL_BRIDGE.md](docs/architecture/PROTOCOL_BRIDGE.md) | Protocol bridging, reconnection |
| [TOPOLOGY.md](docs/architecture/TOPOLOGY.md) | Network topology, relay architecture |
| [CLIENTS.md](docs/architecture/CLIENTS.md) | CLI and Flutter client architecture |
| [SCHEMAS.md](docs/architecture/SCHEMAS.md) | SQLite schema designs |
| [SECURITY.md](docs/architecture/SECURITY.md) | Auth, authorization, sandboxing |
| [SUBAGENTS.md](docs/architecture/SUBAGENTS.md) | Multi-agent orchestration, DAG scheduling |
| [CONFIG_REFERENCE.md](docs/architecture/CONFIG_REFERENCE.md) | Configuration reference and sub-docs |
| [ROADMAP.md](docs/architecture/ROADMAP.md) | Phased implementation plan |
| [GLOSSARY.md](docs/architecture/GLOSSARY.md) | Terminology definitions |
| [ARCHITECTURE_DECISIONS.md](docs/architecture/ARCHITECTURE_DECISIONS.md) | ADR index |

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions, code quality standards, commit conventions, and PR requirements.

---

## License

Licensed under the [Apache License 2.0](LICENSE).

```
Copyright 2026 Konstantin Sazhenov
```

---

## Disclaimer

This project is **not** affiliated with, endorsed by, or sponsored by [Anthropic](https://www.anthropic.com/). "Claude" and "Claude Code" are trademarks or products of Anthropic, PBC. BetCode is an independent open-source project that wraps the publicly available Claude Code CLI. Use of Claude Code is subject to Anthropic's own terms of service.
