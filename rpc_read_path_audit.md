# RPC / Transaction Submission Read-Path Audit

Scope: read-only investigation of how RPC handlers and tx submission obtain
ledger and account state in the `quaxar` codebase. No files were modified.

## 1. Concrete trace: `account_info`

File: `xrpld/rpc/src/handlers/account_info.rs`

`do_account_info()` takes a generic `AccountInfoSource` (production impl:
`ApplicationAccountInfoSource<'a>`, wrapping `&ApplicationRoot`).

Call chain for a single `account_info` request:

1. `lookup_ledger_with_result()` (via `LedgerLookupSource` trait,
   `xrpld/rpc/src/handlers/ledger_lookup.rs`) resolves the target ledger
   handle (`LedgerLookupLedger { hash, seq, open }`) using
   `ApplicationAccountInfoSource::get_ledger_by_seq/get_ledger_by_hash/
   get_validated_ledger/get_closed_ledger/get_current_ledger`.
   - These methods read **only** three in-memory `ArcSwapOption<Ledger>`
     slots on `ApplicationRoot`: `validated_ledger()`, `published_ledger()`,
     `closed_ledger()` (plus an optional caller-supplied "current" open
     ledger). See `application_root.rs:4277` (`validated_ledger`),
     `:3883` (`closed_ledger`), `:3937` (`published_ledger`), backed by
     `SharedLedgerMasterState` (`xrpld/app/src/ledger/ledger_master_state.rs`),
     which is three `arc_swap::ArcSwapOption<Ledger>` fields — pure RAM,
     no cache, no NuDB fallback in this accessor layer.
2. `source.read_account_root(&ledger, account_id)` →
   `ApplicationAccountInfoSource::read_entry()` →
   `self.lookup_resolved_ledger(ledger)` (matches the `LedgerLookupLedger`
   handle back to the concrete `Arc<Ledger>` from step 1) →
   `resolved.read(keylet)`.
3. `Ledger::read()` (`xrpld/ledger/src/lib.rs:1372`) calls
   `self.state_map.peek_item(keylet.key, &mut fetch_fn)`, where `fetch_fn`
   invokes `self.node_fetcher` — a closure stored on the `Ledger` struct
   itself (`xrpld/ledger/src/lib.rs:492-510`, field `node_fetcher:
   Option<Arc<dyn Fn(SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>>>>`).

**Answer to (1): it is a mix, but layered, not "either/or" per call.**
The RPC handler always reads through an in-memory `Arc<Ledger>` object
(never touches NuDB directly itself). That `Ledger`'s SHAMap traversal
falls back to NuDB **only for tree nodes not already resident in the
SHAMap's in-process node cache**, via the `node_fetcher` closure attached
to the ledger. So:
   - Ledger *selection* (which `Arc<Ledger>` to read from) = pure RAM,
     three `ArcSwap` slots.
   - SHAMap *node* reads within that selected ledger = RAM-cache-first,
     NuDB-fallback-second, via `node_fetcher`.

## 2. Where the NuDB-backed node fetcher comes from, and what happens on eviction

The production node fetcher closure is built once by
`ApplicationRoot::node_fetcher_from_store()`
(`xrpld/app/src/state/application_root.rs:2486`):

```
fetch(hash) =
    node_family.fetch_cached_node(hash, 0)        // in-RAM SHAMap node cache
        .or_else(|| nodestore.fetch_node_object(hash, 0, Synchronous, false)
                       .and_then(decode))          // NuDB fallback
```

This closure is attached to a `Ledger` via `Ledger::set_node_fetcher()`
(`xrpld/ledger/src/lib.rs:1140`), and the attach point is
`ApplicationRoot::ledger_with_node_fetcher()` (`application_root.rs:3758`).
Every ledger that becomes reachable through the three RPC-visible slots
goes through this function first:
   - `on_closed_ledger()` → `ledger_master_state.note_closed_ledger(normalized)`
   - `on_validated_ledger()` / `note_validated_ledger_for_sync()` →
     `ledger_master_state.note_validated_ledger(normalized)`
   - `on_published_ledger()` → `ledger_master_state.note_published_ledger(normalized)`

