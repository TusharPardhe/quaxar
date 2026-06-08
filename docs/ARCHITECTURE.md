# Architecture

This document describes the high-level architecture of the quaxar Rust implementation.

## Crate Dependency Diagram

```
                         ┌──────────────┐
                         │  xrpld/main  │
                         └──────┬───────┘
                                │
              ┌─────────────────┼─────────────────┐
              │                 │                  │
       ┌──────▼──────┐  ┌──────▼──────┐  ┌───────▼───────┐
       │ xrpld/server│  │  xrpld/app  │  │ xrpld/metrics │
       └──────┬──────┘  └──────┬──────┘  └───────────────┘
              │                 │
       ┌──────▼──────┐         │
       │  xrpld/rpc  │         │
       └──────┬──────┘         │
              │      ┌─────────┼──────────────┐
              │      │         │              │
       ┌──────▼──────▼──┐ ┌───▼────────┐ ┌───▼──────────┐
       │  xrpld/ledger  │ │xrpld/overlay│ │xrpld/consensus│
       └──────┬─────────┘ └───┬────────┘ └───┬──────────┘
              │                │              │
       ┌──────▼────────┐      │              │
       │xrpld/nodestore│      │              │
       └──────┬────────┘      │              │
              │                │              │
       ┌──────▼────────────────▼──────────────▼──┐
       │              xrpld/tx                    │
       └──────────────────┬──────────────────────┘
                          │
       ┌──────────────────▼──────────────────────┐
       │            xrpl/protocol                 │
       ├──────────────────────────────────────────┤
       │  xrpl/shamap  │  xrpl/core  │ xrpl/basics│
       └───────────────────────────────────────────┘
```

## Crate Responsibilities

### Library Crates (`xrpl/`)

| Crate | Responsibility |
|-------|---------------|
| **protocol** | XRP Ledger wire format types, protobuf message definitions, serialization/deserialization of ledger objects and transactions. |
| **basics** | Foundation utilities: tagged integer types, time abstractions, config parsing, range sets, and common traits. |
| **shamap** | SHAMap radix trie implementation used for state maps, transaction maps, and account state trees. Supports copy-on-write, proof paths, and sync ingestion. |
| **core** | Cryptographic primitives: secp256k1/ed25519 key derivation, signing, verification, and seed generation. |
| **resource** | Load tracking and resource consumption management for rate limiting peers and RPC clients. |

### Application Crates (`xrpld/`)

| Crate | Responsibility |
|-------|---------------|
| **app** | Top-level application orchestration: ledger acquisition pipeline, state machine, peer coordination, and startup sequencing. |
| **consensus** | XRPL consensus protocol: proposal generation, validation, phase transitions, and UNL trust management. |
| **ledger** | Ledger lifecycle management: open → closed → validated transitions, ledger info, and state access. |
| **overlay** | P2P networking layer: peer discovery, connection management, message framing, and protocol handshake over TLS. |
| **nodestore** | Persistent storage backend using NuDB: node object read/write, batch operations, and cache integration. |
| **rpc** | JSON-RPC method handlers: account_info, ledger, server_info, fee, tx, and all other public/admin methods. |
| **server** | HTTP and WebSocket server: request routing, connection handling, admin vs public access control. |
| **tx** | Transaction processing: signature verification, pre-flight checks, application to ledger state, and result codes. |
| **metrics** | Prometheus metrics: counters, gauges, and histograms for sync progress, peer counts, RPC latency, and storage stats. |
| **main** | Binary entry point: CLI parsing, interactive mode, config loading, and daemon startup. |

## Key Design Decisions

### Parallel Acquisition

Unlike C++ rippled which acquires ledgers sequentially, quaxar uses a multi-threaded acquisition pipeline:

- A coordinator task dispatches ledger sequence ranges to worker threads
- Each worker independently fetches and validates state/transaction maps
- Results are assembled in-order and committed to the nodestore
- This achieves ~2x faster initial sync compared to sequential acquisition

### Tokio for Networking, Sync Threads for Consensus

- **Networking (overlay, server, RPC)**: Runs on the tokio async runtime for efficient I/O multiplexing across hundreds of peer connections
- **Consensus and ledger processing**: Runs on dedicated OS threads to avoid blocking the async runtime during CPU-intensive hash computations and state transitions
- **NuDB writes**: A dedicated writer thread with a channel-based queue to serialize disk I/O and avoid lock contention

### NuDB Storage

- Pure Rust NuDB implementation (no C++ dependency)
- Handles the 48-bit key space correctly (fixed overflow bug from initial port)
- Batch write support for efficient ledger commits
- Read-through cache with configurable size

### Structured Observability

- All logging uses `tracing` with structured fields (not string formatting)
- Spans track request lifecycle from peer message through to RPC response
- Prometheus metrics are zero-cost when not scraped
- Log levels configurable at runtime via CLI or RPC

## Data Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                          XRPL Network                                │
└────────────────────────────────┬────────────────────────────────────┘
                                 │ TCP + TLS
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Overlay (P2P)                                                       │
│  • Peer handshake & protocol negotiation                            │
│  • Message routing (proposals, validations, tx, ledger data)        │
└────────────────────────────────┬────────────────────────────────────┘
                                 │
                    ┌────────────┼────────────┐
                    ▼            ▼            ▼
            ┌─────────────┐ ┌────────┐ ┌──────────────┐
            │ Validations │ │  Tx    │ │ Ledger Data  │
            └──────┬──────┘ └───┬────┘ └──────┬───────┘
                   │            │              │
                   ▼            ▼              ▼
┌─────────────────────────────────────────────────────────────────────┐
│  App (Acquisition & Orchestration)                                   │
│  • Validate incoming data against consensus                         │
│  • Parallel ledger/state acquisition                                │
│  • Assemble complete ledgers                                        │
└────────────────────────────────┬────────────────────────────────────┘
                                 │
                    ┌────────────┼────────────┐
                    ▼            │            ▼
            ┌─────────────┐     │     ┌─────────────┐
            │  Consensus  │     │     │   Ledger    │
            │  (validate  │     │     │  (state     │
            │   & agree)  │     │     │   mgmt)     │
            └─────────────┘     │     └─────────────┘
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Nodestore (NuDB)                                                    │
│  • Persist validated ledger objects                                  │
│  • Cache hot nodes in memory                                        │
│  • Batch commit for write efficiency                                │
└─────────────────────────────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│  RPC / Server                                                        │
│  • Serve JSON-RPC queries from validated state                      │
│  • WebSocket subscriptions for real-time updates                    │
│  • Admin commands for node management                               │
└─────────────────────────────────────────────────────────────────────┘
```
