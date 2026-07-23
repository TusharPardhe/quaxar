# Ledger Acquisition & Storage Redesign Plan

Status: proposed, not yet implemented
Scope: `SharedInboundLedgers`/`InboundLedgers` acquisition workers, NuDB write/read path,
RPC ledger lookup, consensus-driven acquisition triggers
Non-goals: changing wire protocol, changing consensus algorithm, changing RPC method surface

Supporting documents (read first, this plan assumes their findings):
- `RAM_CACHE_AUDIT.md` — full inventory of every in-RAM cache today
- `rpc_read_path_audit.md` — how RPC/tx-submit currently select and read ledgers

## 0. Why this redesign, in one paragraph

Investigation (including a live testnet trace) found the acquisition worker downloading
and durably writing state nodes for 10+ minutes without ever completing a single ledger:
`good`/`write_count` climbed past 12 million for a tree that should hold a few hundred
thousand entries, while NuDB's actual unique key count (`node_writes` in `get_counts`)
stayed two orders of magnitude lower. Root cause chain, confirmed against source:

1. `good`/`write_count` is incremented in the SHAMap sync layer the instant a node is
   accepted into the in-memory tree (`SHAMapAddNode::useful()`), not when it is durably
   stored. The actual NuDB write happens later, asynchronously, on a single writer thread
   draining an **unbounded** `mpsc` channel.
2. The dedup mechanism that should stop the same hash being processed/counted twice
   (`should_store_hash` / the `shared_stored` `KeyCache`) is fully implemented but **never
   called** from any production call site — confirmed by grep and by a test whose own name
   documents the bypass.
3. NuDB's own last-resort dedup (`find_bucket_entry` pre-check in `store()`) has a TOCTOU
   race (checked outside `store_mutex`, inserted inside) and is skipped entirely during
   bulk import.
4. Two independent acquisition systems (`SharedInboundLedgers` for `--start`/consensus,
   `InboundLedgers` in `main.rs` for catchup) share the node store but keep separate
   registries, dedup caches, and `pending_writes` maps — increasing surface area for the
   same class of bug and making the RAM budget impossible to reason about globally.
5. Peer connections churn every 10–70 seconds because quaxar never sends a proactive
   keepalive ping (rippled's `kPeerTimerInterval=60s` self-rearming ping has no quaxar
   equivalent — the constant exists, nothing calls it), so every acquisition target
   restarts its peer set mid-flight repeatedly.

None of this is fixable with another targeted patch — the last 20 commits on the debug
branch already tried that, several were reverted, and the underlying design (counting
before durability, per-target ad-hoc caches, two parallel worker systems) is what keeps
producing new symptoms. This plan replaces the acquisition and storage-visibility layer
with a single coherent design, while explicitly preserving every rippled behavioral
contract identified during the reference audit.

## 1. Design goals, in priority order

1. **Correctness first**: an acquisition must provably terminate (complete or fail) in
   bounded time given responsive peers, with no possibility of counting a node as
   acquired before it is durably queryable.
2. **NuDB is the only source of truth.** Every cache in the system is allowed to be
   wrong, empty, or evicted at any time without changing the answer to "do we have X" —
   only slower. No RAM structure may be required for correctness except transient,
   necessarily-in-flight state (open ledger deltas, actively-downloading trees).
3. **Bounded, accountable RAM.** One global budget, one place that enforces it, sized by
   `node_size` the way rippled does, with no unbounded structure anywhere (the current
   `pending_writes` map is the one existing violation).
4. **Real parallel acquisition.** Multiple peers should fetch disjoint parts of the same
   state tree concurrently, coordinated so no two workers redundantly fetch the same
   subtree, with the result written incrementally to NuDB rather than only at the end.
5. **rippled parity.** Every constant, threshold, and retry/backoff behavior catalogued
   in the reference audit must have a named equivalent in the new design (a parity table
   is included below), not just "similar" behavior.
6. **One acquisition system, not two.** `SharedInboundLedgers` and `InboundLedgers`
   collapse into a single service used by both consensus and catchup.

## 2. Target architecture overview

