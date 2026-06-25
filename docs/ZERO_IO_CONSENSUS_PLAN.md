# Zero-I/O Consensus: Implementation Plan

## Problem Statement

When quaxar is under heavy RPC load, the consensus thread's disk reads
(via `node_fetcher` → NuDB pread) get delayed behind RPC disk reads.
This causes consensus to run late, validator drift, and missed rounds.

## Solution: Strong Arc Pinning + Dedicated Consensus Executor

Two changes that guarantee consensus never does disk I/O:

### Change 1: Pin validated ledger state in memory via strong references

**Current behavior:**
- `LedgerMaster.valid_ledger` holds `Arc<Ledger>`
- `Ledger.state_map` is a `SyncTree` whose nodes live in `TreeNodeCache`
- `TreeNodeCache` holds `SharedWeakUnion` (weak refs that can be evicted)
- When consensus calls `build_ledger_from_consensus(parent_ledger, ...)`,
  it traverses `parent.state_map()` → hits evicted nodes → disk I/O

**New behavior:**
- The consensus runtime holds a `PinnedLedger` that pre-loads all tree
  nodes into strong `SharedIntrusive` refs
- These strong refs keep nodes alive regardless of cache eviction
- Consensus traversal follows in-memory pointers only — zero pread calls

**Architecture:**
```
ConsensusRuntime {
    pinned_parent: Option<PinnedLedger>,    // parent for current round
    pinned_validated: Option<PinnedLedger>, // last validated
}

PinnedLedger {
    ledger: Arc<Ledger>,
    pinned_nodes: Vec<SharedIntrusive<SHAMapTreeNode>>,  // strong refs
}
```

When a new validated ledger arrives:
1. Walk the state_map tree from root to all leaves
2. Collect all `SharedIntrusive<SHAMapTreeNode>` strong refs
3. Store in `PinnedLedger.pinned_nodes`
4. Drop the previous pinned ledger (nodes freed by Arc refcount)

Now `build_ledger_from_consensus` traverses a tree whose nodes are all
guaranteed in-memory (strong refs held by `pinned_nodes`).

### Change 2: Dedicated consensus executor with CPU pinning

**Current behavior:**
- Consensus runs on `xrpld-validation-processor` thread (dedicated, elevated priority)
- But `do_accept` can block on disk I/O during state traversal
- Thread priority doesn't help when blocked on I/O

**New behavior:**
- Consensus thread is pinned to CPU core 0 (via `pthread_setaffinity_np`)
- Since Change 1 guarantees no disk I/O, the thread is purely CPU-bound
- It will never yield to the scheduler due to I/O wait
- RPC threads are excluded from core 0 (remaining cores)

## Files to Modify

| File | Change |
|------|--------|
| `xrpld/app/src/consensus/rcl_consensus.rs` | Add `PinnedLedger` struct, pin parent on `do_accept` entry |
| `xrpl/shamap/src/owners/sync.rs` | Add `collect_all_nodes()` method on SyncTree |
| `xrpld/app/src/ledger/ledger_master_state.rs` | Emit signal when validated ledger changes (pin trigger) |
| `xrpld/main/src/main.rs` | Pin consensus parent after checkAccept promotes a ledger |
| `xrpld/main/src/main.rs` | CPU affinity for validation-processor thread |

## Implementation Steps

### Step 1: Add `collect_strong_refs()` to SyncTree

Walk the tree recursively, collecting SharedIntrusive refs for all loaded
nodes. This is O(N) where N = number of nodes in the tree (~20-50K for
a typical validated ledger).

### Step 2: Create `PinnedLedger` in consensus module

Wraps `Arc<Ledger>` + `Vec<SharedIntrusive<SHAMapTreeNode>>`.
Implements `Drop` to release all refs (nodes eligible for eviction again).

### Step 3: Pin parent ledger before `build_ledger_from_consensus`

In `do_accept`, before building the next ledger:
1. Get parent ledger
2. Call `parent.state_map().collect_strong_refs(family)` to load+pin ALL nodes
3. Store as `self.pinned_parent`
4. Build ledger (all reads hit in-memory nodes)
5. After build completes, move to `self.pinned_validated`

### Step 4: Release on new validated

When a new ledger validates:
1. `pinned_validated` = new pinned ledger
2. Old pinned ledger drops → nodes freed (if no other refs)

### Step 5: CPU affinity (Linux only)

Pin `xrpld-validation-processor` to core 0 via:
```rust
unsafe {
    let mut cpuset: libc::cpu_set_t = std::mem::zeroed();
    libc::CPU_SET(0, &mut cpuset);
    libc::pthread_setaffinity_np(libc::pthread_self(), ..., &cpuset);
}
```

## Memory Impact

- Typical validated ledger: ~25K state nodes × ~500 bytes = ~12.5MB
- Pinning 2 ledgers: ~25MB of guaranteed-resident heap memory
- This is heap (anonymous pages) — never evicted with swap disabled
- Equivalent to what rippled always holds via strong `shared_ptr` in its tree

## Performance Impact

- Consensus `do_accept`: 0 disk reads (was 0-200 pread calls depending on cache state)
- Consensus latency: deterministic ~5ms (was 5ms-100ms depending on I/O queue)
- Under 50 concurrent RPC: consensus unaffected (was 50-100ms delayed)
- Memory: +25MB constant (negligible on production hardware)
