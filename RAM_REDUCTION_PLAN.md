# RAM Reduction Plan — Verified & Corrected

## Audit Verdicts (Independent Re-validation)

| Claim | Verdict | Evidence |
|-------|---------|----------|
| F1: backed flag not set during acquisition | **DISPROVEN** | `Ledger::new(header, true)` at `ledger_fetcher.rs:2734` → `SyncTree::new_with_type(_, backed=true, _)` at `ledger/src/lib.rs:541`. Acquisition maps START with backed=true. |
| F2: Release mechanisms are dead code | **CONFIRMED** | `spill_full_below_subtrees` and `release_deep_children` have zero call sites in `xrpld/`. Only `release_to_disk` (full eviction) is called in application code. |
| F3: Shared FullBelowCache targets dead object | **CONFIRMED** | Registry's `full_below` (registry.rs:85) is only used for generation numbers. The live cache is per-acquisition `worker_full_below` on `AcquisitionState`. |
| F4: Per-acq FullBelowCache = 63 MB each | **PARTIALLY CONFIRMED** | Target size is 524K entries but entries are ~40 bytes each (KeyCache), not 110-120. Max ~20 MB per cache. 4–8 acquisitions = 80–160 MB, not 250–500 MB. |
| F5: fetch_info is a stub | **CONFIRMED** | Returns `{}` at `app_server_info.rs:2802`. |
| F6: No RSS monitoring exists | **DISPROVEN** | `tikv-jemalloc-ctl` RSS reading at `application_root.rs:4334-4357`. Logged during `on_closed_ledger`. Not exposed via RPC. |
| F7: Sweep doesn't immediately free | **PARTIALLY TRUE** | Sweep evicts entries from the cache map, but deallocation depends on `SharedIntrusive` refcount. If no other holder exists, memory IS freed immediately. |
| F8: Ledger clone is expensive | **DISPROVEN** | `SyncTree` derives `Clone` which only increments the root `SharedIntrusive` refcount (8-byte atomic increment). Cheap. |
| S7: data_buffer unbounded | **CONFIRMED** | `Vec<(u64, InboundLedgerPacket)>` with no cap. Low practical risk (one packet per peer response). |

---

## Corrected Design Assumptions

1. **`backed = true` from creation** — No fix needed. `spill_full_below_subtrees` and `release_deep_children` WILL work on acquisition maps without any additional changes. The earlier plan was correct on this point.

2. **Release mechanisms need activation, not repair** — They exist, are safe, but are never called during acquisition. The fix is to ADD call sites, not repair the functions themselves.

3. **Per-acquisition FullBelowCache is the correct target** — Phase 0 sweep must target `state.worker_full_below`, not the registry's shared cache.

4. **RSS monitoring exists but isn't in RPC** — Phase 0 only needs to expose the existing jemalloc stats via get_counts, not build a new monitoring system.

5. **Ledger completion is cheap** — No optimization needed for `completed_ledger()`.

---

## Revised Implementation Plan

### Phase 0: Observability & Cleanup

#### 0.1 — Expose Existing RSS via get_counts RPC
The jemalloc stats already exist at `application_root.rs:4334`. Add an equivalent read in the `GetCountsSource` implementation:
```rust
fn process_rss_bytes(&self) -> u64 {
    tikv_jemalloc_ctl::epoch::advance().ok();
    tikv_jemalloc_ctl::stats::resident::read().unwrap_or(0) as u64
}
```
Expose as `process_rss_bytes` in the get_counts JSON output.

#### 0.2 — TreeNodeCache High-Water-Mark
Track `max(get_track_size())` between sweeps. Expose as `treenode_cache_hwm` in get_counts.

#### 0.3 — Per-Acquisition Memory Counter
Add `nodes_accepted: AtomicU64` to `AcquisitionState`. Increment on each successful `add_known_node`. Report in fetch_info (once wired).

#### 0.4 — Wire fetch_info to Real Data
Replace `get_ledger_fetch_info()` stub with actual acquisition state: active count, per-ledger progress (nodes received / missing), sequence, age.

#### 0.5 — FullBelowCache Sweep Housekeeping
In the housekeeping timer (bootstrap.rs:~1070), iterate active acquisitions and call `worker_full_below.cache.sweep()` on each. This prevents mid-lifetime unbounded growth.

