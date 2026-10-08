# Fjall pruned node store: design and staged implementation plan

Status: proposed · Branch: `feat/fjall-pruned-nodestore` · Base: `sync/rippled-aug-sept-2026` (`b8d3e196`)

## 1. Goal

Replace the NuDB two-generation rotation (`online_delete` via `DatabaseRotating`)
with a single **fjall** LSM database that prunes old SHAMap nodes continuously,
using a **stale-node index plus reference counts** (Jellyfish Merkle Tree /
Aptos `StaleNodeIndex` technique). When the migration is complete, NuDB,
RocksDB, the rotation machinery and the RocksDB native build chain
(`librocksdb-sys`, `bindgen`, `clang-sys`, libclang, `librocksdb-dev`) are removed.

Non-goals: any change to ledger hashes, SHAMap layout, transaction results,
RPC output or the peer protocol. Only how node bytes sit on our disk changes.

### 1.1 Hard constraints

| # | Constraint | How the design meets it |
|---|---|---|
| C1 | Consensus untouched | Store is below SHAMap; nothing hashed differently. |
| C2 | Fetch by hash stays | `NODES` keyspace is keyed by the 32-byte node hash. |
| C3 | Crash safe | Every claim and prune step is one atomic fjall batch plus a META cursor. |
| C4 | Never delete a live node | Single writer, the three safety rules (§5), conservative death seqs, verify. |
| C5 | No regression in sync, publish or fork rate | Gated soak against a NuDB node on testnet (Stage 5). |

### 1.2 Research basis

- Jellyfish Merkle Tree (Diem), designed for LSM-tree key-value storage:
  <https://developers.diem.com/docs/technical-papers/jellyfish-merkle-tree-paper/>
- Aptos `StaleNodeIndexSchema` (aptos-core `storage/aptosdb/src/schema/stale_node_index`):
  key = `stale_since_version` (BE u64) ‖ `node_key`, empty value, range-scanned by the pruner.
- fjall 3.1 (crates.io 3.1.12, 2026-10-03): keyspaces with cross-keyspace atomic
  batches, range/prefix iteration, per-key remove, compaction filters (3.1),
  xxh3 block checksums, partitioned filters, stable disk format promise, MSRV 1.90.
  Gaps today: no merge operator, no range delete, SingleDelete experimental.
- Alternatives evaluated and rejected: RocksDB (C++ build, already being removed),
  SurrealKV (0.x format), redb/LMDB/MDBX (B-tree write pattern, mmap RSS),
  coordinode-lsm-tree, mace, agatedb, sled (immature, abandoned or unsafe fsync).

### 1.3 Why refcounts are needed but rarely non-trivial

rippled hashes account-state leaves as `sha512Half(LeafNode, data, key)`
(`SHAMapAccountStateLeafNode.h:54`) and every entry carries `PreviousTxnID`.
Tx+meta leaves hash `(TxNode, data, key)`. So one hash appearing twice in the
same tree is effectively impossible, and a hash coming back to life later is
rare. Because storage is content-addressed, both are still *possible*, so
correctness requires counts.

## 2. Data model

One fjall `Database` at `[node_db] path`, with four keyspaces:

| Keyspace | Key | Value | Notes |
|---|---|---|---|
| `nodes` | node hash (32 B) | NodeObject encoding (unchanged codec) | Bloom filters on, LZ4, block cache from `node_size` |
| `notebook` | `seq: u32 BE` ‖ hash (36 B) | 1 byte kind: `0 = STATE`, `1 = OWNED` | Sorted by ledger seq, so pruning is one range scan |
| `counts` | node hash | `u32 LE` | Absent means 1. Stored only when count ≠ 1 |
| `meta` | ascii name | value | `schema`, `claimed_seq`, `claimed_state_root`, `pruned_to`, `anchor_seq`, `mode` |

Counter semantics: `count(h)` = multiplicity of `h` in the latest **claimed**
(validated) **state** tree.

```
 count = 1  → no row       (≈ every live state node, zero bookkeeping)
 count = 0  → explicit row (dead, waiting for its notebook seq to age out)
 count ≥ 2  → explicit row (rare duplicate)
```

Notebook kinds:

- `STATE` rows are deleted only if `count(h) == 0` and `h ∉ UNCLAIMED`.
- `OWNED` rows are for nodes owned by exactly one ledger: transaction-tree
  nodes and the ledger header object. They are deleted unconditionally,
  except when `h ∈ UNCLAIMED`. This is safe because tx/header node hashes use
  prefixes and keys disjoint from state nodes.

## 3. Components

```
 ┌──────────────────────────── quaxar ─────────────────────────────┐
 │ SHAMap flush / acquisition / bootstrap      (unchanged callers)  │
 │        │ store(h, bytes)            │ on_validated(L)            │
 │        ▼                            ▼                            │
 │ ┌─────────────────────────────────────────────────────────────┐ │
 │ │ IndexWriter  (one thread, bounded ordered queue)             │ │
 │ │   store │ claim(L) │ prune(K) │ orphan_sweep │ verify-sample │ │
 │ │   owns: UNCLAIMED: HashMap<hash, first_seen_seq>             │ │
 │ └───────────────────────────────┬─────────────────────────────┘ │
 │                                 ▼ atomic batches                 │
 │ ┌──────────────────────── fjall Database ──────────────────────┐│
 │ │  nodes │ notebook │ counts │ meta                             ││
 │ └───────────────────────────────────────────────────────────────┘│
 │ fetch(hash) → nodes.get()  (any thread, no writer involvement)   │
 └──────────────────────────────────────────────────────────────────┘
```

