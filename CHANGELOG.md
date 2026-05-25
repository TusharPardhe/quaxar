# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

## [Unreleased]

## [0.1.0] - 2025-05-25

### Added

- **Mainnet sync** — Full synchronization with XRPL mainnet, tracking validated ledgers in real-time
- **Parallel acquisition** — Multi-threaded ledger and state map acquisition for ~2x faster initial sync
- **RPC endpoints** — JSON-RPC API with parity for core methods: `server_info`, `ledger`, `account_info`, `fee`, `tx`, `account_tx`, `ledger_data`, and more
- **CLI tools** — Interactive CLI with fuzzy-search command completion, gradient logo, and rich formatted output
- **Validator key management** — Generate, rotate, sign, and revoke validator keys from the CLI
- **Prometheus metrics** — Built-in `/metrics` endpoint exposing sync progress, peer counts, RPC latency, and storage statistics
- **Structured logging** — `tracing`-based structured logs with runtime-configurable levels via CLI or RPC
- **NuDB storage** — Pure Rust NuDB implementation for node object persistence
- **P2P overlay** — TLS-encrypted peer connections with protocol handshake, message routing, and peer management
- **Doctor command** — Pre-flight diagnostics checking config, ports, disk space, and network connectivity

### Fixed

- **NuDB 48-bit overflow** — Corrected key space handling that caused data corruption on databases exceeding 2^48 entries
- **Tokio timer panics** — Resolved panic when timer futures were dropped during shutdown sequence
- **Manifest key rotation** — Fixed validator manifest signing to correctly handle ephemeral key rotation
- **Clock sync** — Corrected network time offset calculation that caused validation timestamp drift