```
                         ┌─────────────────────────┐
 consensus loop  ───────▶│                         │
 (bootstrap.rs)          │   AcquisitionService    │◀────── catchup loop (main.rs)
                         │   (single instance,     │
 RPC "acquire missing"──▶│    replaces both        │
 (new: on-demand path)   │    SharedInboundLedgers │
                         │    and InboundLedgers)   │
                         └───────────┬─────────────┘
                                     │ owns
                     ┌───────────────┼────────────────────┐
                     ▼               ▼                    ▼
             TargetRegistry   PeerFetchRouter      NodeStoreWriter
          (one entry per      (subtree-partitioned  (single, bounded,
           ledger hash in     fan-out across peers,  backpressured queue;
           flight; bounded    liveness-aware)        "good" only counted
           by MAX_TARGETS)                           after ack from here)
                     │
                     ▼
            per-target AcquisitionState
          (rippled InboundLedger-equivalent
           state machine: have_header/state/tx,
           timeouts, by-hash fallback — pure,
           no I/O, fully unit-testable)
```

Key structural change from today: **the state machine that decides what to request next
is separated from the code that owns peer sockets and the code that owns NuDB writes.**
Today `InboundLedgerLocal` (good, mostly rippled-parity logic) is entangled with
worker-thread-local mpsc plumbing, per-tick cache construction, and direct write-channel
access. The new design keeps `InboundLedgerLocal`'s logic almost as-is (it's the right
shape) but drives it from a scheduler that can fan a single target across N peers for
disjoint subtrees, and that never reports progress until the write layer acks.

## 3. Component-by-component design

### 3.1 `AcquisitionService` (replaces `SharedInboundLedgers` + `InboundLedgers`)

- One process-wide instance, constructed once at startup, handed to both the consensus
  driver and the catchup loop. Both call the same `request(hash, seq, reason)` API;
  `reason` is `Consensus | History | RpcOnDemand` (extensible, but today's two call
  sites collapse to one enum instead of two whole subsystems).
- `MAX_TARGETS` concurrent in-flight acquisitions (config-driven, default derived from
  `node_size`, replacing the current hardcoded `MAX_CONCURRENT_ACQUISITIONS = 8`).
  Eviction policy unchanged in spirit (evict lowest-seq, never evict recently-touched)
  but now a single, testable function instead of inline logic in `acquire()`.
- Replaces the `has_worker` hard gate. That gate exists today because two concurrent
  workers fight over I/O bandwidth and share mutable caches unsafely. The new design
  allows genuine concurrency (see 3.3) because subtree partitioning removes the reason
  the gate was needed — concurrent fetches for *different* ledgers no longer contend for
  the same peer-request budget blindly; the `PeerFetchRouter` is peer-liveness-aware and
  rate-limits per peer, not per acquisition.