So **every** `Arc<Ledger>` an RPC handler can obtain via
`validated_ledger()/closed_ledger()/published_ledger()` already carries a
working NuDB-backed `node_fetcher`. If a SHAMap node for that specific
ledger sequence was evicted from `node_family`'s in-RAM tree-node cache
(`shamap::family::SHAMapFamily`, a `TaggedCache`-style bounded cache), the
`node_fetcher` transparently re-fetches it from NuDB. **This part of the
design works correctly today** — SHAMap node eviction is a non-issue for
correctness, only for latency.

**However, whole-`Ledger`-object eviction is a different, unresolved gap:**

- `LedgerMaster::ledger_history()` (`xrpld/ledger/src/domain/master.rs`)
  wraps a `LedgerHistory<C,S>` whose backing store is a size/age-bounded
  `TaggedCache<SHAMapHash, Ledger, ...>`
  (`xrpl/basics/src/sync/tagged_cache.rs`, default size 256, default age
  5 min — see `LedgerMasterConfig::history_cache_size/history_cache_age`
  in `master.rs`). `TaggedCache::sweep()` evicts entries once
  `state.cache.len() > target_size` or an entry ages past `target_age`.
- `LedgerMaster::get_ledger_by_hash()` / `get_ledger_by_seq()`
  (`master.rs`, no `LedgerInfoProvider`/family args) check, in order:
  `ledger_history.get_cached_ledger_by_hash/seq()` → then the single
  `closed_ledger` `LedgerHolder` slot → return `None` on a full miss.
  **There is no NuDB fallback in this method.**
- This is the method actually called by the production RPC lookup helper
  `get_ledger_obj()` (`xrpld/rpc/src/state/app_server_info.rs:1683`, used
  by `ApplicationServerInfo<V>` — the shared `LedgerLookupSource` /
  `LedgerSource` implementation wired into `server_handler.rs`'s dispatched
  commands such as `ledger`, `ledger_entry`, `ledger_data`, `tx`,
  `transaction_entry`, etc.): it calls
  `app.ledger_master_runtime().ledger_master().get_ledger_by_hash/by_seq()`
  first, and if that misses, falls back only to the same three
  `validated_ledger()/closed_ledger()/published_ledger()` slots — **still
  all in-memory, still no NuDB rebuild of a fully-evicted historical
  ledger.**
- A NuDB-backed reload path for whole ledger **headers+SHAMap** genuinely
  exists — `LedgerHistory::get_ledger_by_hash/by_seq()`
  (`xrpld/ledger/src/history_runtime/history.rs:216-330`), generic over a
  `LedgerInfoProvider` + `SHAMapFamily`, calling
  `Ledger::load_by_hash_with_provider_and_config_or_none()` /
  `load_by_index_with_provider_and_config_or_none()`
  (`xrpld/ledger/src/domain/persistence.rs`) — but it is wired into exactly
  one call site: `xrpld/app/src/ledger/loaded_ledger_runtime.rs`
  (`AppLedgerMasterRuntime::resolve_request_ledger`), which serves the
  **peer overlay wire protocol's `GetLedger` request** (peer-to-peer
  ledger sync), invoked from `xrpld/main/src/main.rs`. It is **not called
  from anywhere in the `rpc` crate.**

