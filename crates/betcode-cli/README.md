# betcode-cli

Terminal client for BetCode with a ratatui TUI.

## Overview

The CLI provides a terminal interface for interacting with the BetCode daemon:

- `clap`-based command parsing (chat, session management, daemon control)
- `ratatui` TUI with streaming markdown output
- Permission prompt dialogs
- Headless mode for scripted usage (`-p` with text output)
- Daemon certificate management (`betcode daemon rotate-cert`)
- Subagent orchestration (`betcode subagent spawn/list/cancel/watch`)
- Machine management (`betcode machine register/list/switch/status`)
- GitLab integration (`betcode gitlab mr/pipeline/issue list/get`)
- Relay authentication (`betcode auth login/logout/status`)

## Architecture Docs

- [CLIENTS.md](../../docs/architecture/CLIENTS.md) -- CLI and Flutter client architecture
- [CONFIG_CLIENTS.md](../../docs/architecture/CONFIG_CLIENTS.md) -- Client configuration