- One `TargetRegistry`: `HashMap<Uint256, AcquisitionHandle>`, bounded by `MAX_TARGETS`.
  Each handle owns exactly one `AcquisitionState` (3.2) and is the single place
  `route_response()` looks up — no more "fallback to first active worker" heuristic
  (`shared_inbound_ledgers.rs`'s `route_response` fallback-to-any-worker path is a latent
  correctness hazard: a response can be misattributed to the wrong acquisition if the
  intended target's registry entry is momentarily absent).

### 3.2 `AcquisitionState` (evolves `InboundLedgerLocal`)

Kept almost unchanged — the rippled-parity audit confirms this state machine's shape
(header → state+tx trees → completion; timeout/retry/by-hash-fallback semantics) is
already faithfully ported. Changes:

- **Remove all I/O and cache-construction from this type.** It becomes pure: given
  "here is what I have," "here is what just arrived," "here is the current time,"
  produce "here is what to request next" and "here is what to persist." No thread
  spawning, no channel access, no `FullBelowCacheImpl::new()` inside its methods.
- **Split "accepted into tree" from "durably stored."** Today `SHAMapAddNode::useful()`
  is both. New: accepting a node into the in-memory sync tree returns
  `Accepted { hash, data }` without touching any counter. The caller (the scheduler)
  hands `Accepted` entries to the `NodeStoreWriter` and only increments the
  publicly-visible progress counter (`good`, used for peer scoring and stall detection)
  when the writer acks. This directly closes the bug that caused 12M-good/~460K-write
  divergence.
- **Dedup at acceptance, not at storage.** Before calling `add_known_node`, check a
  request-scoped `requested: HashSet<Uint256>` (already-known-in-tree nodes are
  naturally rejected by `add_known_node_impl`'s tree-structural checks, but duplicate
  *wire deliveries* of the same hash from multiple peers should short-circuit before
  even attempting the tree insert). This finally wires up the intent behind
  `should_store_hash` — but as a call inside `AcquisitionState`, not a separate cache
  that nothing calls.

### 3.3 `PeerFetchRouter` — real parallel/partitioned acquisition

This is the new capability, not present today in any form:

- For the account-state tree, once the root and first level of children are known
  (typically within the first response), the router assigns **disjoint branches**
  (nibble ranges at a fixed depth, e.g. the 16 top-level branches, subdividing further
  if a branch peer is slow) to different peers. Each peer receives a `TMGetLedger` whose
  `node_ids` are drawn only from its assigned branch's missing set.
- This is a direct extension of rippled's existing `getMissingNodes(max, filter)` +
  `REQ_NODES_REPLY` fan-out (already ported), adding a partition key so two peers are
  never asked for overlapping missing-node sets in the same tick. Today's fan-out
  (`trigger_with_family` called per `triggered_peer_ids`) already sends different
  *requests* to different peers, but doesn't prevent overlap — two peers can be asked
  for the same missing hash in the same or adjacent ticks. Partitioning removes that.
- Peer liveness: implements the rippled `kPeerTimerInterval` contract (3.6) so peers
  assigned a branch don't silently die mid-fetch without the router noticing within one
  timer tick and reassigning that branch.
- Backpressure-aware: a peer whose outbound send queue is growing (mirroring rippled's
  `kDropSendQueue`/`kSendqIntervals` checks, which live on the *peer* side today and
  should also inform the *router's* peer selection, not just the peer's own self-defense)
  is deprioritized for new branch assignments before it's dropped outright.

### 3.4 `NodeStoreWriter` — the only writer, bounded, ack-based

- One instance for the whole process (not one per worker as today, not two independent
  implementations as today between `shared_inbound_ledgers.rs` and `main.rs`).
- **Bounded queue** (`crossbeam::channel::bounded` or equivalent), sized from `node_size`
  (e.g. `write_queue_depth`: tiny=4096 … huge=65536). `send()` blocks (with a short
  timeout + backoff signal to the caller) when full — this is the mechanism that makes
  "counting ahead of durability" structurally impossible: a producer cannot get more than
  `write_queue_depth` nodes ahead of the writer thread, and progress accounting only
  happens post-ack, not post-send.
- **Per-write ack channel** (oneshot or a returned `Future`/callback) so callers — i.e.
  `AcquisitionState` via the scheduler — can await durability for the specific hashes
  that matter (e.g. before reporting `have_state = true`, confirm the last-written batch
  acked) without blocking the whole pipeline on every single write.
- **Dedup fixed at the source of truth.** `NuDbBackend::store()`'s `find_bucket_entry`
  check moves *inside* `store_mutex` (closes the TOCTOU race identified in the audit).
  Bulk import keeps its fast path but gains a documented, explicit invariant: bulk-loaded
  snapshots must not contain intra-snapshot duplicate hashes (the importer already knows
  the source is a coherent snapshot, so this is a validation-at-import-time check, not a
  runtime cost on the hot path).
- **No more `pending_writes` shadow map.** Because writes are now acked, and the reader
  path (3.5) can await the same ack, there's no need for a separate "read your own write
  before it's in NuDB" RAM structure. This removes the one unbounded, correctness-load-
  bearing cache identified in the RAM audit.

### 3.5 Unified read path — NuDB as sole source of truth

Per the RPC audit, node-level reads already have NuDB fallback (`node_fetcher` pattern);
the real gap is **ledger-level** lookup for RPC. Fix, matching rippled's exact two-tier
`LedgerMaster::getLedgerByHash` contract from the reference audit:

- `LedgerHistory` (today's bounded `TaggedCache`) keeps its role as a fast-path cache —
  it is allowed to be wrong/empty at any time.
- On a cache miss, fall through to the **already-implemented** `LedgerInfoProvider` +
  `Ledger::load_by_hash/index_with_provider` path (currently wired only to the peer
  overlay's `GetLedger` handler) — thread it into `app_server_info.rs`'s `get_ledger_obj()`
  so RPC gets the same NuDB-backed rebuild the overlay already benefits from.
- Add the single-slot `closed_ledger`/`validated_ledger` fallback exactly as rippled does
  (history cache miss → NuDB rebuild miss → check the live in-memory slot by hash/seq)
  for the narrow window where a just-closed ledger hasn't been persisted as "history" yet.
- Net effect: **every** RPC path (current state, historical ledger, account lookup, tx
  submission's base-ledger selection) has a defined, tested fallback to NuDB. The 3
  `ArcSwapOption<Ledger>` slots remain — they are correctly RAM-only, since "which ledger
  is currently open/closed/validated" is inherently live state, not historical data — but
  they stop being a hard dependency for anything past "what's current right now."

### 3.6 Peer liveness — close the churn gap

Direct port of the rippled contract extracted in the reference audit, Section 7:

- Add a per-peer 60s self-rearming timer (`kPeerTimerInterval`) that, on each tick:
  1. Checks consecutive-large-send-queue count (`kSendqIntervals = 4`, `kTargetSendQueue`
     threshold) → disconnect if exceeded.
  2. Checks outbound-peer tracking staleness (`Diverged` > 5 min / `Unknown` > 10 min) →
     disconnect if exceeded.
  3. Checks for an outstanding, unanswered PING from the *previous* tick → disconnect on
     timeout.
  4. Otherwise sends a fresh `TMPing`, records the sequence, re-arms.
- This is new code (quaxar currently only replies to pings, never initiates), but it's a
  bounded, well-specified addition — the constant CHECK_IDLE_PEERS already imported
  everywhere just needs its call site.
- Directly reduces the churn observed in testnet tracing (10–70s connection lifetimes),
  which today forces `first_add_peers` to re-run and duplicate in-flight requests every
  time a peer dies mid-fetch.

### 3.7 RAM budget — single, accountable, `node_size`-driven

Consolidate the caches catalogued in `RAM_CACHE_AUDIT.md`:

| Today (11 structures, 3+ independent TreeNodeCache instances) | Redesign |
|---|---|
| Per-subsystem `TreeNodeCache` (acquisition, app/RPC, consensus-fallback — 3 separate instances) | **One** process-wide `TreeNodeCache`, sized by `node_size`, shared by acquisition, RPC, and consensus. No subsystem constructs its own. |
| Per-worker `FullBelowCacheImpl` (fresh instance per acquisition target) + one global instance | **One** global `FullBelowCache`, generation-bump semantics exactly as rippled (clear() bumps generation, invalidating stale marks) — the per-worker isolation "fix" is no longer needed once there's one target-partitioned scheduler instead of racing independent workers. |
| Two independent `FetchPackCache` instances (main.rs, bootstrap.rs fallback) | **One**, shared. |
| Two independent write-dedup `KeyCache` instances ("acq-write-dedup", "shared-acq-dedup") + one fallback | **One**, and actually wired to the dedup call site (3.2) instead of being unused. |
| `pending_writes: HashMap` — unbounded, correctness-critical | **Removed** (3.4 — ack-based writer makes it unnecessary). |
| `LedgerHistory` TaggedCache | Unchanged role, gains NuDB fallback (3.5). |

Net result: total RAM ceiling becomes `f(node_size)` computed once, in one place,
matching rippled's `SizedItem` table shape (already partially ported — extend
`NodeSizeResourceProfile`/`CatchupResourceProfile` to be the *only* sizing source,
consumed once by the one-instance-per-cache-type structure above). Every cache is a
pure accelerator; every miss falls to NuDB; nothing is required to be resident for
correctness except in-flight write-queue entries bounded by `write_queue_depth` (3.4)
and the live open/closed/validated ledger slots (3.5), which are inherent, not
optimizable away.

## 4. rippled parity checklist

Every item below must have a named, tested equivalent before this redesign is considered
complete. Source: reference audit (rippled files/lines cited there).

| Contract | rippled value | quaxar today | Redesign target |
|---|---|---|---|
| Max timeouts before permanent failure | `kLedgerTimeoutRetriesMax = 6` | `INBOUND_LEDGER_TIMEOUT_RETRIES_MAX = 6` (matches) | Keep, move to `AcquisitionState` |
| Timeouts before by-hash fallback | `kLedgerBecomeAggressiveThreshold = 4` | `INBOUND_LEDGER_BECOME_AGGRESSIVE = 4` (matches) | Keep |
| Missing-nodes-per-scan cap | `kMissingNodesFind = 256` | `MISSING_NODES_FIND = 256` (matches), plus a quaxar-only `_COLD_START = 1024` | Keep both; document the cold-start extension as an intentional quaxar addition for parallel fan-out, not a parity deviation |
| Nodes per request (reply-triggered / other) | `kReqNodesReply=128` / `kReqNodes=12` | matches | Keep |
| Per-attempt timer | `kLedgerAcquireTimeout = 3000ms` | 3s tick (matches) | Keep |
| `getLedgerByHash` two-tier lookup | history cache → `closedLedger_` single-slot fallback | history cache only in RPC path (gap) | Implement per 3.5 |
| `shouldAcquire` retention gate | current/future OR within `ledger_history` window OR `>= minimumOnline` | present in history-backfill path — verify against this exact 3-clause form during implementation | Verify + align exactly |
| `fullBelowCache` generation contract | generation captured once per scan; node's full-below mark only valid for that generation | correctly implemented, but undermined by per-tick fresh cache instances | Fix per 3.7 (one global cache) |
| NodeStore positive/negative cache (`Dummy` sentinel) | absent-hash negative caching, `canonicalize` only overwrites `Dummy` | not implemented in quaxar's `DatabaseNodeImp` equivalent | Add — closes repeated-miss re-fetch cost for genuinely-absent hashes (e.g. pruned history) |
| `SizedItem` table (`TreeCacheSize`, `LedgerFetch`, etc.) | full 5-tier table | partially ported (`NodeSizeResourceProfile`) | Extend to be the single sizing source (3.7) |
| Peer keepalive ping / disconnect order | 60s self-rearming timer; sendq→tracking→ping-timeout order | ping never initiated | Implement per 3.6, exact order preserved |
| Peer `isHighLatency` → query depth 2 | 300ms threshold | implemented (`ReplyHighLatency` trigger) | Keep |

## 5. Phased implementation plan

Each phase ends with a mergeable, testable state — no phase depends on a later phase's
code existing, so this can ship incrementally without a long-lived feature branch.

### Phase 1 — Close the counting-before-durability bug (highest priority, smallest diff)

Goal: stop the infinite-loop non-convergence without yet touching the broader
architecture. This alone should make single-peer acquisition converge correctly.

- Move the `good`/progress counter increment from `SHAMapAddNode::useful()` time to
  post-write-ack time. Minimal version: keep `pending_writes` for now, but have the
  progress counter read off `write_count` (post-NuDB-store) instead of sync-layer
  acceptance.
- Wire `should_store_hash`/`shared_stored` into the actual call path (`got_node` in
  `account_state_sf.rs`/`transaction_state_sf.rs`) so duplicate wire deliveries are
  rejected before a redundant tree-insert + write attempt.
- Fix the `NuDbBackend::store()` TOCTOU by moving `find_bucket_entry` inside `store_mutex`.
- Tests: unit test reproducing the exact bug (feed the same node hash twice, assert
  `good`/write count increments once, not twice); integration test against a local
  two-node setup (quaxar ↔ quaxar or quaxar ↔ rippled if available) asserting a full
  ledger acquisition completes and `have_state` flips true within a bounded number of
  ticks.

### Phase 2 — Peer keepalive (unblocks reliable multi-minute acquisitions)

- Implement the 60s ping timer per Section 3.6, exact disconnect-order parity.
- Tests: simulate a peer that never responds to ping → assert disconnect at the correct
  tick; simulate normal ping/pong → assert connection survives indefinitely in a
  soak test (e.g. 30 minutes, connection count stays stable, zero unexplained churn).

### Phase 3 — Unify the two acquisition systems

- Introduce `AcquisitionService` as described in 3.1, initially as a thin wrapper that
  both `bootstrap.rs` and `main.rs` call into, internally still using much of today's
  `run_acquisition_worker` logic but with **one** registry, **one** dedup cache, **one**
  `pending_writes`-equivalent (or none, if Phase 1's ack-based counting already removed
  the need), **one** `NodeStoreWriter`.
- Remove `has_worker` hard gate; replace with `MAX_TARGETS` bound (Section 3.1).
- Tests: consensus-triggered and catchup-triggered acquisitions for different ledgers
  run concurrently without contention bugs; RAM usage under concurrent load stays within
  the single documented budget (Section 3.7) — add an integration test that samples RSS
  before/after a multi-target acquisition burst and asserts it returns to baseline.

### Phase 4 — `NodeStoreWriter` with ack + bounded queue

- Replace the two independent writer-thread implementations with one, bounded, ack-based
  (Section 3.4).
- Remove `pending_writes` entirely once ack-based read-your-own-write is proven (readers
  await the ack for hashes they just wrote instead of checking a shadow map).
- Tests: backpressure test (flood the writer, assert producers block/slow rather than
  the queue growing unboundedly); crash-recovery test (kill the writer mid-batch, assert
  NuDB's log-based recovery — already implemented per the I/O audit — restores a
  consistent state on restart).

### Phase 5 — Subtree-partitioned parallel fetch

- Implement `PeerFetchRouter` branch partitioning (Section 3.3).
- Tests: multi-peer simulation asserting no two peers are ever sent overlapping missing-
  hash sets in the same tick; failure-injection test (kill one assigned peer mid-fetch)
  asserting its branch is reassigned within one timer tick and the ledger still completes.
- Benchmark: time-to-sync for a single large ledger with 1 peer vs. 3 peers, on the same
  test fixture, to quantify the parallel-acquisition win.

### Phase 6 — Unified RAM budget

- Consolidate all caches to single process-wide instances per Section 3.7.
- Extend `node_size` sizing to cover every remaining cache from the current 11-structure
  inventory.
- Tests: RSS ceiling test per `node_size` tier (tiny/small/medium/large/huge), run under
  sustained synthetic load, asserting peak RSS stays under a documented bound per tier.

### Phase 7 — RPC/tx-submission NuDB fallback completion

- Wire `LedgerInfoProvider` into `app_server_info.rs`'s `get_ledger_obj()` per Section 3.5.
- Add the rippled-parity single-slot `closed_ledger`/`validated_ledger` fallback after
  the NuDB rebuild attempt.
- Tests: request a historical ledger old enough to have aged out of `LedgerHistory`'s
  cache but present in NuDB → assert RPC returns it instead of `LedgerNotFound`; restart
  the node (cold cache) and immediately serve an RPC request for a recent ledger → assert
  it resolves via NuDB without waiting for a fresh acquisition.

### Phase 8 — Testnet validation & soak

- Deploy to the existing testnet server, same methodology used during debugging (wire-
  level tracing, `get_counts`/`db-stats` comparison, disk-growth-vs-good-counter
  correlation) — but now to *confirm* convergence rather than diagnose non-convergence.
- Success criteria: `have_state` and `have_transactions` both reach `true` for a full
  cold-start acquisition within a bounded time budget (define target, e.g. under 5
  minutes for a medium node_size on the current testnet peer set); `good` counter tracks
  `node_writes` (NuDB unique key count) within a small, explained delta (in-flight queue
  depth only, no unbounded divergence); peer connections survive multi-hour soak without
  unexplained churn; RSS stays within the Phase 6 budget throughout.

## 6. Risks and open decisions

- **Subtree partitioning granularity** (Phase 5): fixed top-level 16-way split is simple
  but may be too coarse if one branch is much larger than others (XRPL account
  distribution is not uniform across the top nibble). May need adaptive re-splitting of
  a slow branch into sub-branches assigned to additional peers. Flag for design review
  before Phase 5 implementation, not blocking earlier phases.
- **Ack latency vs. throughput** (Phase 4): a fully synchronous ack-per-write would hurt
  throughput; batching acks (ack per N writes or per T milliseconds) preserves the
  bounded-queue guarantee while keeping write throughput close to today's. Needs a
  benchmark during Phase 4, not a decision made in the abstract now.
- **Bulk import dedup invariant** (Section 3.4): moving the "no intra-snapshot duplicate
  hashes" assumption from implicit to documented is safe for snapshots produced by
  quaxar's own export, but any external/third-party snapshot source would need the same
  invariant verified or the bulk-import fast path re-evaluated.
- **Backward compatibility during Phase 3**: unifying the two acquisition systems touches
  both the `--start` consensus path and the standalone catchup path; each phase should be
  validated on both startup modes before merging, not just the mode most recently tested.

## 7. Cross-references

- Full current-state RAM inventory: `RAM_CACHE_AUDIT.md`
- Full current-state RPC/tx-submission read path: `rpc_read_path_audit.md`
- rippled reference contracts (constants, thresholds, lookup semantics): Section 4 above,
  sourced from `InboundLedger.cpp`, `InboundLedgers.cpp`, `LedgerMaster.cpp`,
  `SHAMapNodeID.h/.cpp`, `SHAMapSync.cpp`, `FullBelowCache.h`, `DatabaseNodeImp.h/.cpp`,
  `PeerImp.cpp`, `Tuning.h`, `Config.cpp` in the rippled reference tree.