**Answer to (2):** SHAMap-node-level eviction from the RAM tree-node cache
is handled correctly (NuDB fallback via `node_fetcher`, verified in
`account_info`'s trace above). Whole-`Ledger`-object eviction from
`LedgerHistory`'s `TaggedCache` is **not** handled correctly for RPC
today: once a historical ledger ages/sizes out of that cache and is not
one of the three live `validated/closed/published` slots, RPC calls
through `get_ledger_obj()` return `LedgerNotFound` even though the ledger
header and full state are durably present in NuDB. The NuDB-backed
rebuild logic that would fix this (`LedgerHistory::get_ledger_by_hash/
by_seq` with a `LedgerInfoProvider`) exists in the codebase already but is
currently reachable only from the peer/overlay `GetLedger` path, not from
RPC.

## 3. Transaction submission read path

Entry point: `ApplicationRoot::submit_transaction_to_network_ops()`
(`application_root.rs:3071`) → `NetworkOpsRuntime::submit_transaction()` →
queued job → `NetworkOpsRuntime::process_transaction()` → eventually
`apply_network_ops_pending_to_open_ledger()`
(`application_root.rs:~3205-3260`), which is the actual state-read/apply
step:

```rust
let base_ledger = self.closed_ledger().or_else(|| self.validated_ledger());  // in-memory slot
...
let mut submit_view = sandbox_holder.lock()...take()
    .unwrap_or_else(|| Sandbox::new(Arc::clone(&base_ledger), ApplyFlags::NONE));
```

- `base_ledger` comes from the same two `ArcSwapOption<Ledger>` slots used
  by RPC reads (`closed_ledger()` / `validated_ledger()`), both of which —
  per section 2 — always carry an attached NuDB-backed `node_fetcher`
  (via `ledger_with_node_fetcher()` at every write site).
- `open_ledger_sandbox: Arc<Mutex<Option<Sandbox<Ledger>>>>`
  (`application_root.rs:230`) is a **persistent, process-lifetime mutable
  overlay** (`ApplyStateTable` deltas) on top of `base_ledger`, matching
  rippled's persistent `OpenView` pattern — subsequent submits see prior
  submits' effects without re-reading the base ledger.
- `Sandbox<Ledger>::read()` (`xrpld/ledger/src/views/sandbox.rs`) delegates
  cache misses to `self.table.read(self.base.as_ref(), k)`
  (`ApplyStateTable`), which falls through to `base.read()` =
  `Ledger::read()` = the same `node_fetcher`-backed NuDB fallback traced
  in section 1.

**Answer to (3):** Tx submission reads current account/ledger state through
the *same* mechanism as RPC reads — an in-memory `Arc<Ledger>` (closed or
validated slot) wrapped in a mutable `Sandbox`, with SHAMap-node-level NuDB
fallback via the ledger's `node_fetcher`. It is not "in-memory only" at the
node level, but it *is* "in-memory only" at the ledger-selection level: if
`closed_ledger()` and `validated_ledger()` are both `None` (e.g. very early
startup, before first sync), submission returns `None`/no-op
(`apply_network_ops_pending_to_open_ledger` early-returns) — there is no
NuDB path to reconstruct "the current ledger" from scratch for submission.

## 4. What must stay in RAM vs. what could move to NuDB-backed lookups

### Must remain in RAM today (hard requirement of current design)

| Data structure | Location | Why it must stay in RAM |
|---|---|---|
| `SharedLedgerMasterState` (`closed_ledger`, `validated_ledger`, `published_ledger` — 3×`ArcSwapOption<Ledger>`) | `xrpld/app/src/ledger/ledger_master_state.rs:21` | Sole source of truth for ledger *selection* in every RPC handler and in tx submission (`account_info.rs` via `ApplicationRoot::validated_ledger/closed_ledger/published_ledger`; `application_root.rs:submit_transaction_to_network_ops` base_ledger). No NuDB read path substitutes for these three pointers. |
| `open_ledger_sandbox: Arc<Mutex<Option<Sandbox<Ledger>>>>` | `application_root.rs:230` | Holds *unpersisted* deltas from transactions applied to the open ledger since the last close. This data does not exist in NuDB until the ledger closes; by definition it cannot be NuDB-backed. |
| `open_ledger_account_seqs: Mutex<HashMap<AccountID,u32>>` | `application_root.rs` (used in `network_ops_current_account_seq`, `note_open_ledger_tx`) | Tracks per-account next-expected sequence across in-flight open-ledger submissions, ahead of the closed ledger's persisted `Sequence` field. Ephemeral, NuDB has no concept of "not yet closed" sequence state. |
| `node_family: SHAMapFamily` in-RAM tree-node cache (`FullBelowCache`, `TreeNodeCache`) | wired through `node_fetcher_from_store()` (`application_root.rs:2486`) | Not a hard requirement for *correctness* (NuDB fallback exists), but required for acceptable latency — every cache miss is a synchronous NuDB fetch. |
| `LedgerMaster::ledger_history()` (`TaggedCache<SHAMapHash, Ledger>`, size 256 / age 5 min default) | `xrpld/ledger/src/domain/master.rs` | Currently a **hard** requirement for RPC by-seq/by-hash historical ledger lookups (`get_ledger_obj`), because — per section 2 — the NuDB-backed rebuild path (`LedgerHistory::get_ledger_by_hash/by_seq` with `LedgerInfoProvider`) is not wired into RPC. If a ledger falls out of this cache and isn't validated/closed/published, RPC cannot recover it today even though NuDB has it. This is the most actionable gap. |
| `held_transactions: Mutex<CanonicalTXSet>`, `local_txs: LocalTxs` | `master.rs` | Local/held transaction state has no ledger-state representation; inherently RAM-only by design (matches rippled). |

### Already NuDB-backed / cache-miss-fallback capable (could be leaned on more)

| Mechanism | Location | Current usage |
|---|---|---|
| `Ledger::node_fetcher` closure (SHAMap-node-level NuDB fallback) | `xrpld/ledger/src/lib.rs:1140` (`set_node_fetcher`), populated by `ApplicationRoot::node_fetcher_from_store()` (`application_root.rs:2486`) | Used by every RPC read (`account_info`, `account_lines`, etc. via `Ledger::read()`) and by tx submission (via `Sandbox<Ledger>::read()`). This is the "WorkerNodeFetcher pattern" the task asked about — it is the general-purpose seam, not test-only. |
| `WorkerNodeFetcher` (test/bootstrap-scoped duplicate of the same fetch-then-NuDB pattern) | `xrpld/app/src/ledger/shared_inbound_ledgers.rs:725` | Used during ledger **acquisition** (catching up from peers), for filling in SHAMap nodes as they arrive. Same `fetch_node_object` shape as `node_fetcher_from_store`; not currently reused by RPC but architecturally identical — RPC already has its own equivalent. |
| `LedgerHistory::get_ledger_by_hash/by_seq` with `LedgerInfoProvider` (full ledger header+SHAMap reload from NuDB/SQL) | `xrpld/ledger/src/history_runtime/history.rs:216-330`, backed by `Ledger::load_by_hash_with_provider_and_config_or_none` / `load_by_index_with_provider_and_config_or_none` (`xrpld/ledger/src/domain/persistence.rs`) | **Only wired into the peer overlay's `GetLedger` handler** (`xrpld/app/src/ledger/loaded_ledger_runtime.rs`, driven from `xrpld/main/src/main.rs`). Not reachable from any RPC handler. This is the clearest "reuse this" candidate: it already knows how to go from a ledger hash/seq → NuDB header row (via `LedgerInfoProvider`, backed by `LoadedLedgerDbProvider`'s SQL) → attach a `SHAMapFamily` for node reads → produce a fully-usable `Arc<Ledger>`. |
| `RpcNodeStoreFetcher` / `RpcInboundLedgerStore` (`fetch_node_object` wrappers) | `xrpld/rpc/src/state/context.rs` | Used by the RPC crate's own `ledger_request`/inbound-ledger-acquisition support code (peer-driven ledger acquisition triggered via RPC), not by ordinary read handlers — but demonstrates the RPC crate already has direct NuDB access plumbing (`app::SHAMapStoreNodeStore`) it could reuse for a `get_ledger_obj` NuDB-fallback extension. |