- `FjallBackend`: implements the extended `Backend` trait (§7, Stage 1).
- `IndexWriter`: the single serialization point for every index mutation.
- `PrunedStore`: replaces `SHAMapStore` + `DatabaseRotating`. It owns the
  writer, schedules prune/sweep/verify and exposes counters to RPC/CLI.
- `Reconciler`: offline/online mark-sweep from retained roots.

## 4. Operations

### 4.1 `store(h, bytes)`: every node write (flush, acquisition, fetch packs)

```
 nodes.put(h, bytes)
 UNCLAIMED.insert(h, current_validated_seq)   (keep the first seq seen)
```

In bulk mode (initial sync, §6 case 7) the UNCLAIMED insert is skipped.

### 4.2 `claim(L)`: each validated ledger, in publish order

Always diff against the last claimed state root (`meta.claimed_state_root`),
not necessarily the parent. This one rule covers both normal operation and gaps.

```
 if L.seq <= meta.claimed_seq → no-op (idempotent after restart)
 P = tree(meta.claimed_state_root)          (∅ for the anchor)
 NEW  = visit_differences(root=L.state, have=P)   nodes in L, not in P
 DEAD = visit_differences(root=P, have=L.state)   nodes in P, not in L
        (equal child hash ⇒ subtree skipped: cost ∝ change, not size)

 ONE BATCH:
   for h in NEW:
       counts[h] += 1          (0→1 removes the row; 1→2 writes 2)
       if h ∉ UNCLAIMED: nodes.put(h, bytes-from-memory)   (rule R3)
       UNCLAIMED.remove(h)
   for h in DEAD:
       counts[h] -= 1          (1→0 writes explicit 0)
       notebook.put((L.seq, h), STATE)
   for h in tx_tree(L) ∪ {header(L)}:
       notebook.put((L.seq + 1, h), OWNED); UNCLAIMED.remove(h)
       if h ∉ nodes: nodes.put(h, bytes)
   meta.claimed_seq = L.seq; meta.claimed_state_root = L.state_root
 db.persist(SyncAll)            (one fsync per validated ledger)
```

`visit_differences` already exists in `xrpl/shamap/src/operations/difference.rs`.

### 4.3 `prune(K)`

`K = min(validated_seq − online_delete, can_delete)`. K is the oldest ledger we
keep; a node whose notebook seq is `≤ K` was last needed by ledger `seq − 1 < K`.

```
 for (seq, h, kind) in notebook.range(meta.pruned_to+1 ..= K) in chunks of prune_batch:
     if h ∈ UNCLAIMED                                   → keep node, drop row
     else if kind == OWNED                              → nodes.remove(h)
     else if counts[h] == 0                             → nodes.remove(h); counts.remove(h)
     else (count ≥ 1, i.e. resurrected)                 → keep node, drop row
     notebook.remove((seq, h))
 per chunk: meta.pruned_to = last fully processed seq; one batch
```

### 4.4 `orphan_sweep`

Runs every N ledgers on the writer.

```
 for (h, first_seen) in UNCLAIMED where first_seen < validated_seq − online_delete:
     counts.put(h, 0); notebook.put((validated_seq, h), STATE); UNCLAIMED.remove(h)
```

Orphans are nodes stored for ledgers the network never validated: abandoned
close attempts, wrong-ledger builds (for example the 2026-10-07 fork at
21357187), and speculative acquisition. Later they go through the normal prune path.

## 5. The three safety rules

| Rule | Statement | Prevents |
|---|---|---|
| R1 | Every index mutation (store bookkeeping, claim, prune, sweep) runs on the one `IndexWriter`, in queue order | Interleaving between prune and resurrection |
| R2 | Prune never deletes `h ∈ UNCLAIMED` | Deleting a node that a not-yet-claimed ledger has just re-stored |
| R3 | Claim writes bytes for any `h ∈ NEW` not in `UNCLAIMED` (idempotent overwrite) | A claimed ledger referencing a node deleted before the claim, or lost on restart |

Rule of thumb: when unsure of a death seq, pick the **later** one. A later seq
only leaks space for a while; an earlier one deletes live data.

Two more rules govern the in-memory caches above the store: R4 (remove a
hash from FullBelowCache when it dies) and R5 (reusing a dead-pending node pins
it). Both are in Case 16.

The race these rules close:

```
 A100 died@50 (count 0); window 10; validated 60 → K = 50

 WITHOUT R1–R3                         WITH R1–R3
 t1 builder store(A100)                t1 store(A100) → UNCLAIMED ∋ A100
 t2 prune(50): count 0 → DELETE  ✗     t2 prune(50): A100 ∈ UNCLAIMED → keep ✅
 t3 claim(60): bytes "present" skip    t3 claim(60): count 0→1, row removed ✅
 ⇒ L60 missing a node ❌               later NOTEBOOK(50,A100): count=1 → keep ✅
```

## 6. Cases the node must handle

Each case lists the expected behaviour and the test that pins it (`T-*` IDs are
used in the stage plans in §7).

### Case 1: the same node comes back (resurrection)

