# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

## [Unreleased]

### Fixed

- Testnet forks 21660851, 21661277 and 21661304:
  - AccountDelete by a Delegate's delegatee removes the entry from the
    delegator's and delegatee's directories and decrements the delegator's
    OwnerCount, like rippled `DelegateSet::deleteDelegate` (it returned
    `tefBAD_LEDGER`, dropping the transaction).
  - Equal AMMWithdraw, AMMDeposit and AMMClawback compute the LP fraction as
    an STAmount quotient like rippled `divide(tokens, lptAMMBalance, ...)`,
    not a higher-precision Number quotient.
- NetworkOps no longer stalls its transaction queue. A synchronous batch
  (ledger close with held transactions, or a local submit) that saw new
  arrivals left the dispatch state `Scheduled` with no job queued, so every
  later transaction was only queued: relayed and submitted transactions were
  never applied and memory grew by about 1 GB a day. Each batch now ends at
  `None` like rippled, synchronous callers hand leftover work to a fresh
  JtBatch job, and the batch job returns if another batch is running.
- Testnet forks 21632948, 21633206, 21633429 and 21636525:
  - AMMDeposit checks the depositor's IOU and MPT funds before each send
    (`tecUNFUNDED_AMM`), not just XRP.
  - AMMWithdraw and AMMClawback return `tecINTERNAL` when the withdrawn LP
    tokens exceed the AMM's `LPTokenBalance`, like rippled.
  - IOU EscrowCreate uses rippled's `canAdd` precision check
    (`tecPRECISION_LOSS`), not just an overflow check.
- Cold sync on 2 vCPU hosts no longer deadlocks when fjall reaches 30 L0
  runs: at least 4 fjall workers, plus a watchdog that forces a flush when L0
  stays high.
- The disk-full ballast now allocates real blocks on every OS (it was a sparse
  file) and is released and re-created as free space changes.
- `online_delete = 0` or unset now disables online deletion like rippled; it
  was silently pruning to 512 ledgers.

### Changed

- Config parsing matches rippled:
  - `[ledger_history]` defaults to 256.
  - `[fetch_depth]` is read and bounds the ledgers served to peers.
  - `[compression]` and `[ledger_replay]` default off and are applied.
  - `[beta_rpc_api]` gates API version 3; an invalid `api_version` is
    rejected with `invalid_API_version`.
  - `[ssl_verify]`, `[ssl_verify_file]` and `[ssl_verify_dir]` apply to
    outbound HTTPS.
  - Per-port `send_queue_limit` is applied.
  - Numeric `[node_size]` is accepted.
  - Unknown `[features]`, malformed values and setting both reduce-relay vp
    keys fail startup.
- The RPC command-line client uses the configured admin port.

### Removed

- The `[node_db]` keys `prune_mode`, `prune_batch` and `verify_interval` are
  now fixed internals.
- The `[rpc_ip]` and `[rpc_port]` config sections are gone; use
  `--rpc_ip`/`--rpc_port`. `quaxar config` warns if either is still present.

## [0.10.0] - 2026-10-15

### Fixed

- Fixed five testnet consensus forks, each with a byte-exact sparse replay of
  the network ledger as a regression test:
  - 21534114: a FillOrKill OfferCreate now removes an offer unfunded by an
    earlier liquidity pass, comparing funds against the strand's `afView`.
  - 21540882: IOU transfers record PaymentSandbox deferred credits and
    `accountHolds` applies `balanceHookIOU` across nested sandboxes; AMM pool
    balances stay hook-free (`ammAccountHolds`).
  - 21558891: CheckCash by the issuer waives the writer's line limit before
    `fixCleanup3_4_0`, so flow may issue up to `SendMax`.
  - 21511422: an over-range unchecked `Number` mantissa is truncated when
    converted to amount parts.
  - 21485530: flow engine exceptions are contained as `tecINTERNAL` with
    rippled's exception classes at every catch site.
- Fork recovery no longer cancels the network ledger acquisition when the
  local LCL forks, and the pruned store rebuilds its dead-pending set on
  restart.
- Fixed a timer service lost wakeup on stop.
- History ledgers no longer in the cache are loaded from the store, so the
  `ledger` RPC no longer reports `lgrNotFound` inside `complete_ledgers`.
- A dead node read by an acquisition is kept by the pruned store instead of
  being deleted under the ledger being acquired; a read or write racing a
  delete batch is written back.

### Changed

- Fork recovery and restart catch-up skip every subtree shared with the
  newest claimed ledger (a rolling trusted baseline). Measured on testnet:
  catch-up to an out-of-window ledger takes 10-16 s instead of not finishing,
  with RSS under 1 GB instead of 3-5.6 GB.
- Bounded memory: `LedgerHistory` bookkeeping, the validation trie and the
  validations ledger map are swept like rippled; the journal and peer charge
  history are not kept in memory; acquisitions release durable subtrees.
- The TreeNodeCache is sized from a sampled miss-ratio curve. Set
  `QUAXAR_MRC_CACHE=0` to keep the configured size.
- Removed the NodeObject cache; node-store reads go to fjall and the OS page
  cache, as in rippled with `online_delete`.
- Snapshot format v2 stores only leaves and rebuilds inner nodes on load, with
  bulk ingestion and parallel export. **v1 snapshots are no longer loadable;
  re-export them.**
- Mismatched built and validated ledgers are logged with rippled's
  `handleMismatch` detail, including per-transaction metadata.
