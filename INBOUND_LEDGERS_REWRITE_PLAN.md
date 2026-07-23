# InboundLedgers Unified Rewrite Plan

## Goal
Delete the two redundant acquisition systems and replace with ONE global `InboundLedgers` service matching rippled's architecture exactly.

## What Exists Today (to delete)

### 1. `xrpld/app/src/ledger/shared_inbound_ledgers.rs` (1678 lines)
- Used by consensus/validation path
- Has: job queue, AcquisitionState, dedup, eviction, sweep, thread pool
- Arc-based, shared across threads

### 2. `xrpld/main/src/main.rs` lines ~2998-3550 (553 lines)  
- `struct InboundLedgers` — used by catchup/bootstrap loop
- Has: entries HashMap, acquire, sweep, thread spawn per acquisition
- &mut self, single-threaded bootstrap loop

### 3. `xrpld/ledger/src/acquisition/inbound_ledgers.rs` (250 lines)
- `InboundLedgersLocal` — lower-level state machine wrapper
- Route packets, track failures, sweep
- NOT directly used by either of the above (legacy code)

### 4. `xrpld/ledger/src/acquisition/inbound_ledger.rs` (95KB!)
- `InboundLedgerLocal` — the per-ledger state machine
- Handles: planner, packet dispatch, SHAMap walk, completion
- KEEP THIS (it's the actual acquisition logic, not the registry)

## Rippled Architecture (the target)

```
ApplicationImp
  └── InboundLedgers (ONE instance)
        ├── acquire(hash, seq, reason) → shared_ptr<Ledger const>
        ├── acquireAsync(hash, seq, reason) → void
        ├── sweep() — remove idle entries after 60s
        ├── gotLedgerData() — route peer responses
        └── ledgers_: hash_map<uint256, shared_ptr<InboundLedger>>
              └── InboundLedger (per-ledger state machine)
                    ├── init() → checkLocal, then fetch from peers
                    ├── gotData() → process peer response
                    ├── runData() → job that processes buffered data
                    ├── isComplete() / isFailed()
                    └── getLedger() → completed ledger
```

Key behaviors:
- `acquire()` is synchronous: returns the ledger immediately if already complete
- `acquireAsync()` just triggers acquisition, doesn't block
- One entry per hash — never duplicated
- Entries stay alive as long as they're accessed (touch on acquire)
- Sweep removes entries idle >60s
- Failed entries stay to prevent re-acquisition loops
- `recentFailures_` with 5-minute cooldown prevents retry storms
- PeerSet manages which peers to ask (round-robin, cooldown)
- `runData()` posted as a job — no dedicated thread per acquisition

## New Architecture

```
xrpld/app/src/ledger/
├── mod.rs
├── inbound_ledgers/
│   ├── mod.rs              — public API: InboundLedgers struct + methods
│   ├── registry.rs         — entry map, dedup, sweep, failure tracking  
│   ├── worker_pool.rs      — fixed thread pool (8 workers), job submission
│   ├── acquisition.rs      — AcquisitionState: per-hash state + completion
│   ├── peer_set.rs         — peer selection, round-robin, cooldown
│   └── reason.rs           — Reason enum (Consensus, Generic, History)
```

### `mod.rs` — Public API
```rust
pub struct InboundLedgers { ... }

impl InboundLedgers {
    pub fn new(config) -> Self;
    
    /// Acquire a ledger. Returns immediately if already complete.
    /// If not tracked, starts acquisition. If in-progress, touches and returns None.
    pub fn acquire(&self, hash: Uint256, seq: u32, reason: Reason) -> Option<Arc<Ledger>>;
    
    /// Fire-and-forget acquire (for consensus/validation callers that don't need the result).
    pub fn acquire_async(&self, hash: Uint256, seq: u32, reason: Reason);
    
    /// Route a TMLedgerData response to the correct acquisition.
    pub fn got_ledger_data(&self, hash: &Uint256, peer_id: u64, packet: InboundLedgerPacket);
    
    /// Remove entries idle for >60s.
    pub fn sweep(&self);
    
    /// Number of in-progress acquisitions.
    pub fn active_count(&self) -> usize;
    
    /// Check if tracking a hash.
    pub fn contains(&self, hash: &Uint256) -> bool;
    
    /// Notify ledger completed (called by worker on finalization).
    pub fn on_complete(&self, hash: Uint256, ledger: Arc<Ledger>);
    
    /// Notify ledger failed.
    pub fn on_failed(&self, hash: Uint256);
    
    /// Stop all acquisitions.
    pub fn stop(&self);
}
```

### `registry.rs` — Entry Management
```rust
struct Entry {
    seq: u32,
    reason: Reason,
    state: Arc<AcquisitionState>,
    last_touched: Instant,
    started_at: Instant,
    result: Option<Arc<Ledger>>,  // Set when complete
    failed: bool,
}

struct Registry {
    entries: HashMap<Uint256, Entry>,
    recent_failures: HashMap<Uint256, Instant>,
}
```
- Single Mutex protecting the registry
- `acquire()` checks: recent_failure → already tracked (touch+return) → create new
- `sweep()` removes entries where `last_touched` > 60s ago
- `recent_failures` entries expire after 5 minutes

### `worker_pool.rs` — Job Execution
- 8 fixed threads (configurable via node_size)
- VecDeque<Job> + Condvar for wakeup
- Jobs are small ticks (process data, send requests, check completion)
- No dedicated thread per acquisition

### `acquisition.rs` — Per-Hash State
- Wraps `InboundLedgerLocal` (the existing state machine in xrpld/ledger)
- Holds: data_buffer, peer_set, node_store refs, completion flag
- `submit_tick()` pushes a job to worker_pool
- On completion: calls `InboundLedgers::on_complete`

### `peer_set.rs` — Peer Management
- Which peers to request from
- Round-robin selection
- Cooldown per peer (don't spam the same peer)
- Refresh peer list from overlay

### `reason.rs`
```rust
pub enum Reason {
    Consensus,  // RCLConsensus, validations, NetworkOPs
    Generic,    // LedgerMaster, catchup, publication
    History,    // History fill, sequential catchup
}
```

## Callers to Rewire

| Current caller | Current target | New call |
|---|---|---|
| `rcl_consensus.rs:382` | `shared.acquire_for_consensus(hash, seq)` | `inbound.acquire_async(hash, seq, Consensus)` |
| `rcl_consensus.rs:890` | `shared.acquire_for_consensus(hash, seq)` | `inbound.acquire_async(hash, seq, Consensus)` |
| `rcl_validation.rs:260` | `shared.acquire(hash, 0)` | `inbound.acquire_async(hash, 0, Consensus)` |
| `application_root.rs:1783` | `shared.acquire_for_consensus(hash, 0)` | `inbound.acquire_async(hash, 0, Consensus)` |
| `application_root.rs:4182` | `shared.acquire(hash, seq)` | `inbound.acquire_async(hash, seq, Generic)` |
| `application_root.rs:4435` | `shared.acquire_for_consensus(hash, seq)` | `inbound.acquire_async(hash, seq, Consensus)` |
| `bootstrap.rs:1515` | `shared_inbound.acquire(hash, 0)` | `inbound.acquire_async(hash, 0, Generic)` |
| `bootstrap.rs:1753` | `shared_inbound.acquire(hash, seq)` | `inbound.acquire_async(hash, seq, Generic)` |
| `main.rs:1859` | `inbound_ledgers.acquire(hash, seq, validated)` | `inbound.acquire_async(hash, seq, Generic)` |
| `main.rs:1922` | `inbound_ledgers.acquire(hash, seq, validated)` | `inbound.acquire_async(hash, seq, Generic)` |
| `main.rs:5039` | `inbound_ledgers.acquire(hash, seq, validated)` | `inbound.acquire_async(hash, seq, History)` |
| `overlay_impl.rs` | route_response to shared_inbound | `inbound.got_ledger_data(hash, peer_id, packet)` |

## Files to Delete

1. `xrpld/app/src/ledger/shared_inbound_ledgers.rs` — replaced entirely
2. `xrpld/ledger/src/acquisition/inbound_ledgers.rs` — legacy unused wrapper
3. `xrpld/main/src/main.rs` lines 2998-3550 — the duplicate InboundLedgers struct
4. Related tests that test the deleted code

## Files to KEEP (reuse)

1. `xrpld/ledger/src/acquisition/inbound_ledger.rs` — the per-ledger state machine (95KB)
   - InboundLedgerLocal, planner, packet dispatch, SHAMap walk
   - This is the CORE acquisition logic — no changes needed
2. `xrpld/ledger/src/acquisition/fetch_pack.rs` — fetch pack storage
3. `xrpld/ledger/src/acquisition/delta_acquire.rs` — delta acquisition
4. `xrpld/ledger/src/acquisition/skip_list_acquire.rs` — skip list traversal

## Implementation Order

1. Create `xrpld/app/src/ledger/inbound_ledgers/` directory with all sub-files
2. Implement `reason.rs`, `peer_set.rs`, `worker_pool.rs`, `registry.rs`, `acquisition.rs`, `mod.rs`
3. Wire into `ApplicationRoot` (replace `shared_inbound_ledgers` field)
4. Rewire all callers (consensus, validation, bootstrap, main.rs catchup)
5. Delete old files
6. Build and verify
7. Docker test against rippled cluster

## Key Design Decisions

- **Single Mutex** for the registry (matching rippled's single ScopedLockType)
- **Touch-on-access** keeps entries alive (matching rippled's lastAction)
- **60s sweep** removes stale entries (matching rippled)
- **5-minute failure cooldown** prevents retry storms (matching rippled)
- **8 worker threads** in pool (matching current SharedInboundLedgers)
- **acquire() returns Option<Arc<Ledger>>** — if complete, return immediately (matching rippled)
- **No per-acquisition thread spawn** — jobs submitted to pool
- **Completion notification** via channel to bootstrap loop (for catchup progress tracking)
