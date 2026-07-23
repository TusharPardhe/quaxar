# quaxar In-Memory Cache / Store Inventory

Read-only investigation. No files were modified. Scope: every in-memory
structure that holds SHAMap nodes, ledger data, or account state.

## Summary Table

| Cache / Store | Holds | RAM footprint driver | Scope | Eviction policy | Required for correctness? | File : Function |
|---|---|---|---|---|---|---|
| `TreeNodeCache` (shared acquisition tree cache) | Decoded `SHAMapTreeNode` objects (inner + leaf, account-state & tx-tree nodes), keyed by node hash | `node_size` profile: 262,144 (tiny) → 524,288 (small) → 2,097,152 (medium) → 4,194,304 (large) → 8,388,608 (huge) entries; age 30–900s | **Global/shared** across all acquisition workers in the ledger-catchup thread (one `Arc` shared by `InboundLedgers` and `SharedInboundLedgers`) | Age+size-scaled sweep (`TaggedCache::sweep`), weak-pointer downgrade then drop when unreferenced; explicit `.clear()` after a ledger completes | **No** — pure performance cache; on miss the family fetcher falls through to NuDB (`fetch_node_object`) | `xrpld/main/src/main.rs:4445` (`TreeNodeCache::new`, catchup thread) via sizing in `xrpld/app/src/node_family/node_family.rs:38-66` (`NodeSizeResourceProfile::for_node_size`) |
| `TreeNodeCache` (app-level default/RPC node family) | Same as above, used by RPC/status/state contexts and the default `attach_default_node_family` path | Same `node_size` profile sizing | **Per-Application-instance** (one per `ApplicationRoot`/RPC context, not shared with the acquisition-thread cache) | Same sweep/expire mechanics via `TaggedCache` | No — performance cache with NuDB fallback | `xrpld/app/src/state/application_root.rs:2745-2753` (`attach_default_node_family`); also constructed ad hoc in `xrpld/rpc/src/state/context.rs:1132,1176,1298` and `xrpld/app/src/state/app_registry.rs:1027` |
| `TreeNodeCache` (consensus-driver fallback) | Same node type, used only when `SharedInboundLedgers` has not yet been wired via `ledger_master_runtime` | Hardcoded **131,072** entries / 300s age (independent of `node_size`) | Global fallback singleton created lazily in the consensus driver | Same `TaggedCache::sweep` | No — performance cache | `xrpld/app/src/bootstrap/bootstrap.rs:962-967` |
| `FullBelowCacheImpl` (shared acquisition) | Node **hashes only** (`Uint256` keys, unit `()` values) marking subtrees already confirmed fully present, used to prune sync traversal | Hardcoded **524,288** entries across all `node_size` profiles (`acq_full_below_size`), 10-minute default expiration | **Global/shared** across acquisition workers via `Arc` | `TaggedCache::sweep`; `.reset()`/`.clear()` bumps a generation counter to invalidate stale marks | **No** — an eviction only means the traversal re-checks a subtree; never causes incorrect state, only extra work | `xrpld/main/src/main.rs:4456` (construction); `xrpl/shamap/src/owners/family.rs:86-160` (`FullBelowCacheImpl`, `FullBelowCache` trait) |
| `FullBelowCacheImpl` (per-worker, `SharedInboundLedgers`) | Same (hash-only "fully below" marks) | Fixed **1024** entries, generation seeded from shared cache generation+1 | **Per-acquisition-worker** — deliberately *not* shared across workers to avoid cross-worker false "complete" marks (see code comment) | `TaggedCache::sweep`/`clear` | No — perf/traversal-pruning only | `xrpld/app/src/ledger/shared_inbound_ledgers.rs:1077-1083` (`run_acquisition_worker`, `worker_full_below`) |
| `FetchPackCache` | Raw serialized SHAMap node blobs (`Uint256 -> Blob`) fetched via the legacy fetch-pack delta-sync protocol; validated by re-hashing (`sha512_half`) on retrieval | `node_size`-scaled: 8,192 (tiny) → 16,384 (small) → 32,768 (medium) → 49,152 (large) → 65,536 (huge); 45s age in the main catchup path | **Global/shared** — one `Arc<FetchPackCache>` reused by `InboundLedgers` and `SharedInboundLedgers` | `TaggedCache::sweep` age/size-based; also **consumed on read** — `get_fetch_pack` deletes the entry after a successful hash-verified retrieval (`cache.del`) | **No** — delta-sync optimization; absence just means falling back to normal node-by-node ledger sync | `xrpld/app/src/consensus/fetch_pack.rs:16-72` (`FetchPackCache`, `add_fetch_pack`/`get_fetch_pack`); construction at `xrpld/main/src/main.rs:4468` |
| `KeyCache<Uint256>` "shared_stored" write-dedup (main catchup) | Node **hashes only**, marking hashes already written to the NuDB write queue this session, to avoid double-enqueuing the same node | `node_size`-scaled `write_dedup_size`: 131,072 (tiny) → 262,144 (small) → 524,288 (medium) → 786,432 (large) → 1,048,576 (huge); 30s age | **Global/shared** — passed into `InboundLedgers::new` | `TaggedCache`-style age/size sweep (`KeyCache::sweep`) | **No** — a false-negative (evicted-then-rewritten) just causes a redundant, idempotent NuDB write, not corruption | `xrpld/main/src/main.rs:4516-4521` (`shared_stored` for `InboundLedgers`) |
| `KeyCache<Uint256>` "shared_stored" write-dedup (`SharedInboundLedgers`) | Same purpose, separate instance for the consensus-driven acquisition path | Same `write_dedup_size` scaling as above, 30s age | **Global/shared** for `SharedInboundLedgers`, distinct instance from the main-catchup one | Same `KeyCache::sweep` | No | `xrpld/main/src/main.rs:4564-4569` (`SharedInboundLedgers::new` call, "shared-acq-dedup"); fallback with fixed size 1024 at `xrpld/app/src/bootstrap/bootstrap.rs:978-982` ("driver-dedup") |
| `pending_writes: HashMap<Uint256, PendingNodeStoreObject>` | **Full serialized node bytes** (`obj_type`, `data: Vec<u8>`, `hash`) staged after acquisition but before the async NuDB writer thread confirms the write | Unbounded `HashMap` — no capacity limit, no eviction; grows with however many nodes are acquired faster than the NuDB writer thread drains them | **Global/shared** `Arc<Mutex<HashMap<...>>>`, one instance shared between acquisition workers and the NuDB writer thread | **None** — entries are only removed explicitly: (a) by the writer thread after a successful `store()` call (`pending_writes.lock()...remove(&hash)`), or (b) bulk-cleared via `.clear()` once a ledger finishes and `flush_writes` confirms all writes drained | **Yes, for correctness** — acts as the authoritative read-through layer (`WorkerNodeFetcher`/`WorkerStore::fetch_node_data`) so a node written-but-not-yet-flushed to NuDB can still be read back during the same acquisition; without it, `NodeStoreNodeStore` reads could miss just-written nodes | Struct: `xrpld/app/src/ledger/shared_inbound_ledgers.rs:60-66` (`PendingNodeStoreObject`); map: `set_pending_writes`/field at `xrpld/app/src/ledger/shared_inbound_ledgers.rs:~220,~330`; writer drain: `spawn_nodestore_writer` `xrpld/app/src/ledger/shared_inbound_ledgers.rs:100-145`; consumer reads: `WorkerNodeFetcher::fetch_node_object` and `WorkerStore::fetch_node_data`/`store_object` `xrpld/app/src/ledger/shared_inbound_ledgers.rs:~700-830` |
| `TaggedCache` fast-path `DashMap` (`fast_map`) | Strong pointer shortcuts to already-canonicalized cache values (lock-free duplicate index over the same entries tracked in `state.cache`) | Bounded implicitly by the same `target_size`/sweep as the owning `TaggedCache`; cleared at the start of every `sweep()` | Internal to each `TaggedCache`/`TreeNodeCache`/`FullBelowCacheImpl` instance — inherits that cache's scope (shared or per-worker) | Cleared wholesale on every `sweep()` and on `clear()`/`reset()`; individual keys removed on `del()` | No — pure read-path acceleration mirroring the authoritative `state.cache` | `xrpl/basics/src/sync/tagged_cache.rs:~430` (`fast_map: DashMap<K, SP, S>`, `fetch`, `sweep`) |
| `KeyCache` internal map | Key-only presence tracking (used for `FullBelowCache`-style and write-dedup caches) | Governed by `target_size`/`target_age` passed at construction (see rows above) | Inherits scope of the specific `KeyCache` instance | `KeyCache::sweep` — age-based with size-scaled cutoff (`expiration_cutoff`) | No | `xrpl/basics/src/sync/tagged_cache.rs:~940-1040` (`KeyCache`, `KeyCacheState`) |