```
 L1: Alice=100 → node A100      count 1 (no row)
 L2: Alice= 90 → A100 dies      count 0  notebook (2, A100, STATE)
 L3: Alice=100 → A100 again     count 1  ← DIFF sees A100 as NEW → count +1, row removed
 prune reaches (2, A100): count = 1 → SKIP ✅ (still needed), row dropped
 L7: A100 dies again → count 0, notebook (7, A100) → deleted when K ≥ 7 ✅
```

Tests: `T-RES-1` exact sequence above; `T-RES-2` resurrection *after* the node
was already pruned (claim must rewrite bytes, R3); `T-RES-3` resurrection racing
prune in the same writer tick (R2).

### Case 2: transaction trees (and ledger headers)

Each ledger has its own tx tree, which no later ledger reuses.

```
 L5 writes tx tree T5a T5b T5c (+ header H5)
   → notebook (6, T5a, OWNED) (6, T5b, OWNED) (6, T5c, OWNED) (6, H5, OWNED)
     "these die when ledger 6 arrives"
 → pruned unconditionally (unless UNCLAIMED) when K ≥ 6, i.e. once L5 leaves the window
```

An empty tx tree (zero root hash) writes no rows. Tests: `T-TX-1` rows and
deletion timing; `T-TX-2` empty tx set; `T-TX-3` tx_entry/RPC `tx` of a ledger
inside the window still resolves, outside it returns the same not-found as rippled.

### Case 3: crash in the middle

```
 crash during CLAIM → batch never committed (journal atomic) → as if L2 never claimed
                      on restart: meta.claimed_seq = L1 → re-diff L1 → L2, write again ✅
 crash during PRUNE → some chunks done, the rest not
                      on restart: resume from meta.pruned_to; re-deleting = no-op ✅
 crash during STORE → bytes not persisted → node refetched from peers (as today);
                      UNCLAIMED is empty after restart → R3 rewrites bytes at claim ✅
 crash after STORE, before CLAIM, ledger never validated → orphan leaks until the
                      reconciler runs (space only, never correctness) ✅
```

Tests: `T-CRASH-1..4` use a fault-injecting backend that fails or aborts at
every batch boundary and every Nth operation, followed by reopen and verify;
`T-CRASH-5` is kill -9 during a testnet soak (Stage 5).

### Case 4: a gap (missed ledgers)

```
 have claimed L100, node offline, next validated L150 (L101..L149 missing)
 → claim(L150) diffs against meta.claimed_state_root (= L100) directly
   NEW  = nodes in L150 not in L100
   DEAD = nodes in L100 not in L150 → notebook rows (150, …, STATE)
 ✅ still correct; one larger batch. Death seqs are later than the truth → safe.
```

A large batch (thousands of ledgers missed) is split: NEW and DEAD are
streamed in chunks, with `meta.claimed_*` updated only in the final chunk;
counts are applied idempotently via a per-claim staging marker. Tests:
`T-GAP-1` 50-ledger gap; `T-GAP-2` gap larger than the window; `T-GAP-3` crash
in the middle of a chunked gap claim.

### Case 5: peers ask for a node by hash

```
 peer: "send node with hash H(C)"   (TMGetObjectByHash / ledger data)
 us:   nodes.get(H(C)) → found → send   ← unchanged for the network
```

Nodes of ledgers inside the window are always present; nodes only reachable
from pruned ledgers are not found, exactly as with rippled's online delete.
Tests: `T-FETCH-1` every node of every retained ledger is fetchable;
`T-FETCH-2` p99 latency gate (Stage 5).

### Case 6: orphans (stored, never validated)

Covered by `orphan_sweep` (§4.4). Tests: `T-ORPH-1` local wrong ledger is fully
reclaimed after `online_delete` ledgers; `T-ORPH-2` UNCLAIMED stays bounded
under synthetic forks.

### Case 7: initial sync, anchor and snapshot load

```
 sync stores ~9 M nodes (bulk mode: no UNCLAIMED tracking, no prune)
 first validated ledger A: claim(A) with P = ∅
   NEW = whole tree, DEAD = ∅ → every count is 1 → zero count rows written
   meta.anchor_seq = A
 then Reconciler once: delete nodes not reachable from retained roots
 load-snapshot: same path (bulk import → anchor → reconcile)
```

Tests: `T-ANCH-1` anchor writes no count rows; `T-ANCH-2` sync leftovers
reclaimed by the reconciler; `T-ANCH-3` load-snapshot then the first claim.

### Case 8: backfill of older ledgers inside the window

History acquisition (`ledger_history`) may fetch a ledger B below `claimed_seq`.
Its state nodes that are not in the latest claimed tree get `STATE` rows at
seq `B + 1` (or the next retained successor; never earlier) with explicit
count 0 if they have no count row; its tx tree and header get `OWNED` rows at
`B + 1`. It never decrements counts of the latest tree. Ledgers below K are not
backfilled. Tests: `T-BACK-1`, `T-BACK-2` (backfill racing prune).

### Case 9: `can_delete` / advisory delete

`K` is capped by `can_delete` when `advisory_delete = 1`. Prune pauses at the
cap; claims continue. Tests: `T-ADV-1`.

### Case 10: disk nearly full

LSM deletes need space until compaction runs, so a full disk can block pruning.

- A ballast file (`[node_db] reserve_mb`, default 1024) is created at open and released when free space drops below the threshold.
- The node then prunes aggressively, forcing K up to `validated − online_delete` and running orphan_sweep, and raises an operator alert.
- New stores are refused only as a last resort, with the node degrading to not-full, never corrupting.

