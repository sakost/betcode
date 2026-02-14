# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0-alpha.1] - 2026-02-14

Initial pre-release.

### Added

- CLI terminal UI for interacting with Claude Code through the daemon
- Daemon subprocess manager with gRPC server, SQLite storage, session multiplexing
- Relay server with JWT authentication, tunnel management, gRPC routing
- E2E encryption (X25519 + ChaCha20-Poly1305) between clients and daemons
- Proto definitions for gRPC API shared with mobile app
- Binary download/redirect server (betcode-releases) with platform detection
- Deployment setup tool (betcode-setup) for relay and releases servers
- Caddy reverse proxy support for automatic HTTPS
- Cross-platform release workflow (Linux, macOS, Windows)

### CI

- Lint pipeline: Rustfmt, Clippy, cargo-deny, cargo-machete, jscpd
- Pre-commit hooks via cargo-husky
- Release-plz for automated changelog and version management

### Documentation

- Architecture documentation (42 documents)
- Contributing guide with development setup
- Release policy and versioning strategy
- Third-party license attribution