## Detailed Findings

### 1. `SHAMapFamily` — `xrpl/shamap/src/owners/family.rs`

`SHAMapFamily<C, S, FB, F, MR, NS>` (struct at line ~333) is not itself a
cache/store — it is a **coordinator** that bundles together:

- `tree_node_cache: Arc<TreeNodeCache<C, S>>` — the actual node cache (see §2).
- `full_below_cache: FB` — a `FullBelowCache` implementation (see §3).
- `fetcher: F` — a `SHAMapNodeFetcher` seam that falls through to the node
  store (NuDB) or `pending_writes` on cache miss.
- `missing_node_reporter: MR` — no memory footprint of note; triggers
  acquisition requests.
- `node_store: Option<Mutex<NS>>` — a write seam, not a cache; wraps whatever
  concrete node-store sink is passed in (e.g. `SHAMapStoreNodeStore`).

Key method `fetch_cached_node_result_with_ledger_seq` (line ~430) implements
the read path: tree-cache hit → `fetcher.fetch_node`/`fetch_node_object`/
`fetch_node_blob` → decode and `canonicalize` (insert) into the tree cache.
This confirms the tree cache is a **read-through/write-through cache in front
of NuDB**, not an independent source of truth — correctness does not depend
on any particular entry staying resident.

`sweep()` (line ~700) and `reset()` (line ~690) simply delegate to the
`full_below_cache` and `tree_node_cache`, confirming both are swept/reset
together as one logical unit whenever a `NodeFamily` is swept/reset (see
periodic sweep call sites in `main.rs`, `last_cache_sweep_at`).