- Since 0.9.0 (#83-#90): rippled 3.4.1 and Oct 2026 develop sync, confidential
  MPT key rotation, the `mpt_issuances` stream, a faster RPC/WS path, hot
  restart convergence, a bounded resident ledger tree, and `account_tx`
  `ledger_index_min` from the full validated range.

### Added

- `heap-profiling` build feature (jemalloc profiling), off by default.
- `scripts/capture_sparse_replay.py` captures a network ledger as a replay
  fixture for fork investigation.
- jemalloc and miss-ratio-curve fields in `get_counts`.

## [0.9.0] - 2026-10-08

### Added

- Added a continuously pruned, fjall-backed node store as the sole node-store
  backend. Each validated ledger is claimed as an exact add/remove delta;
  reference counts and self-validating death-sequence records drive deletion
  once nodes fall more than `online_delete` ledgers behind, with incremental
  and full-window verification that halts pruning on any inconsistency.
- Added `[node_db] compaction_base_mb` (16-4096, default 256) to size the
  first leveled-compaction level of the node keyspace, bounding peak space
  amplification. Testnet and devnet default to 64.
- Added a disk-full ballast (`reserve_mb`), pruned-aware snapshot load with
  anchor adoption, and pruned-store health in `server_info` and `get_counts`.

### Changed

- Ported `rippled` Aug-Sept 2026 parity work: serialization and preflight
  hardening, peer/overlay DoS limits, lending/vault invariant relaxations and
  precision fixes, and canonical rounding for issued amounts, the NFT royalty
  cut, and the CLOB/flow strand ordering (several testnet fork fixes with
  byte-exact regression tests).
- Improved consensus and acquisition stability: validations processed
  synchronously, parallel per-session SHAMap read-apply, lock-free status
  reads, edge-triggered preferred-LCL reconciliation, no mode demotion on a
  pure forward LCL catch-up, and only trusted UNL proposals influencing
  consensus.

### Removed

- Removed the NuDB and RocksDB node-store backends, the rotating node store,
  and the associated clang/librocksdb build chain. `type = fjall` is the
  default; `NuDB` and `RocksDB` are rejected with a migration hint.

### Credits

- The pruned node-store design follows the work of **@shortthefomo** in
  [XRPLF/rippled#6549](https://github.com/XRPLF/rippled/pull/6549) (null-backend
  object-cache, priming-retry and `completeLedgers`/purge-interval fixes),
  **@sublimator**, whose design discussion and gist shaped the null-backend
  pruned approach, and **@RichardAH** and contributors in
  [Xahau/xahaud#728](https://github.com/Xahau/xahaud/pull/728) (RWDB null
  node-store and online-delete behaviour). This release is built on their ideas;
  thank you.

### Changed

- Matched consensus child-ledger application semantics: transaction state is
  based on the parent tree while sequence and close-time reads use the child
  header being built.
- Matched `rippled` transaction-map construction for Payment delivered
  metadata, CheckCash and AccountDelete delivery handoff, owner-directory
  descriptors and capacity, TicketCreate insertion, and OfferCreate
  transfer-rate quality thresholds.
- Made payment and offer strands asset-aware for MPT endpoints and books,
  including issuer capacity, authorization, freeze/lock, transfer-rate and
  owner-funding rules; restored issued/MPT CheckCash flow and amendment-gated
  FillOrKill completion semantics.
- Prevented late acquisition of a superseded validation from resurrecting
  historical trie support, and kept timer-time consensus view changes
  mode-only until serialized preferred-LCL reconciliation selects a target.
- Preserved canonical empty affected-node metadata objects selected from typed
  `NotPresent` SLE slots, and retained a signer's older acquired validation-trie
  support when removing a different, newer unacquired validation.
- Matched targetless NetworkOps demotions for zero consensus positions,
  amendment/UNL blocking, and peer counts below the configured network quorum,
  while keeping zero-threshold acquisition transport updates phase-neutral.
- Restored `ApplyStateTable` owner threading for created and deleted ledger
  entries, rejected unauthorized synthetic AMM liquidity during offer crossing,
  and serialized proposals, transaction-set completions, and consensus timers
  through one ordered owner ingress.

- Reworked current-ledger recovery around the application-owned NetworkOps
  strand, a typed acquisition coordinator, preferred-ledger retargeting,
  durable exact-identity handoff, an independent startup/recovery latch, and
  the NetworkOps-to-LedgerMaster promotion boundary.
- Preserved live acquisition traversal plans across deferred work and reuse
  verified nodes through the shared NodeFamily tree cache, fetch pack, and
  NodeStore after individual per-hash sessions finish or expire.
- Decoupled factual coordinator publication identity from the fresh
  end-consensus proof used for `tracking` to `full` promotion, while preserving
  contiguous-chain checks and updates for an already-full node.
- Restored the reference distinction between issued-currency `creditBalance`
  and spendable `accountHolds`, including canonical account orientation and
  typed zero amounts for absent trust lines.
- Matched direct OfferCreate self-cross callback ordering and quality gates so
  eligible self offers are cancelled without deleting worse-than-limit offers
  or leaking speculative strand state.
- Migrated operator-facing packages, binaries, metrics, environment variables,
  configuration paths, containers, and services from `xrpld` branding to
  Quaxar while retaining `xrpld/` as the source comparison layout and the
  legacy Docker physical-volume default as a data-safe upgrade exception.
- Validator key generation now emits XRPL node-public/node-private encodings, a
  family validation seed, RFC-1751 words, and a protected non-overwriting
  `validator-keys.json`.
- Validator token, local sign, and revocation helpers remain experimental even
  though earlier releases exposed those commands; `[validation_seed]` is the
  currently documented operational path.
- Updated operator and contributor documentation for the current architecture,
  synchronization states, validator setup, CLI, RPC, and deployment paths.
- Made AWS provisioning refuse legacy in-place state before service/data/config
  mutation and require config plus local RPC readiness before enabling the new
  service.

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