**Rollout gate**: Deploy to testnet. Verify `process_rss_bytes` and `treenode_cache_hwm` appear in `get_counts` output. Confirm `fetch_info` shows active acquisitions. Collect 24h baseline.

---

### Phase 1: Conservative Spill After Each Tick

**What**: Call `spill_full_below_subtrees()` after each `get_missing_nodes` / `trigger_with_family()` cycle in `process_data_job`.

**Why it's safe**:
- `backed = true` from construction — the guard passes.
- Only releases nodes already marked `full_below` — their subtree is complete and persisted.
- FullBelowCache prevents re-scanning of released subtrees.
- NuDB reload path (`fetch_cached_node_result_with_ledger_seq`) is tested and functional.

**Location**: `acquisition.rs`, inside `process_data_job` after the `trigger_with_family` call completes:

```rust
// Release completed subtrees to bound memory growth
let generation = state.worker_full_below.generation();
let mut mutable = state.mutable.lock().expect("acquisition mutable lock");
let state_spilled = mutable.inbound.ledger_mut()
    .map(|l| l.state_map_mut().spill_full_below_subtrees(generation))
    .unwrap_or(0);
let tx_spilled = mutable.inbound.ledger_mut()
    .map(|l| l.tx_map_mut().spill_full_below_subtrees(generation))
    .unwrap_or(0);
if state_spilled + tx_spilled > 0 {
    tracing::debug!(target: "acquisition", state_spilled, tx_spilled, seq = state.seq, "Released full-below subtrees");
}
```

**Invariants**:
- Never called unless `backed == true` (enforced by function guard)
- Never releases a node that isn't also in NuDB (guaranteed by `got_node` sync filter)
- FullBelowCache generation matches the current acquisition (no stale generations)

**Validation**:
- Unit test: Construct a complete SyncTree, call `spill_full_below_subtrees`, verify all nodes remain fetchable via the family
- Integration: Cold-bootstrap testnet. Compare peak RSS (expect 30–50% reduction), sync time (expect <5% regression)
- Assertion: Zero hash mismatches on completed ledgers

**Rollout gate**: Peak RSS drops measurably. No hash errors. Sync time ≤ 105% of baseline.

---

### Phase 2: Periodic Aggressive Release

**What**: Periodically call `release_deep_children(keep_depth=2)` from the housekeeping timer for all active acquisitions.

**Why it's safe**:
- Same `backed = true` invariant
- Reload path exists: `resolve_sync_child_for_add_known_node_with_family` → NuDB fetch
- Concurrency safe: `Mutex<AcqMutableState>` serializes all tree operations

**NVMe-only gate**: At startup, probe NuDB read latency (10 random fetches from existing data). If p50 > 1ms, disable Tier 2 and log warning.

**New method on InboundLedgers** (`registry.rs`):
```rust
pub fn release_deep_children_all(&self, keep_depth: usize) {
    let inner = self.inner.lock().expect("registry lock");
    for (_, entry) in inner.entries.iter() {
        if let Ok(mut mutable) = entry.mutable.lock() {
            if let Some(ledger) = mutable.inbound.ledger_mut() {
                ledger.state_map_mut().release_deep_children(keep_depth);
                ledger.tx_map_mut().release_deep_children(keep_depth);
            }
        }
    }
}
```

**Call site** (bootstrap.rs housekeeping timer, every `sweep_interval`):
```rust
if on_nvme {
    shared_inbound.release_deep_children_all(2);
}
```

**Emergency eviction**: If `stats::resident::read()` > `rss_hard_limit`, call with `keep_depth = 1`.

**Validation**:
- Benchmark cold bootstrap: peak RSS every 30s, NuDB reads/s, time-to-full

---

### Phase 3: Hardening

#### 3.1 — TreeNodeCache Hard Cap
Add `hard_max_entries` (default 2× profile target). If exceeded, trigger synchronous sweep before inserting.

#### 3.2 — "Huge" Profile TTL Cap During Acquisition
While any acquisition is active, cap effective TTL to 180s regardless of profile. Prevents 4–8 GB cache growth under the 900s default.

