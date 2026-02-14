# betcode-crypto

mTLS certificate generation and management for BetCode.

## Overview

This crate handles the cryptographic operations needed for secure communication
between BetCode components:

- mTLS certificate generation (CA, client, server)
- Certificate signing request (CSR) creation
- Certificate validation and chain verification

## Architecture Docs

- [SECURITY.md](../../docs/architecture/SECURITY.md) -- Auth, authorization, sandboxing