Tests: `T-DISK-1` with a quota-limited tmpfs or loop device.

### Case 11: verify failure (a live node is missing)

Pruning stops (`mode = halted`), an ERROR is logged with the hash and ledger,
the node is requested from peers, and `server_info` / `fetch-info` expose the
condition. Tests: `T-VER-1` (inject a deleted live node).

### Case 12: prune or claim falling behind

The writer queue is bounded, and claim has priority over prune. Lag is exported
(`claimed_seq` vs validated, `pruned_to` vs K). If the claim lag exceeds the
threshold, ledger publication is never blocked; claims catch up as a gap
(Case 4). Tests: `T-LAG-1` with a throttled backend.

### Case 13: concurrent reads during delete

Readers use `nodes.get` without the writer. Only nodes not needed by any
retained ledger are deleted, so no correct reader can race a delete. Tests:
`T-CONC-1` loom/stress test with readers walking retained trees while pruning.

### Case 14: standalone / reporting / no online_delete

With `online_delete` unset or `0`, claims still run (so enabling pruning later
needs no rebuild), but prune never runs. `ledger_history = full` requires
`online_delete = 0` (existing rule). Tests: `T-CFG-1..3`.

### Case 15: schema and engine versioning

`meta.schema = 1`. An unknown schema refuses to open. `type = NuDB` or
`type = RocksDB` after Stage 7 fails config validation with a migration
message (§8). Tests: `T-CFG-4`, `T-CFG-5`.

### Case 16: FullBelowCache must never vouch for a deletable subtree

FullBelowCache (FBC) records "every node below hash H is present" so sync can
skip a subtree (`xrpl/shamap/src/owners/sync.rs`, `touch_if_exists` → `duplicate`).
The claim is keyed by hash, not by the stored nodes. Today that is safe for two
reasons: NuDB never deletes single keys, and rotation wipes the whole FBC
(`clear_full_below_cache()` in `shamap_store_app_runtime.rs:431`). Per-ledger
pruning breaks both assumptions.

The failure without a fix:

```
 L10: Carol's branch J fully synced     → FBC[J] = full
 L20: J replaced (Carol changed)        → J, c1 dead, count 0
 prune deletes J's children (c1 …)      → FBC[J] still "full"  (stale)
 L25: an inbound ledger contains J again (resurrection / backfill)
      sync sees FBC[J] → "duplicate, skip" → c1 never fetched
 read Carol → c1 missing → SHAMapMissingNode 💥
```

The rules:

- **R4: remove FBC entries at death, not at deletion.** When claim moves a
  hash to count 0 (the DEAD set), it also removes that hash from FBC, through a
  callback from the writer to `NodeFamily` so that nodestore stays below SHAMap.
  FBC can then only describe live subtrees (count ≥ 1). Every descendant of a
  live node is live, so it is never pruned. Cost: one hash-map remove per dead
  node (~300–650 per ledger).
- **R5: reuse pins dead nodes.** When acquisition or sync reuses a node that is
  already on disk and that node is dead-pending (count 0, not yet pruned), it is
  added to UNCLAIMED, so R2 protects it until the next claim or the orphan
  sweep. The writer mirrors the count-0 rows in an in-memory `DEAD_PENDING` set
  (about `online_delete × 650` hashes, ≈ 330 k for 512), so this check needs no
  disk read.
- The bulk `clear_full_below_cache()` on rotation goes away together with
  rotation (Stage 7).

The ancestor invariant makes R4 sufficient. A parent hash contains its child's
hash, so whenever a child changes, every ancestor changes in the same ledger:
`died(ancestor) ≤ died(descendant)`. A present, live ancestor therefore implies
a present subtree. FBC only becomes dangerous for dead ancestors, and R4
removes those.

Tests: `T-FBC-1` is the exact sequence above (it must refetch c1, not skip it).
`T-FBC-2` is a dead-pending reuse during catch-up that races prune (R5).
`T-FBC-3`: the property test from Stage 3 also checks after every step that each
FBC entry's subtree is fully present.

### Case 17: serve peers from in-memory nodes first

At the moment, `serve_get_object_by_hash_request` (`xrpld/app/src/bootstrap/bootstrap.rs`)
answers each hash through `node_store.fetch_node_object`. That checks the
nodestore's own NodeObject cache, then the NuDB writable and archive backends.
It never consults the SHAMap `TreeNodeCache`, even though that cache already
holds about 8.9 M live nodes in memory on our testnet node. This matches stock
rippled. Xahau #728 adds the fallback.

The new lookup order, in an app-level helper (`TreeNodeCodec`, kept out of the
nodestore layer):

```
 1. TreeNodeCache.fetch(h)  → alive anywhere in RAM (strong or weak tier)
                             → serialize_with_prefix → reply
 2. LedgerMaster by hash     → ledger header objects (otLEDGER)
 3. nodestore.fetch(h)       → NodeObject cache → fjall `nodes`
```

Why it is safe: storage is content-addressed, and every node in memory was
hash-verified when it was created. Bytes from memory are therefore exactly the
bytes on disk, and the peer verifies the hash again. What it buys: hot
consensus-era requests skip disk and LSM reads, and nodes stay servable for as
long as any live ledger holds them, independently of prune timing. Billing
(`hits`/`misses`, `computeGetObjectByHashFee`) is unchanged; a memory hit counts
as a hit. The same helper serves fetch packs and `TMGetLedger` node lookups.

