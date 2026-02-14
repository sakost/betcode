# BetCode -- Claude Code Project Instructions

## Project Overview

BetCode is a multi-client, multi-machine infrastructure for Claude Code. It wraps the Claude Code CLI as a subprocess and builds orchestration, transport, persistence, and UI layers around it.

- **Language**: Rust (edition 2024)
- **Workspace**: 8 crates in `crates/`
- **Version**: 0.1.0-alpha.1 (pre-release)
- **License**: Apache-2.0

## Build and Test

```bash
cargo build --workspace          # Full build
cargo test --workspace           # Run all tests
just check                       # All quality gates (fmt, clippy, test, deny, machete, jscpd)
just lint                        # Clippy with strict warnings
just fmt                         # Auto-format
cargo build -p betcode-proto     # Rebuild proto after .proto changes
```

## Code Quality Rules

- **No `unwrap()` in production code** -- denied by Clippy (`unwrap_used = "deny"`)
- **No `panic!`, `todo!`, `dbg!`** in production code
- **Clippy pedantic + nursery** lints enabled workspace-wide
- In test code: `expect()`, `unwrap()`, `panic!()` are acceptable -- use granular `#[allow(clippy::...)]` on specific test functions
- Code duplication must stay under 0.07% (enforced by jscpd)
- No unused dependencies (cargo-machete)

## Workspace Dependency Pattern

Dependencies are defined once in the root `Cargo.toml` under `[workspace.dependencies]` and referenced from crate-level `Cargo.toml` with `{ workspace = true }`:

```toml
# Root Cargo.toml
[workspace.dependencies]
tokio = { version = "1", features = ["full"] }

# Crate Cargo.toml
[dependencies]
tokio = { workspace = true }
```

## Commit Messages

[Conventional Commits](https://www.conventionalcommits.org/) format:

```
type(scope): short description
```

Types: `feat`, `fix`, `refactor`, `test`, `docs`, `ci`, `chore`, `style`
Scopes: `daemon`, `relay`, `cli`, `proto`, `setup`, `releases`, `crypto`, `core`, `lint`, `deps`

## Proto Submodule

Protobuf definitions live in a git submodule at `proto/`. Always clone with `--recurse-submodules`. After editing `.proto` files, regenerate with `cargo build -p betcode-proto`.

## Key Patterns

- **Error handling**: `thiserror` for library errors, `anyhow` for binary error propagation
- **Async runtime**: `tokio` (full features)
- **gRPC**: `tonic` for server/client, `tonic-build` for codegen
- **Storage**: `sqlx` with SQLite, WAL mode
- **CLI**: `clap` derive for argument parsing
- **TUI**: `ratatui` + `crossterm`
- **Serialization**: `serde` + `serde_json` for NDJSON parsing

## Architecture Documentation

Key docs in `docs/architecture/`:

- `OVERVIEW.md` -- System overview, C4 diagrams, tech stack
- `DAEMON.md` -- Daemon internals, subprocess management
- `PROTOCOL.md` -- Protocol layer reference
- `ROADMAP.md` -- Implementation phases and current progress
- `CONFIG_REFERENCE.md` -- Configuration reference hub
- `SECURITY.md` -- Auth, authorization, sandboxing

## Crate Responsibilities

| Crate | Role |
|-------|------|
| `betcode-proto` | Protobuf codegen, shared gRPC types |
| `betcode-core` | Config parsing, NDJSON types, shared errors |
| `betcode-crypto` | mTLS certificate generation |
| `betcode-daemon` | Subprocess manager, session store, gRPC server |
| `betcode-cli` | clap CLI, ratatui TUI |
| `betcode-relay` | gRPC router, JWT auth, message buffer |
| `betcode-setup` | First-run setup wizard |
| `betcode-releases` | Release artifact packaging |