### 2. `TreeNodeCache` — `xrpl/shamap/src/nodes/tree_node_cache.rs`

```rust
pub type TreeNodeCache<C = MonotonicClock, S = HardenedHashBuilder> = TaggedCache<
    Uint256, SHAMapTreeNode, C, S,
    SharedWeakUnion<SHAMapTreeNode>, SharedIntrusive<SHAMapTreeNode>,
>;
```

It is a **type alias** over the generic `TaggedCache` (see §7), specialized
to hold intrusive, weak-upgradeable pointers to `SHAMapTreeNode`. Each entry
therefore holds a decoded, in-memory SHAMap tree node (inner node child-hash
array, or leaf node's serialized item/blob). Entries can be either "strong"
(kept alive purely by the cache — subject to sweep/eviction) or "weak"
(pinned alive by an external strong reference elsewhere, e.g. an in-flight
SHAMap traversal — the cache only tracks it, it does not control its
lifetime).

There is **no single global instance**. Distinct `TreeNodeCache` instances
exist for:
- the shared acquisition/catchup path (`main.rs:4445`, sized by
  `CatchupResourceProfile`/`NodeSizeResourceProfile`),
- the default/RPC application node family (`application_root.rs:2747`,
  `rpc/src/state/context.rs`, `app_registry.rs:1027`),
  each sized independently from the same `node_size` profile function but
  **not sharing memory** with the acquisition cache,
- the consensus-driver fallback (`bootstrap.rs:962`, fixed 131,072 entries,
  independent of `node_size`),
- transaction-set caches in consensus (`rcl_consensus.rs:357,940`,
  `RclTxSetSharedCache`) — same type, separate purpose (caching transaction
  SHAMap nodes during consensus, not ledger state).

This means **actual peak RAM usage from tree-node caching is the sum across
all concurrently-instantiated caches**, not a single bounded number — e.g. a
running node has at minimum the acquisition-thread cache *and* an
app/RPC-level cache alive simultaneously, each capped independently at the
`node_size`-derived `tree_cache_size`.

Cleared explicitly on ledger-acquisition completion:
`shared_tree_cache.clear()` in `shared_inbound_ledgers.rs` (both the
in-loop completion branch and the post-loop completion branch), specifically
to free RAM once nodes are confirmed persisted in NuDB.

### 3. `FullBelowCache` / `FullBelowCacheImpl` — `xrpl/shamap/src/owners/family.rs`

`FullBelowCache` is a trait (line ~48); `FullBelowCacheImpl<C, S>` (line ~86)
is the concrete implementation, wrapping a
`TaggedCache<Uint256, (), C, S, SharedWeakCachePointer<()>, Arc<()>>` — i.e.
it stores **no payload**, only hash keys, used purely to remember "this
subtree hash was already confirmed fully present locally" so that the sync
traversal (`SyncTree::get_missing_nodes_with_family`) can skip re-descending
into it. `generation()` is an `AtomicU32` used to invalidate all entries in
O(1) on `.clear()`/`.reset()` without a real sweep (the generation bump is
compared elsewhere — though in the current code the impl still delegates to
`cache.clear()`; the atomic generation counter matches the reference
rippled design intent of avoiding stale full-below hits after a fork/reset).

Two independent scopes exist, deliberately:
- **Shared** `FullBelowCacheImpl` (`main.rs:4456`, `acq_full_below_size` =
  524,288 for all node_size profiles) — reused across the whole acquisition
  subsystem.
- **Per-worker** `FullBelowCacheImpl` (`shared_inbound_ledgers.rs:1077`,
  fixed size 1024) — explicitly *not* the shared one. The comment at that
  call site explains why: sharing `full_below` marks across concurrently
  running acquisition workers would let one worker's "fully synced" marks
  cause a different worker to wrongly skip fetching leaf nodes it never
  actually downloaded, leading to `have_state=true` with an incomplete state
  map. This is a correctness-motivated *isolation* decision, even though the
  cache's own contents remain non-authoritative (a miss just means
  redundant traversal work, not wrong data).