Tests: `T-SERVE-1`: a reply from memory is byte-identical to the reply from
disk for every node of a retained ledger. `T-SERVE-2`: a node that is only in
memory (deleted on disk) is still served while a live ledger holds it.
`T-SERVE-3`: the cost accounting matches the current tests. `T-SERVE-4` is a
benchmark: disk reads per 1 k peer requests, before and after.

### Case 18: `complete_ledgers` moves in lockstep with prune

The advertised range (`complete_ledgers`, peer status) is lowered to `K + 1`
in the same writer step that advances `meta.pruned_to`. It never advertises a
ledger whose nodes may already be deleted. Xahau saw the advertised range grow
to about 91 ledgers when `online_delete` was 16, because the bulk rotation it
depended on was blocked.
Tests: `T-RANGE-1`.

### Case 19: no FULL-mode gate on prune

The NuDB rotation waits on health gates (`shamap_store_health.rs`, rippled
`healthWait`) because a premature rotation drops the archive. Prune deletes only
dead nodes, so it has no such risk. It runs whenever claims advance, including
during catch-up, and is throttled only by `prune_batch` and writer priority.
Xahau observed rotation starving behind exactly this gate during catch-up. It is
also the leading suspect, unconfirmed, for our NuDB rotation stalling since
2026-10-02.
Tests: `T-GATE-1` (prune keeps advancing while the node is `syncing`).

## 7. Staged implementation

