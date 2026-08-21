# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

## [Unreleased]

### Changed

- Reworked current-ledger recovery around the application-owned NetworkOps
  strand, preferred-ledger coordination, and LedgerMaster promotion flow.
- Retain and reuse incomplete SHAMaps through the shared NodeFamily, tree,
  fetch-pack, negative, full-below, and NodeStore caches.
- Migrated operator-facing packages, binaries, metrics, environment variables,
  configuration paths, containers, and services from `xrpld` branding to
  Quaxar while retaining `xrpld/` as the source comparison layout.
- Validator key generation now emits XRPL node-public/node-private encodings, a
  family validation seed, RFC-1751 words, and a protected non-overwriting
  `validator-keys.json`.
- Updated operator and contributor documentation for the current architecture,
  synchronization states, validator setup, CLI, RPC, and deployment paths.

## [0.3.1] - 2026-06-26

### Added

- **Consensus engine redesign** — Complete rewrite of the consensus participation lifecycle matching rippled's architecture. Single 1s heartbeat thread, channel-driven validation processor, immediate ledger promotion, parking_lot::Mutex-based ConsensusDriver
- **Channel-driven validation processing** — Dedicated thread blocks on an mpsc channel and wakes when validations arrive from peers
- **ConsensusDriver** — New unified consensus entry point with peer count gate, canValidateSeq monotonicity enforcement, and spawn_heartbeat API

### Changed

- **Timer architecture** — Replaced 200ms polling timer with single 1s heartbeat thread (ledgerGRANULARITY). Removed maybe_tick_consensus macro and inline ticks that caused 5x speed bug
- **get_prev_ledger** — Simplified to pure validation trie query (no peer voting). Peer voting moved to endConsensus/checkLastClosedLedger where it belongs
- **Validation emission** — Added canValidateSeq (strictly increasing), !consensusFail, and proposers>0 guards. Prevents stale fork validations from polluting the trie
- **acquire_ledger** — Relaxed immutability check, added store_consensus_ledger for immediate LedgerHistory availability

### Fixed

- **Consensus convergence** — Nodes now converge reliably across all configurations (5q, 3r+2q, 4r+1q, 2r+3q)
- **5x timer speed bug** — tick_fixed(1s) was called every 200ms, causing consensus timers to run at 5x real speed
- **Validation trie pollution** — Solo-built ledger validations no longer enter the trie, preventing spurious wrong-ledger detection
- **Ledger promotion lag** — Validated-ledger advancement was moved closer to the reference acceptance sequence

## [0.2.0] - 2026-06-08

### Added

- **Snapshot export** — Live background snapshot export via `export_snapshot` while the node continues running
- **Snapshot import** — Bulk import mode with a pre-allocated hash table, skipped existence checks, and deferred bucket splits
- **export-snapshot CLI** — Triggers export via RPC to the running node, returns immediately with progress in logs
- **load-snapshot CLI** — Offline import with sharded NuDB path resolution and crash recovery markers
- **Dynamic log level** — Runtime log level changes via `log_level` RPC using tracing-subscriber reload handle. Partition-scoped and input-validated
- **Snapshot scheduler** — Configurable automatic exports every N ledgers on a background thread
- **Windows installer** — PowerShell installer script for cross-platform support
- **Binary rename** — Renamed binary from `xrpld` to `quaxar`

### Fixed

- **Peer sampling panic** — Off-by-one in `sample_peer_ids` caused `swap_remove` panic when `rand_int_to` (inclusive) returned `len` instead of `len-1`. Reported by [@donovanihide](https://github.com/donovanihide)
- **Inbound peer connections** — Peer listener failed to bind because TLS identity was not generated for non-secure ports. Now always generates anonymous TLS for the peer protocol
- **CLI RPC URL resolution** — CLI connected to `0.0.0.0` instead of `127.0.0.1` when config bound to all interfaces, causing admin permission failures
- **RPC handler delegation** — `export_snapshot` and `log_level_set` were not delegated from `ApplicationServerInfo` to `ApplicationRoot`, returning "Not implemented"
- **Export OOM** — Snapshot export ran synchronously on RPC thread, causing OOM on 32 GB machines. Now spawns a background thread
- **NuDB for_each memory** — `scan_indexed_records` loaded all 26.5M records into a Vec before iterating. Now streams one record at a time via bucket traversal

### Performance

- **Bulk import mode** — NuDB `bulk_import_start/finish` bypasses existence checks, burst checkpoints, and bucket splits during snapshot loading
- **Streaming for_each** — Avoids collecting every record before export by processing records during bucket iteration
- **Pre-allocated hash table** — Bulk import pre-creates buckets based on estimated node count, avoiding 26.5M incremental split operations

### Changed

- **Dockerfile** — Uses `debian:bookworm-slim` for both build and runtime stages to avoid glibc mismatch. Statically links RocksDB
- **install.sh** — RPC and WebSocket bind to `0.0.0.0` by default with `admin = 127.0.0.1` so admin commands work out of the box

## [0.1.0] - 2025-05-25

### Added

- **Mainnet sync** — Full synchronization with XRPL mainnet, tracking validated ledgers in real-time
- **Parallel acquisition** — Multi-threaded ledger and state-map acquisition
- **RPC endpoints** — JSON-RPC API with parity for core methods: `server_info`, `ledger`, `account_info`, `fee`, `tx`, `account_tx`, `ledger_data`, and more
- **CLI tools** — Interactive CLI with fuzzy-search command completion, gradient logo, and rich formatted output
- **Validator key management** — Generate, rotate, sign, and revoke validator keys from the CLI
- **Metrics instrumentation** — Recorder hooks for sync, peer, RPC, and storage activity
- **Structured logging** — `tracing`-based structured logs with runtime-configurable levels via CLI or RPC
- **NuDB storage** — Pure Rust NuDB implementation for node object persistence
- **P2P overlay** — TLS-encrypted peer connections with protocol handshake, message routing, and peer management
- **Doctor command** — Pre-flight diagnostics checking config, ports, disk space, and network connectivity

### Fixed

- **NuDB 48-bit overflow** — Corrected key space handling that caused data corruption on databases exceeding 2^48 entries
- **Tokio timer panics** — Resolved panic when timer futures were dropped during shutdown sequence
- **Manifest key rotation** — Fixed validator manifest signing to correctly handle ephemeral key rotation
- **Clock sync** — Corrected network time offset calculation that caused validation timestamp drift