#### 3.3 — NuDB Read Latency Histogram
Instrument `NuDbBackend::fetch()` with p50/p95/p99 tracking. Expose in `get_counts`. Use for automatic Tier 2 disablement.

#### 3.4 — data_buffer Soft Cap
Add a configurable max size (default 1024 packets). Drop oldest packets when exceeded. Log a warning.

#### 3.5 — Configuration Surface
```ini
[memory_optimization]
tier2 = auto          # auto | true | false
rss_hard_limit = 0    # 0 = 80% system RAM
min_keep_depth = 2    # minimum retained tree depth
```

---

## Non-Goals (Explicitly Excluded)

| Proposed Change | Why Excluded |
|----------------|--------------|
| F1 fix (set_backed after persist) | Not needed — backed is already true |
| F8 fix (avoid Ledger clone) | Ledger clone is cheap (refcount increment) |
| Shared FullBelowCache across acquisitions | Per-acquisition isolation is intentional (generation numbers differ). Sharing would cause false positives |
| RSS monitoring from scratch | Already exists; only needs RPC exposure |

---

## Rollback Conditions

Revert immediately if:
1. Any completed ledger has a hash mismatch
2. `add_known_node` panics or returns unexpected errors
3. Time-to-full regresses > 25%
4. NuDB read errors in logs

All changes are config-gated. `tier2 = false` + removing spill call sites restores baseline.

---

## Risk Matrix (Corrected)

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| NuDB read failure after spill | Very Low | Low (timeout re-requests from peers) | Timeout mechanism + logged warning |
| Throughput regression on HDD | Medium | Medium | NVMe-only gate with latency probe |
| Concurrency race in release | Very Low | High | Mutex + per-branch spinlock (audited sound) |
| Cache thrash (release→reload→release) | Low | Medium | Tier 1 (full_below only) avoids this; Tier 2 uses 60s intervals |
| FullBelowCache false positive | Very Low | High | Append-only per generation; cleared on acquisition failure |

---

## Timeline

| Week | Phase | Deliverable |
|------|-------|------------|
| 1 | Phase 0 | Expose RSS/HWM via RPC, wire fetch_info, worker_full_below sweep |
| 1–2 | Phase 1 | `spill_full_below_subtrees` activation + unit/integration tests |
| 2–3 | Phase 2 | `release_deep_children_all` + NVMe gate + cold-bootstrap benchmark |
| 3–4 | Phase 3 | Hard cap, latency histogram, config surface |

---

## Expected Outcomes (Corrected Estimates)

| Metric | Before | After Phase 1 | After Phase 2 |
|--------|--------|---------------|---------------|
| Peak RSS (medium, 4 acqs) | 2–4 GB | 1–2 GB | 300–500 MB |
| Peak RSS (huge, 8 acqs) | 4–8 GB | 2–4 GB | 500 MB–1 GB |
| FullBelowCache per acq | ~20 MB | ~20 MB (swept) | ~20 MB |
| Time-to-full | Baseline | ≤ +5% | ≤ +15% |
| NuDB reads/s during sync | ~500 | ~500 | ~1200–2000 |

---

## Files to Modify

### Phase 0
- `xrpld/rpc/src/handlers/get_counts.rs` — RSS, HWM fields
- `xrpld/rpc/src/state/app_server_info.rs` — GetCountsSource impl
- `xrpld/rpc/src/commands/fetch_info.rs` + `app_server_info.rs` — wire real data
- `xrpld/app/src/bootstrap/bootstrap.rs` — worker_full_below sweep in housekeeping

### Phase 1
- `xrpld/app/src/ledger/inbound_ledgers/acquisition.rs` — spill call in process_data_job

### Phase 2
- `xrpld/app/src/ledger/inbound_ledgers/registry.rs` — `release_deep_children_all()`
- `xrpld/app/src/bootstrap/bootstrap.rs` — periodic release call
- `xrpld/nodestore/src/backends/nudb_backend.rs` — latency probe

### Phase 3
- `xrpl/basics/src/sync/tagged_cache.rs` — hard cap
- `xrpld/app/src/ledger/inbound_ledgers/registry.rs` — data_buffer cap
- Configuration parsing + docs