### 4. `FetchPackCache` — `xrpld/app/src/consensus/fetch_pack.rs`

```rust
pub struct FetchPackCache<C = MonotonicClock, S = HardenedHashBuilder> {
    cache: TaggedCache<Uint256, Blob, C, S>,
}
```

Holds **raw serialized node blobs** (`Blob = Vec<u8>`), keyed by hash, used
for the legacy XRPL "fetch pack" delta-sync feature (a peer proactively sends
a bundle of nodes it expects the requester to need next). `get_fetch_pack`
(line ~48) validates integrity via `sha512_half(&data) == hash` and — notably
— **deletes the entry on successful retrieval** (`self.cache.del(&hash,
false)`), i.e. this is explicitly single-use/self-draining, not a
long-lived read cache.

There is a second, structurally distinct `FetchPackContainer` trait defined
in `xrpld/ledger/src/acquisition/fetch_pack.rs` (a narrow trait-only file,
no storage) — this is just the interface seam consumed by ledger sync code;
the actual backing store is the `FetchPackCache` in `app/src/consensus/`.

Sized from `CatchupResourceProfile::acq_fetch_pack_size`, scaling with
`node_size` from 8,192 up to 65,536 entries; construction sites at
`main.rs:4468` (main catchup, 45s age) and `bootstrap.rs:974` (consensus
driver fallback, fixed 256 entries / 120s age, independent of `node_size`).

### 5. `KeyCache<Uint256>` "shared_stored" — `xrpl/basics/src/sync/tagged_cache.rs`