Every stage ends with the full gate:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                      # full suite (long; app integration ~40 min)
cargo test -p nodestore -p app --test all state:: integration::   # focused re-run
```

Until Stage 7, builds keep `CC=clang CXX=clang++`. After Stage 7 the gate also
runs with the system C compiler (no clang) to prove the dependency is gone.

### Implementation status

As built on branch `feat/fjall-pruned-nodestore`:

- Stages 0-3 complete: KV v2 backend trait, MemoryBackend/FaultBackend/
  conformance macro, fjall backend (passes conformance), ModelStore oracle,
  and the engine-agnostic index core (`claim`/`prune`/`orphan_sweep`,
  reconcile/verify) with a 300-trial randomized equivalence check against the
  oracle, crash tests over FaultBackend, and a concurrency test.
- Stage 2 wiring: `FjallFactory` registered; `type=fjall` accepted by config.
  `type=fjall` resolves to a single store, so no new node-store enum variant
  is needed and the existing call sites are untouched.
- Stage 4 core integration complete: `compute_claim_delta` bridges a validated
  ledger's SHAMaps to a `ClaimDelta`; `PrunedStore`/`PrunedDriver` drive
  claim+prune; the SHAMap store component runs the driver from
  `on_ledger_closed` when `type=fjall`; metrics are exposed. End-to-end tests
  build real ledgers and verify pruning on the fjall backend.
- Stage 7 removal complete: RocksDB, NuDB and the rotation machinery are gone.
  The `SHAMapStoreNodeStore` enum collapsed to a single `Single` variant; the
  rotating app runtime, bootstrap rotating path and rotation worker wiring are
  removed. In the nodestore crate the NuDB backend, its mmap reader, the
  `DatabaseRotating` trait and `database_rotating.rs`, the `NuDbContext`
  plumbing and the context/deterministic/rotating `Factory`/`Manager`
  constructors are deleted, and `NuDbFactory` is no longer registered
  (fjall/memory/null remain). Config validation rejects `type = NuDB` and
  `type = RocksDB` and defaults an unset type to fjall. The `librocksdb-sys`,
  `rocksdb`, `bindgen`, `clang-sys` and `memmap2` dependencies are absent from
  the tree (verified via `cargo tree -i`), and the workspace builds with
  `CC=cc CXX=c++` with no C++ node-store build step.
- Stage 4 fully complete: Case 16 (FBC removal callback), Case 17 (serve from
  the TreeNodeCache first, byte-identical via `serialize_with_prefix`), Case 10
  (disk-full `reserve_mb` ballast), Case 7 (snapshot load adopts the anchor and
  reconciles leftovers; export streams the retained `nodes` set), Case 18
  (`complete_ledgers` lowered to the retained floor in lockstep with the prune
  cursor via `PrunedDriver::on_floor_advanced` -> `clear_prior_ledgers`), and
  Case 19 (prune runs unconditionally, no operating-mode gate). The pruned
  metrics are exposed through RPC `get_counts`, and `verify_interval` drives a
  time-gated sampled verify of the latest ledger's reachable nodes
  (`verify_last_ok` records the result).
- Stage 0 harness: a replay-determinism test (one trace -> identical final node
  set on independent replays, matching the ModelStore oracle) plus the
  crash-resume tests (`restart_resumes_from_meta_cursors`,
  `crash_during_claim/prune`).
- Stage 6 migration: an end-to-end test exports a snapshot, loads it into a
  fresh fjall store (reclaiming a pre-sync leftover), adopts the anchor and
  keeps claiming; the installer defaults `type = fjall`, writes `reserve_mb`,
  and drops the NuDB block-size prompt and the dead RocksDB/clang build setup.
- Dead code: the rotation-era node-cache paths are gone
  (`invalidate_node_object_cache`, `advance_store_generation`, the
  `NodeObjectCacheMode`/`CacheStorage::Disabled` no-cache mode and its tests).
  The dormant rotation worker machinery is also removed: `SHAMapStore::start`
  returns false for `delete_interval == 0`, so the component's worker thread
  never ran once NuDB was gone. `SHAMapStoreComponent` no longer spawns a
  worker or exposes `process_queued_ledger`/`run_detached_worker_step`,
  `SHAMapStoreComponentRuntime` is now a marker trait, and the three
  now-unreferenced modules `shamap_store_worker.rs`, `shamap_store_copy.rs`
  and `shamap_store_app_runtime.rs` are deleted along with their rotation-only
  tests. The health/rotation-decision/runloop/paths/saved-state modules remain
  (still reachable via `rotation_decision`, the operating-mode health types,
  and saved-state paths) and compile with no dead-code warnings.
- Remaining: Stage 5 (the live 48h/72h testnet soak and its benchmark gates,
  which need a running testnet host), a full retained-window verify walk (the
  current periodic verify samples the latest ledger's reachable set, a sound
  safety net but not every retained ledger; the full walk needs a by-seq
  ledger provider threaded into the driver), and the optional server_info
  halted-state surface (verify_last_ok is already in get_counts).

### Stage 0: baseline and harness

Deliverables:

- `xtask`/bench binary `nodestore-replay`. It records a trace on a live node (every `store` hash/size, each validated ledger's state root, tx root and header) and replays it against a chosen engine.
- Baseline numbers on the 4-vCPU testnet host for NuDB rotation: disk over 48 h, bytes written per ledger, p50/p99 fetch, RSS, CPU, rotation stall time.
- A reference model, `ModelStore`: an in-memory mark-sweep oracle that computes exactly which nodes the retained window needs. Used by property tests in later stages.

Tests: replay determinism (same trace → same final node set), and model unit tests.
Exit: the baseline report is committed under `docs/design/benchmarks/`.

### Stage 1: storage abstraction v2

Deliverables:

- `Backend` trait gains `remove(hash)`, `write_batch(WriteBatch)` (atomic, multi-keyspace), `range(keyspace, from..=to)`, `persist(mode)`, and keyspace handles. `fetch`/`store` are unchanged.
- `MemoryBackend` implements v2 fully (the test engine). `NullBackend` gets stubs.
- `FaultBackend` wrapper for crash injection (fail at the Nth op or batch, torn batch simulation).

Tests: trait conformance suite (`backend_conformance!` macro) run against
Memory and Fault backends: batch atomicity, range ordering with BE keys,
remove idempotence. Existing nodestore and app suites stay green.
Exit: full gate green; no behaviour change.

### Stage 2: fjall backend (no pruning yet)

Deliverables:

- `fjall = "=3.1.12"` (pinned) in `xrpld/nodestore/Cargo.toml`.
- `FjallBackend` with keyspaces `nodes/notebook/counts/meta`, options derived from `node_size` (block cache ≈ 20–25% of the node_size memory budget, bloom filters, LZ4, L0/L1 filters pinned), and journal persist on batch commit.
- `[node_db] type = fjall` accepted by the config, `config_check` and the factory. NuDB stays default in this stage.
- `export-snapshot` / `load-snapshot` and `db-stats` work on fjall.

Tests: conformance suite on fjall (tempdir); snapshot round-trip NuDB → snapshot →
fjall → identical node set; full app suite run with the test default switched to
fjall via a feature/env flag; one testnet node syncs to full on fjall without
pruning (sanity soak, 12 h).
Exit: full gate green on both engines; the testnet sanity soak reaches full
with no extra forks.

### Stage 3: index core (engine-agnostic)

Deliverables (`xrpld/nodestore/src/pruned/`):

- `IndexWriter` with a bounded queue and priorities (claim > store-bookkeeping > prune > sweep).
- `claim`, `prune`, `orphan_sweep`, `UNCLAIMED`, notebook kinds, the counts encoding, the META cursors, and chunked large claims.
- `Reconciler` (mark from retained roots, sweep), plus `verify(sample_rate)`.

Tests:

- Unit tests for every case: `T-RES-*`, `T-TX-*`, `T-GAP-*`, `T-ORPH-*`, `T-ANCH-*`, `T-BACK-*`, `T-ADV-1`, `T-VER-1`, `T-LAG-1`, `T-FBC-1..2`.
- `DEAD_PENDING` mirror, the R4 FBC-removal callback, and R5 reuse pinning.
- A property test (proptest): random ledger sequences with edits, reverts (resurrection), forks/orphans, gaps, backfill and random window sizes. After each step, the store's node set must equal `ModelStore`'s required set ∪ allowed leaks, and no required node may be missing (ever).
- Crash tests `T-CRASH-1..4`: FaultBackend at every boundary, then reopen, resume and assert the invariants.
- Concurrency test `T-CONC-1`.

Exit: 0 property failures over at least 1M generated steps in CI-extended mode.

### Stage 4: integration (replace the rotation worker)

Deliverables:

- Route `Database::store` through `IndexWriter` bookkeeping; hook `on_validated` (the published-ledger path that currently feeds `SHAMapStore`) to `claim`.
- `PrunedStore` replaces `SHAMapStore` when `type = fjall`. NuDB keeps the old worker until Stage 7.
- `[node_db] prune_mode = dry_run | on` (dry-run logs would-be deletes and verifies them against reachability), plus `prune_batch`, `verify_interval`, `reserve_mb`.
- Observability: `get_counts` / `fetch-info` / `server_info` fields `claimed_seq`, `pruned_to`, `unclaimed`, `notebook_rows`, `count_rows`, `prune_lag`, `verify_last_ok`. CLI `db-stats` shows fjall keyspace sizes.
- `can_delete` RPC semantics preserved.
- FBC callback wired to `NodeFamily` (R4); acquisition reuse hook (R5).
- `TreeNodeCodec` app-level helper; `serve_get_object_by_hash_request`, fetch packs and `TMGetLedger` node lookups use memory → header → nodestore (Case 17).
- `complete_ledgers` lowered with `pruned_to` (Case 18); prune not gated on operating mode (Case 19).
- Lock-fairness audit of the read path (`nodes.get`, NodeObject cache, TreeNodeCache): no writer-preferring lock may block peer or consensus reads for longer than one prune batch (lesson L4).

Tests: app integration tests for validated-ledger publishing with pruning;
RPC `ledger`/`ledger_data`/`tx`/`account_tx` inside and outside the window match
rippled-style not-found; `T-CFG-1..3`; `T-DISK-1`; `T-FBC-3`; `T-SERVE-1..4`;
`T-RANGE-1`; `T-GATE-1`; full suite.
Exit: full gate green; dry-run on a local testnet sync shows 0 would-delete live nodes.

### Stage 5: testnet soak and benchmark gates

Deliverables:

- 48 h `prune_mode = dry_run` on the testnet node, then 72 h `prune_mode = on`.
- A second node on NuDB in parallel as control.
- `nodestore-replay` results for fjall vs the NuDB baseline.
- `T-CRASH-5` (kill -9 three times during the soak).

Pass/fail gates:

| Metric | Pass |
|---|---|
| Disk after 72 h, online_delete=512 | ≤ 60% of the NuDB baseline, and flat (no growth trend) |
| p99 `fetch(hash)` | ≤ 1.2× NuDB |
| Peak RSS | ≤ NuDB + 1 GB |
| claim + prune per ledger | p99 < 200 ms |
| Verify | 0 missing live nodes, 0 halts |
| Consensus | forks/demotions ≤ the control node's |

Exit: all gates pass; the report is committed.

### Stage 6: migration path

Deliverables:

- An operator path that keeps no NuDB code alive: `export-snapshot` with the old binary, then `load-snapshot` into fjall with the new binary (Case 7). The alternative is a fresh sync.
- Installer: new installs default to `type = fjall`; existing configs with `type = NuDB` get a guided message.
- Docs: `RUNNING.md`, `CONFIGURATION.md`, `SYNCING.md`, `README.md` updated.

Tests: end-to-end migration test (NuDB fixture → snapshot → fjall → node starts,
claims, prunes); installer dry-run tests.
Exit: the testnet node is migrated via snapshot and stays in sync for 24 h.

### Stage 7: removal of NuDB, RocksDB and the rotation machinery

Deliverables:

- Delete the code listed in §9 and the dependencies `rocksdb`, `librocksdb-sys` (nodestore and basics).
- `config_check`: `type = NuDB|RocksDB` is an error with the migration hint; `nudb_block_size` and the RocksDB option keys are removed.
- `install.sh`, `install.ps1`, `Dockerfile` and `infra/aws/setup-testnet.sh` drop librocksdb-dev and the clang requirement.
- Tests that exist only for NuDB/RocksDB/rotation are deleted. Tests of behaviour (online delete windows, can_delete, snapshot) are ported to fjall.

Tests:

- Full suite with no clang: `CC=cc CXX=c++` (or gcc).
- `cargo tree -i librocksdb-sys`, `-i bindgen` and `-i clang-sys` must each fail ("not found"), asserted by a CI script.
- `grep -ri "nudb\|rocksdb" --include=*.rs` returns only the migration error message and its tests.

Exit: full gate green with gcc only; dependency assertions pass.

### Stage 8: hardening and tuning

Deliverables: compaction and cache tuning per `node_size`; adopt SingleDelete
once fjall stabilizes it (fewer tombstones, since node keys are written once
and deleted once); a scheduled weekly Reconciler; a mainnet-scale replay estimate.
Tests: full suite plus replay regression thresholds in CI.

## 8. Configuration (after Stage 7)

```ini
[node_db]
type = fjall
path = /var/lib/quaxar/db/fjall
online_delete = 512       # keep at least this many ledgers
advisory_delete = 0       # can_delete caps K when 1
prune_mode = on           # on | dry_run
prune_batch = 10000       # notebook rows per prune batch
verify_interval = 3600    # seconds between sampled verifies
reserve_mb = 1024         # disk ballast for Case 10
```

## 9. Removal inventory (Stage 7)

Measured on `b8d3e196`.

| Area | Files (approx. lines) |
|---|---|
| NuDB backend | `xrpld/nodestore/src/backends/nudb_backend.rs` (4415), `backends/mmap_reader.rs` (41) |
| RocksDB backend | `xrpld/nodestore/src/backends/rocksdb.rs` (1310), `xrpl/basics/src/io/rocksdb.rs` (40), plus the doc comment at `xrpl/shamap/src/owners/storage.rs:16` |
| Rotation | `xrpld/nodestore/src/database_runtime/database_rotating.rs` (1684); rotation parts of `manager.rs`, `factory.rs`, `database.rs` |
| SHAMapStore rotation worker | `xrpld/app/src/shamap/shamap_store_{rotation,copy,worker,health,backend,app_runtime,component,paths,runtime_state,bootstrap}.rs` (≈3.9k). `shamap_store_sql.rs` / `_relational.rs` (SQL history clear) move into `PrunedStore` |
| CLI | `xrpld/cli/src/db_stats.rs` NuDB file sizes, `doctor.rs` NuDB check, `config_check.rs` NuDB/RocksDB keys |
| Main | `xrpld/main/src/main.rs` `nudb.dat` path checks and their tests |
| Bootstrap | Genesis/next-ledger "NuDB persistence" messages in `bootstrap.rs` (renamed, logic kept) |
| Build/infra | `librocksdb-sys`/`rocksdb` deps; `install.sh` librocksdb-dev and clang checks; `install.ps1`; `Dockerfile`; `infra/aws/setup-testnet.sh` |
| Docs | NuDB/RocksDB sections in README, `CONFIGURATION.md`, `RUNNING.md`, `SYNCING.md`, `ARCHITECTURE.md` |

Native toolchain after removal (verified with `cargo tree -i` on `b8d3e196`):

- **Removed:** `bindgen` and `clang-sys` (pulled only by `librocksdb-sys`), the C++ RocksDB build and the `librocksdb-dev` package. libclang is no longer required.
- **Still needed:**
  - A plain C compiler (`cc`, gcc or clang) for `secp256k1-sys`, `libsqlite3-sys`, `tikv-jemalloc-sys` and `openssl-sys`.
  - `cmake`, for `aws-lc-sys` (via rustls).
  - Removing those is out of scope.
- **Dockerfile:** uses clang only as the mold linker driver (`RUSTFLAGS -C linker=clang`). Stage 7 switches it to `cc`, or keeps clang deliberately as a linker choice, not a build requirement.

## 10. Sizing (testnet, measured 2026-10-07)

| Input | Value |
|---|---|
| Nodes written per ledger | 654 (~300 KB) |
| Live state | ≈ 8.9 M nodes ≈ 4–5 GB |
| Index rows per ledger | ≈ DEAD (300–650) notebook + same counts ≈ 30–65 KB |
| Index on disk, window 512 | ≈ 16–33 MB (2000: 65–130 MB) |
| Expected disk | ≈ 4.3 GB + LSM slack (10–20%) ≈ 5 GB, flat |
| Current NuDB (rotation stalled since Oct 2) | 26.5 GB and growing ~3.6 GB/day |

LSM write amplification (typically 10–20× for leveled compaction) has **not
been measured** for random 32-byte keys on our host; Stage 0/5 measure it.

## 11. Lessons from Xahau PR #728

Source: <https://github.com/Xahau/xahaud/pull/728> (open), the discussion
between shortthefomo and sublimator, sublimator's design gist
(<https://gist.github.com/sublimator/6da8c771c99e1a2446bb6b70aab571c2>), and the
rippled port XRPLF/rippled #6549 (open). Xahau's `RWDB` is an in-memory
`std::map` backend, not an LSM tree. PR #728 runs it as a null node store, in
which the in-memory `Ledger → SHAMap → node` pointer graph is the store and
`shared_ptr` refcounts do the garbage collection.

| # | Their finding | What we adopt |
|---|---|---|
| L1 | FBC claims are keyed by hash and go stale once the nodes behind them are freed. They needed three checks: liveness, walked-ness, anchoring | Case 16 (R4, R5) |
| L2 | Retire one ledger at a time, with the advertised range moving in lockstep | Cases 18 and 4.3 |
| L3 | `healthWait` blocks rotation during catch-up, so ledgers pile up | Case 19 |
| L4 | glibc `shared_mutex` prefers writers, so readers starve during rotation and consensus transitions stall | Stage 4 lock audit. Also a lead (unverified) for our ~20 s consensus stall at 21357187 |
| L5 | Serve `GetObjectByHash` from TreeNodeCache and ledger headers (their `TreeNodeCodec` proposal) | Case 17 |
| L6 | Rotation copies are redundant when live trees already hold the nodes | Already the premise of this design |
| L7 | A unit typo (`burstSize` 64 TB) went unnoticed | Stage 8: a test that checks the `node_size` tables against rippled |

Not adopted as the primary design: running with no durable store. It needs
the whole state in RAM (#6549 reports 11–13 GB on XRPL), and every restart
means a full resync. A future `type=none` mode for validators could reuse
Case 17 and the FBC rules unchanged.

## 12. Risks and open questions

- DEAD-set exactness is the main correctness risk. It is contained by R1–R3, conservative seqs, the property tests against `ModelStore`, and verify.
- UNCLAIMED memory during heavy acquisition needs a hard cap (spill to an `unclaimed` keyspace if exceeded); the size is to be tuned in Stage 3.
- fjall maturity versus RocksDB: mitigated by the conformance suite, crash tests, the soak, and the snapshot path as an escape hatch.
- Mainnet churn is unmeasured; ratios should hold, absolute numbers will be larger.
- The SQL ledger/transaction DB pruning (`clear_prior`) is kept as is and moves under `PrunedStore`.
- Prerequisite outside this plan: the current NuDB rotation stall (no rotation since 2026-10-02, disk full in ~4 days) needs a stopgap before Stage 5.