## 5. Existing NuDB-backed cache-miss-fallback patterns RPC could reuse

Three independent, structurally-identical `fetch_node_object`-wrapping
closures exist in the codebase today, confirming a repeated, well-understood
pattern rather than a one-off:

1. **`ApplicationRoot::node_fetcher_from_store()`**
   (`xrpld/app/src/state/application_root.rs:2486`) — family-cache-first,
   NuDB-second. This is the one actually used by RPC/tx-submit today (via
   `Ledger::node_fetcher`). Already fully general-purpose.
2. **`WorkerNodeFetcher`** (`xrpld/app/src/ledger/shared_inbound_ledgers.rs:725`)
   — same shape, scoped to the ledger-acquisition worker thread; also checks
   a `pending_writes` map for not-yet-flushed nodes.
3. **`RpcNodeStoreFetcher`** (`xrpld/rpc/src/state/context.rs`) — same
   `fetch_node_object` wrapping, scoped to the RPC crate's own inbound
   ledger acquisition/backfill support (`ledger_request` admin command).

For the *specific* gap identified in section 2 (RPC cannot recover a whole
ledger that has aged out of `LedgerHistory`'s `TaggedCache` and isn't one
of the three live slots), the directly reusable existing implementation is
**`LedgerHistory::get_ledger_by_hash`/`get_ledger_by_seq`** (history.rs)
plus its `LedgerInfoProvider` dependency (already implemented for the peer
overlay path as `LoadedLedgerDbProvider` in `loaded_ledger_runtime.rs`,
backed by SQLite ledger-header rows) and `Ledger::load_by_hash_with_provider_and_config_or_none`
/ `load_by_index_with_provider_and_config_or_none`
(`xrpld/ledger/src/domain/persistence.rs`). These already do exactly what
RPC's `get_ledger_obj()` would need — the missing piece is purely a wiring
gap: `get_ledger_obj()` currently calls the no-fallback
`LedgerMaster::get_ledger_by_hash/by_seq`, and would need to call
`LedgerMaster::ledger_history().get_ledger_by_hash/by_seq(..., provider)`
instead (or in addition), which requires threading a `LedgerInfoProvider`
and a `SHAMapFamily` through to the RPC layer — both of which already exist
in `main.rs`/`loaded_ledger_runtime.rs` for the overlay path and would need
to be exposed to `ApplicationRoot`/`app_server_info.rs`.