`KeyCache<K, C, S>` (struct at line ~940) is a lighter-weight sibling of
`TaggedCache` that tracks **only key presence + last-access time**, no
payload at all (`KeyOnlyEntry { last_access }`). Used for the
`shared_stored` write-dedup map: `should_store_hash` (implemented in
`WorkerStore::should_store_hash`, `shared_inbound_ledgers.rs`) calls
`shared_stored.insert(hash)`, which returns `false` if the hash was already
inserted recently — letting the caller skip re-serializing/re-enqueuing a
node that's already been queued for a NuDB write in this window. This is
purely a **CPU/IO-avoidance optimization**: a false negative here (evicted
too early) merely causes a redundant, idempotent write to NuDB, not any data
loss — NuDB writes for the same hash+content are idempotent.

Sized via `CatchupResourceProfile::write_dedup_size` (`node_size`-scaled:
131,072 → 1,048,576), 30s age. Two separate instances: one for the
main-catchup `InboundLedgers` (`main.rs:4516`, name `"acq-write-dedup"`) and
one for `SharedInboundLedgers` (`main.rs:4564`, name `"shared-acq-dedup"`),
plus a hardcoded-size (1024) fallback instance for the consensus-driver path
(`bootstrap.rs:978`, name `"driver-dedup"`).

### 6. `pending_writes: HashMap<Uint256, PendingNodeStoreObject>` — `xrpld/app/src/ledger/shared_inbound_ledgers.rs`

This is the **most RAM-risk-relevant** structure found:

```rust
#[derive(Debug, Clone)]
pub struct PendingNodeStoreObject {
    pub obj_type: nodestore::NodeObjectType,
    pub data: Vec<u8>,
    pub hash: Uint256,
}
```

Wrapped as `Arc<Mutex<HashMap<Uint256, PendingNodeStoreObject>>>` — full,
owned copies of serialized node bytes staged in RAM between the moment an
acquisition worker decides to persist a node (`WorkerStore::store_object`,
around line ~830) and the moment the dedicated NuDB writer thread
(`spawn_nodestore_writer`, lines 100–145) actually performs the blocking
`db.store(...)` call and removes the entry.

Unlike every other structure in this inventory, **this map has no configured
capacity limit and no time-based eviction** — it is bounded only by how fast
the single NuDB writer thread can drain it relative to how fast acquisition
workers produce writes. Under sustained high-throughput acquisition (e.g.
multiple ledgers' full account-state trees arriving faster than disk I/O can
absorb), this map is the most likely unbounded-growth vector in the
acquisition subsystem.

It is also functionally **required for correctness**, not just an
optimization: `WorkerNodeFetcher::fetch_node_object` and
`WorkerStore::fetch_node_data` both check `pending_writes` *before* falling
through to NuDB, so that a just-written-but-not-yet-flushed node can still be
read back within the same acquisition (NuDB reads would otherwise miss it).
It is explicitly `.clear()`-ed only after `flush_writes()` confirms the NuDB
writer has drained its queue (both in the in-loop and post-loop completion
branches of `run_acquisition_worker`), specifically documented in the code as
being done "to free RAM."

### 7. `TaggedCache` / `MonotonicClock` — `xrpl/basics/src/sync/tagged_cache.rs`

`TaggedCache<K, T, C, S, P, SP>` is the generic building block underlying
`TreeNodeCache`, `FullBelowCacheImpl`, and `FetchPackCache`. Internals:

- `state: RecursiveMutex<TaggedCacheState<K, T, P, SP, S>>` — the
  authoritative store: a `PartitionedUnorderedMap` of `ValueEntry { ptr,
  last_access }`, where `ptr` is either a strong pointer (cache-owned
  lifetime) or a weak pointer (externally-owned lifetime, cache just tracks
  last access).
- `fast_map: DashMap<K, SP, S>` — a lock-free duplicate index of currently
  strong entries, used purely to accelerate `fetch()`/`fetch_with()` without
  taking the `RecursiveMutex`. Cleared at the start of every `sweep()` and
  repopulated lazily as entries are re-fetched.
- `target_size` / `target_age` — the two knobs controlling `sweep()`
  behavior. `expiration_cutoff` (line ~1090) computes an **adaptive**
  expiry: if the cache has grown past `target_size`, the effective max-age is
  scaled down proportionally (`scale_duration`) so a cache under memory
  pressure ages out entries faster than its nominal `target_age`, with a
  1-second floor.
