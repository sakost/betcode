# betcode-relay

Self-hosted gRPC relay server for remote access to BetCode daemons.

## Overview

The relay enables remote access to BetCode daemons from mobile clients or other machines:

- gRPC traffic routing via reverse tunnels
- JWT authentication for clients (with token refresh)
- mTLS authentication for daemons (with cert rotation support)
- Message buffering for offline daemons (SQLite, configurable TTL and capacity)
- Certificate validation and revocation checking
- Push notifications (FCM, feature-gated behind `push-notifications`)
- Opt-in metrics (OpenTelemetry OTLP, feature-gated behind `metrics`)

## Architecture Docs

- [TOPOLOGY.md](../../docs/architecture/TOPOLOGY.md) -- Network topology, relay architecture
- [CONFIG_RELAY.md](../../docs/architecture/CONFIG_RELAY.md) -- Relay configuration
- [SECURITY.md](../../docs/architecture/SECURITY.md) -- Auth, authorization, sandboxing