## Summary table

| Question | Answer |
|---|---|
| (1) SLE/SHAMap-node read source | Ledger *selection*: in-memory only (3 `ArcSwap` slots). SHAMap *node* read within a selected ledger: RAM cache first, NuDB fallback via `node_fetcher` — confirmed end-to-end in `account_info`. |
| (2) In-memory `Ledger` eviction handling | SHAMap-node eviction: handled correctly (NuDB fallback works). Whole-`Ledger`-object eviction from `LedgerHistory`'s bounded cache: **not** handled for RPC — falls through to `LedgerNotFound` even though NuDB has the data; a working NuDB-rebuild path exists but is wired only to the peer overlay `GetLedger` handler, not RPC. |
| (3) Tx submission read source | Same node-level mechanism as RPC (`Sandbox<Ledger>` wrapping `closed_ledger()`/`validated_ledger()`, NuDB fallback via `node_fetcher`). Ledger *selection* is in-memory-only; no base ledger means no submission (no NuDB reconstruction of "current" state). |
| (4) Hard RAM requirements | 3 `ArcSwapOption<Ledger>` ledger-selection slots; `open_ledger_sandbox`; `open_ledger_account_seqs`; SHAMap node cache (perf, not correctness); `LedgerHistory` `TaggedCache` (currently a correctness dependency for historical RPC lookups due to wiring gap, not an inherent one). |
| (5) Reusable NuDB fallback patterns | `node_fetcher_from_store` (already used by RPC/tx-submit), `WorkerNodeFetcher` (acquisition-scoped twin), `RpcNodeStoreFetcher` (RPC-crate's own NuDB access), and `LedgerHistory::get_ledger_by_hash/by_seq` + `LedgerInfoProvider` + `Ledger::load_by_*_with_provider` (full ledger reload, currently peer-overlay-only — the concrete fix target for the gap in (2)). |