- `sweep()` (line ~640): for strong entries past the cutoff, downgrades them
  to weak if still externally referenced (`use_count() > 1`), or removes them
  outright if the cache was the sole owner. For already-weak entries, removes
  them if `expired()` (i.e., the external owner dropped its reference).

`MonotonicClock` (line ~40) is the production `CacheClock` impl, backed by
`std::time::Instant`; `ManualClock` is a deterministic test-only clock. There
is no `MonotonicClock` struct named "the" global clock — every cache
construction site passes its own `MonotonicClock::default()`, so clocks are
**not shared** across cache instances (each just measures wall-clock time
independently, which is fine since `Instant`-based clocks are consistent
process-wide).

### 8. `node_size` cache-sizing configuration

Two node_size-driven sizing tables exist and interact:

**`NodeSizeResourceProfile::for_node_size`**
(`xrpld/app/src/node_family/node_family.rs:38-66`) — the base profile:

| node_size | tree_cache_size | tree_cache_age | ledger_fetch |
|---|---|---|---|
| tiny | 262,144 | 30s | 2 |
| small | 524,288 | 60s | 3 |
| medium (default) | 2,097,152 | 90s | 4 |
| large | 4,194,304 | 120s | 5 |
| huge | 8,388,608 | 900s | 8 |

**`CatchupResourceProfile::for_node_size`**
(`xrpld/main/src/main.rs:820-880`) — wraps the above and adds acquisition-
specific sizes (`acq_full_below_size` fixed at 524,288 for *every* profile;
`acq_fetch_pack_size` and `write_dedup_size` do scale per-profile; also sets
`run_data_concurrency`):

| node_size | acq_fetch_pack_size | write_dedup_size | run_data_concurrency |
|---|---|---|---|
| tiny | 8,192 | 131,072 | 2 |
| small | 16,384 | 262,144 | 3 |
| medium | 32,768 | 524,288 | 6 |
| large | 49,152 | 786,432 | 6 |
| huge | 65,536 | 1,048,576 | 8 |

`node_size` is read from config via `status_rpc_node_size()` /
`ledger_acquisition.ledger_fetch_limit` can override just the `ledger_fetch`
field independently of the rest of the profile
(`with_ledger_fetch_limit_override`, `main.rs:884-889`).

`xrpld.cfg` (repository sample config) sets `[node_size] medium` (line 24)
and documents `[ledger_acquisition] ledger_fetch_limit` as an optional
override (line 29), consistent with `docs/CONFIGURATION.md`'s `[node_size]`
section (lines 43-64), which explicitly documents that the profile
"influences SHAMap tree cache size, cache age, fetch-pack cache size,
write-dedup size, and run-data concurrency" without enumerating the exact
numbers — the concrete constants live only in the two Rust profile functions
above, not in the docs or config file.

## Notable Cross-Cutting Observations

1. **No single "total cache RAM" bound exists.** RAM usage from these caches
   is the sum of at least 3 independently-sized `TreeNodeCache` instances
   (acquisition, app/RPC default, consensus-driver fallback) plus the
   unbounded `pending_writes` map, plus the fixed-size `FullBelowCache`/
   `FetchPackCache`/`KeyCache` instances. None of these are cross-aware of
   each other's memory usage.
2. **Only `pending_writes` is unbounded and correctness-load-bearing** at the
   same time — every other structure inventoried here is a bounded,
   swept, pure-performance cache backed by NuDB as the ground truth.
3. **Per-worker isolation of `FullBelowCacheImpl` in `SharedInboundLedgers`**
   is a deliberate correctness safeguard against false-positive "fully
   synced" marks leaking across concurrent acquisition workers — documented
   directly in code comments at `shared_inbound_ledgers.rs:1071-1076`.
4. **Explicit RAM-reclaim points**: `shared_tree_cache.clear()` and
   `shared_pending_writes.lock()...clear()` are both called immediately after
   a ledger acquisition completes and writes are flushed, in both the
   in-loop and post-loop completion branches of `run_acquisition_worker`
   (`shared_inbound_ledgers.rs`), specifically to bound peak RSS during
   sustained catchup.
