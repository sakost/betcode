# betcode-daemon

The BetCode daemon binary -- manages Claude Code subprocesses and serves the gRPC API.

## Overview

The daemon is the core of BetCode. It:

- Spawns Claude Code as subprocesses with `--output-format stream-json`
- Bridges NDJSON events to gRPC streams
- Multiplexes sessions across multiple connected clients
- Manages git worktrees and session persistence (SQLite)
- Serves a local gRPC API (Unix socket)

## Architecture Docs

- [DAEMON.md](../../docs/architecture/DAEMON.md) -- Daemon internals, subprocess management
- [SCHEMAS.md](../../docs/architecture/SCHEMAS.md) -- SQLite schema designs
- [CONFIG_DAEMON.md](../../docs/architecture/CONFIG_DAEMON.md) -- Daemon configuration
