# Consensus Engine Redesign Spec (v2)

## Why: The Current Architecture Is Broken

The current consensus implementation fails to converge because it fragments
state across too many disconnected systems:

1. **Multiple stores**: LedgerMaster (AppLedgerMasterRuntime), SharedLedgerMasterState
   (ApplicationRoot), LedgerAcceptor trait, InboundLedgersLocal (app registry),
   InboundLedgers (main.rs). Acquired ledgers end up in one store but consensus
   looks in another.

2. **Multiple timers**: 200ms drain loop, 1s heartbeat thread, inline
   `maybe_tick_consensus` macro, main.rs catchup loop. These race, double-tick,
   or starve each other.

3. **Channel-based ledger delivery**: The bootstrap loop polls a channel
   (`completed_ledgers_rx`) every 50ms. The main.rs loop has its own polling.
   Neither is guaranteed to run when consensus needs the ledger.

4. **No sequence monotonicity enforcement**: Nodes validate every ledger they
   build (even solo builds), polluting the validation trie with stale fork hashes
   that cause other nodes to switch to unreachable ledgers.

5. **Proposal delivery latency**: Proposals are batched and drained once per
   heartbeat tick instead of being processed as they arrive.

---

## How: Rippled's Architecture (The Blueprint)

### Startup Sequence (Application.cpp)

```
setup():
  1. Initialize node store, databases, config
  2. Load/create genesis ledger → store in LedgerMaster → set as LCL
  3. Create OpenLedger from the initial LCL
  4. If startup mode is NETWORK: set needNetworkLedger = true
  5. Create Overlay object (but do NOT start it yet — no peers)
  6. Call beginConsensus(closedLedger.hash):
     a. prevLedger = LedgerMaster.getLedgerByHash(parentHash)
     b. updateTrusted() — rebuild UNL from current validators
     c. consensus.startRound(now, networkClosed, prevLedger, ...)
     ← consensus now in OPEN phase, waiting for heartbeat
  7. Setup RPC server handler
  8. If not standalone: start the heartbeat timer (setStateTimer)

start():
  9. Start the overlay (NOW peers can connect)
  10. First heartbeat fires ~1s later:
      a. Check peer count — likely 0 → set DISCONNECTED, skip timerEntry
      b. Eventually peers connect → peer count satisfied → timerEntry runs
```

**Key insight:** Consensus is initialized BEFORE any peers exist. The first
actual tick of the state machine only happens after peers connect and the
heartbeat detects sufficient peers.

### One Shared Cache (LedgerHistory / TaggedCache)

```
TaggedCache<LedgerHash, Ledger> — concurrent hash map with LRU expiry
├── Written by: doAccept() → buildLCL() → storeLedger()    [accept job]
├── Written by: InboundLedger::done() → storeLedger()      [acquisition]
├── Written by: application startup (genesis ledger)
└── Read by:    acquireLedger() → getLedgerByHash()         [consensus tick]
```

`getLedgerByHash` checks:
1. TaggedCache (LedgerHistory) — primary lookup
2. Current closed ledger field — secondary

No channels. No polling. The acquisition thread writes directly to the cache.
The consensus timer reads from the same cache 1 second later.

### One Timer (Heartbeat = 1s)

```
heartbeatTimer_ fires (1s via boost::asio)
  → addJob(jtNETOP_TIMER, "NetOPs.heartbeat")
    → processHeartbeatTimer() [on JobQueue worker thread]:
        1. Check peer count:
           if numPeers < minPeerCount:
             setMode(DISCONNECTED)
             reschedule timer
             RETURN  ← timerEntry is NOT called when disconnected!
        2. Mode promotions (DISCONNECTED → CONNECTED if enough peers)
        3. mConsensus.timerEntry(now)  ← takes consensus mutex internally
        4. setHeartbeatTimer() — reschedule
```

**Critical:** timerEntry is SKIPPED entirely when not enough peers. This
prevents nodes from closing ledgers solo and polluting the network.

### Proposal Processing (via JobQueue, NOT peer I/O thread)

```
Peer I/O thread:
  → decode TMProposeSet message
  → addJob(jtPROPOSAL_t, "recvPropose")  ← queue to JobQueue
    → checkPropose() [on JobQueue worker]:
        • Verify signature
        • Check if trusted (UNL membership)
        • mConsensus.peerProposal(now, pos)  ← takes consensus mutex
```

**Important:** The proposal does NOT block the peer I/O thread. It goes through
the JobQueue first. This means:
- Peer I/O never contends on the consensus mutex
- Multiple proposals serialize naturally through the mutex
- Proposals arrive within ~1ms (JobQueue processing is fast)
- `curr_peer_positions` is updated between heartbeat ticks

The quaxar equivalent: proposals can be processed directly from the overlay
message handler (more aggressive than rippled, lower latency) OR through a
small bounded channel to a dedicated thread. Both are valid.

### Async onAccept (Job Queue)

```
phaseEstablish() [under consensus mutex]:
  → consensus reached
  → phase_ = ConsensusPhase::accepted
  → adaptor.onAccept(result, ..., validating)
    → queues jtACCEPT job  [exits consensus mutex scope]

jtACCEPT job [NO consensus lock held]:
  → doAccept():
      1. Build new ledger (buildLCL → storeLedger into LedgerHistory)
      2. notify(neACCEPTED_LEDGER) — broadcast StatusChange to peers
      3. Censorship detection
      4. validate(built) — if validating_ && !consensusFail && canValidateSeq
      5. ledgerMaster_.consensusBuilt(built) — full ledger processing
      6. Rebuild open ledger (acquires MasterMutex + LedgerMaster::peekMutex)
      7. switchLCL(built) — update LedgerMaster's closed_ledger field
  → endConsensus():
      1. Cycle dead peer status (peers pointing to parent of last closed)
      2. checkLastClosedLedger(peers) → determine networkClosed:
         a. Collect peer LCL hash votes
         b. Ask validation trie: getPreferredLCL(lcl, minSeq, peerCounts)
         c. If preferred != ours AND ledger available: switchLastClosedLedger
         d. If preferred != ours AND NOT available: acquire it
      3. Mode promotion:
         CONNECTED/SYNCING + !ledgerChange + !needNetworkLedger → TRACKING
         CONNECTED/TRACKING + !ledgerChange + ledger is fresh → FULL
      4. beginConsensus(networkClosed):
         a. updateTrusted() — rebuild UNL
         b. consensus.startRound(now, networkClosed, prevLedger)
            ← takes consensus mutex briefly, resets to OPEN phase
```

**Lock ordering:** doAccept acquires MasterMutex and LedgerMaster::peekMutex
(for rebuilding the open ledger). These are NEVER held while the consensus
mutex is held. The consensus mutex is only taken by `startRound` at the very
end of the chain.

### Sequence Monotonicity (canValidateSeq)

```
canValidateSeq(seq):
  // SeqEnforcer: only allows strictly increasing sequences
  if now > (last_when + VALIDATION_SET_EXPIRES): last_seq = 0  // expire stale
  if seq <= last_seq: return false                              // monotonicity
  last_seq = seq
  last_when = now
  return true
```

Additionally, `preStartRound` sets `validating_` based on:
```
validating_ = has_validator_keys
    && prev_ledger.seq >= max_disallowed_ledger  // prevents re-validating after restart
    && !is_amendment_blocked
```

### acquireLedger: Cache Lookup + Async Fetch

```
acquireLedger(hash):
  // 1. Fast path: check LedgerHistory (TaggedCache)
  if ledger = ledgerMaster_.getLedgerByHash(hash):
    return Some(ledger)

  // 2. Slow path: trigger async fetch (dedup by hash)
  if acquiring_ != hash:
    acquiring_ = hash
    app_.getJobQueue().addJob(jtADVANCE, ..., || {
      app_.getInboundLedgers().acquireAsync(hash, 0, CONSENSUS)
    })

  return None  // consensus stays in WrongLedger, retries next tick
```

When InboundLedger completes:
```
InboundLedger::done()
  → LedgerMaster::storeLedger(mLedger_)
    → mLedgerHistory.insert(ledger, validated)  // into TaggedCache
```

Next `timerEntry` (≤1s): `acquireLedger(hash)` → `getLedgerByHash` → FINDS IT.

### get_prev_ledger (SIMPLE — 5 lines)

```cpp
RCLConsensus::Adaptor::getPrevLedger(ledgerID, ledger, mode) {
    uint256 netLgr = vals.getPreferred(
        RCLValidatedLedger{ledger},
        ledgerMaster_.getValidLedgerIndex());

    if (netLgr != ledgerID) {
        if (mode != ConsensusMode::wrongLedger)
            app_.getOPs().consensusViewChange();  // demotes FULL/TRACKING → CONNECTED
    }
    return netLgr;
}
```

NO peer voting. NO `valid_ledger_index==0` guard. NO overlay queries.
Just: ask the validation trie, return result.

`getPreferred(curr, minValidSeq)` returns `curr.id()` when:
- Trie is empty (no validations received yet)
- Preferred seq < minValidSeq

This means: during early bootstrap with no validations, `get_prev_ledger`
always returns the current ledger (no switch). Correct behavior.

### getPreferredLCL (peer voting — ONLY in checkLastClosedLedger)

```cpp
getPreferredLCL(lcl, minSeq, peerCounts) {
    auto preferred = getPreferred(lcl);
    if (preferred && preferred.seq >= minSeq)
        return preferred.second;  // validation trie wins

    // No trusted validations — fall back to peer majority vote
    auto it = std::max_element(peerCounts, by_count_then_hash);
    if (it != end && it->count > 0) return it->id;
    return lcl.id();  // no info → stay on current
}
```

This peer fallback is ONLY called from `endConsensus` → `checkLastClosedLedger`.
It is NOT called from `getPrevLedger` (which the consensus algorithm calls every
tick). These are separate code paths with different purposes:
- `getPrevLedger`: "is the network on a different ledger?" (trie-only)
- `checkLastClosedLedger`: "what should I start the next round on?" (trie + peers)

### consensusViewChange

```cpp
void NetworkOPsImp::consensusViewChange() {
    if ((mMode == OperatingMode::FULL) || (mMode == OperatingMode::TRACKING))
        setMode(OperatingMode::CONNECTED);
}
```

Demotes to CONNECTED. Called from `getPrevLedger` when the network prefers a
different ledger AND we're not already in WrongLedger mode. Only fires ONCE
per wrong-ledger detection (mode suppresses repeats).

### needNetworkLedger

- Set in `Application::setup()` when startup mode requires syncing
- Cleared by `switchLastClosedLedger()` (after successfully acquiring network LCL)
- Guards mode promotion: can't reach TRACKING/FULL while set
- Does NOT affect consensus algorithm — only operating mode

---

## The Redesigned Quaxar Architecture

### Rust-Specific Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Consensus mutex | `parking_lot::Mutex` | No poisoning, smaller, fairer under contention |
| LedgerHistory cache | `DashMap<Hash, Arc<Ledger>>` | Concurrent shard-locked reads without full lock |
| Accept worker | Dedicated thread + `crossbeam_channel::bounded(1)` | Bounded, no spawn overhead, clean shutdown |
| Operating mode | `AtomicU8` | Read-heavy (every RPC, every proposal), rarely written |
| last_validated_seq | `AtomicU32` with Release/Acquire | Lock-free monotonicity check |

### Module Boundaries

```
xrpld/consensus/            — Generic algorithm (KEEP, minor fixes only)
  algorithm/consensus.rs    — State machine: Open/Establish/Accepted
  algorithm/params.rs       — Consensus parameters
  algorithm/timing.rs       — Close time resolution
  algorithm/types.rs        — ConsensusTimer, ConsensusResult, etc.
  model/disputed_tx.rs      — Transaction disputes
  model/ledger_trie.rs      — Validation trie
  model/proposal.rs         — ConsensusProposal
  rcl_support/rcl.rs        — RCL types, RclConsensus wrapper
  rcl_support/validations.rs — RclValidations + SeqEnforcer

xrpld/app/src/consensus/    — Application-layer consensus
  driver.rs                 — NEW: ConsensusDriver (the single entry point)
  adaptor.rs                — Simplified RCL adaptor
  rcl_validations.rs        — Keep existing validation bridge
  (other files stay for types/helpers)

xrpld/app/src/bootstrap/    — SIMPLIFIED
  bootstrap.rs              — Start consensus once, then just service peers
```

### Core Design: `ConsensusDriver`

```rust
use parking_lot::Mutex;
use crossbeam_channel::{Sender, bounded};

pub struct ConsensusDriver {
    /// The consensus state machine
    engine: Mutex<RclConsensus<AppAdaptor>>,

    /// Shared ledger cache — THE single source of truth
    ledger_cache: Arc<DashMap<Uint256, Arc<Ledger>>>,

    /// Channel to the dedicated accept worker thread
    accept_tx: Sender<AcceptWork>,

    /// Operating mode (DISCONNECTED/CONNECTED/TRACKING/FULL)
    mode: AtomicU8,

    /// Last locally-validated sequence (canValidateSeq)
    last_validated_seq: AtomicU32,

    /// Overlay reference for peer count checks + status broadcasts
    overlay: Arc<OverlayImpl>,

    /// Validator keys (for validation emission)
    validator_keys: ValidatorKeys,

    /// Minimum peer count before consensus ticks (matches rippled)
    min_peer_count: usize,
}

impl ConsensusDriver {
    /// Called by the heartbeat thread every 1 second.
    /// Matches rippled's processHeartbeatTimer exactly.
    pub fn heartbeat(&self, now: NetClockTimePoint) {
        let peer_count = self.overlay.active_peer_count();

        // Gate: don't tick consensus without peers (prevents solo closing)
        if peer_count < self.min_peer_count {
            self.set_mode(OperatingMode::Disconnected);
            return;
        }

        // Mode promotion: DISCONNECTED → CONNECTED
        if self.mode() == OperatingMode::Disconnected {
            self.set_mode(OperatingMode::Connected);
        }

        // Tick the consensus state machine
        let mut engine = self.engine.lock();
        engine.timer_entry(now);

        // If round completed (phase == accepted), dispatch accept work
        if let Some(work) = engine.adaptor_mut().take_pending_accept() {
            drop(engine); // RELEASE lock before sending to worker
            let _ = self.accept_tx.send(work);
        }
    }

    /// Called from overlay peer handler when a trusted proposal arrives.
    /// Takes the mutex briefly to update peer positions.
    pub fn peer_proposal(&self, now: NetClockTimePoint, pos: RclCxPeerPos) -> bool {
        let mut engine = self.engine.lock();
        engine.peer_proposal(now, pos)
    }

    /// Called from overlay when a transaction set is received.
    pub fn got_tx_set(&self, now: NetClockTimePoint, txset: Vec<RclCxTx>) {
        let mut engine = self.engine.lock();
        engine.got_tx_set(now, txset);
    }

    /// Start the first consensus round (called once at startup, before overlay)
    pub fn begin_consensus(&self, now: NetClockTimePoint, closed_hash: Uint256, prev: RclCxLedger) {
        let mut engine = self.engine.lock();
        engine.start_round(now, closed_hash, prev);
    }
}
```

### Accept Worker Thread (replaces jtACCEPT job)

```rust
fn accept_worker(
    rx: crossbeam_channel::Receiver<AcceptWork>,
    driver: Arc<ConsensusDriver>,
    ledger_cache: Arc<DashMap<Uint256, Arc<Ledger>>>,
) {
    while let Ok(work) = rx.recv() {
        // 1. Build ledger from consensus result (expensive, outside mutex)
        let built = build_ledger_from_consensus(&work);

        // 2. Store in shared cache (available to acquire_ledger immediately)
        let hash = *built.header().hash.as_uint256();
        ledger_cache.insert(hash, Arc::clone(&built));

        // 3. Broadcast StatusChange (neACCEPTED_LEDGER) to peers
        driver.overlay.broadcast_status_change(NeAcceptedLedger, &built);

        // 4. Emit validation (if canValidateSeq passes)
        let seq = built.header().seq;
        let consensus_fail = work.state == ConsensusState::MovedOn;
        if driver.is_validating()
            && !consensus_fail
            && seq > driver.last_validated_seq.load(Ordering::Acquire)
        {
            driver.last_validated_seq.store(seq, Ordering::Release);
            driver.emit_validation(&built);
        }

        // 5. endConsensus: mode promotion + start next round
        driver.end_consensus(&built);
    }
}
```

### end_consensus (matches rippled's endConsensus exactly)

```rust
impl ConsensusDriver {
    fn end_consensus(&self, built: &Arc<Ledger>) {
        // 1. Cycle dead peer status
        let dead_hash = built.header().parent_hash;
        for peer in self.overlay.active_peers() {
            if peer.closed_ledger_hash() == dead_hash {
                peer.cycle_status();
            }
        }

        // 2. checkLastClosedLedger: determine networkClosed
        let our_closed = *built.header().hash.as_uint256();
        let mut peer_counts: HashMap<Uint256, u32> = HashMap::new();
        if self.mode() >= OperatingMode::Tracking {
            *peer_counts.entry(our_closed).or_default() += 1;
        }
        for peer in self.overlay.active_peers() {
            let h = peer.closed_ledger_hash();
            if !h.is_zero() {
                *peer_counts.entry(h).or_default() += 1;
            }
        }
        let network_closed = {
            let engine = self.engine.lock();
            let adaptor = engine.adaptor();
            adaptor.validations().get_preferred_lcl(built, peer_counts)
        };

        let ledger_change = network_closed != our_closed;

        // 3. Mode promotion (matching rippled exactly)
        if !ledger_change && !self.need_network_ledger() {
            let mode = self.mode();
            if mode == OperatingMode::Connected || mode == OperatingMode::Syncing {
                self.set_mode(OperatingMode::Tracking);
            }
            let mode = self.mode();
            if mode == OperatingMode::Connected || mode == OperatingMode::Tracking {
                if ledger_is_fresh(built) {
                    self.set_mode(OperatingMode::Full);
                }
            }
        }

        // 4. beginConsensus(networkClosed)
        if network_closed.is_zero() { return; }
        let prev_ledger = self.ledger_cache.get(&network_closed)
            .map(|r| r.value().clone());
        if let Some(prev) = prev_ledger {
            let now = net_clock_now();
            let prev_cx = consensus_ledger_from(&prev);
            let mut engine = self.engine.lock();
            // Update trusted validator set (UNL may change between rounds)
            engine.adaptor_mut().update_trusted();
            engine.start_round(now, network_closed, prev_cx);
        }
    }
}
```

### Heartbeat Thread

```rust
fn spawn_heartbeat(driver: Arc<ConsensusDriver>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("consensus-heartbeat".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if stop.load(Ordering::Acquire) { break; }
                driver.heartbeat(net_clock_now());
            }
        })
        .expect("spawn heartbeat thread")
}
```

### InboundLedger Completion (direct cache insert)

```rust
// In the acquisition worker thread, after ledger is fully assembled:
fn on_acquisition_complete(
    ledger: Arc<Ledger>,
    cache: &Arc<DashMap<Uint256, Arc<Ledger>>>,
) {
    let hash = *ledger.header().hash.as_uint256();
    cache.insert(hash, ledger);
    // No notification needed. Next timerEntry (≤1s) will find it via
    // checkLedger → handleWrongLedger → acquireLedger → cache.get(hash)
}
```

### get_prev_ledger (matching rippled — 5 lines)

```rust
fn get_prev_ledger(&self, prev_id: &Uint256, prev_ledger: &RclCxLedger, mode: ConsensusMode) -> Uint256 {
    let preferred = self.validations.get_preferred_with_min_seq(
        validated_ledger_from(prev_ledger),
        self.valid_ledger_index(),
    );
    if preferred != *prev_id && mode != ConsensusMode::WrongLedger {
        self.consensus_view_change(); // FULL/TRACKING → CONNECTED
    }
    preferred
}
```

No peer voting (that's in `end_consensus`). No `valid_ledger_index==0` guard
(trie returns `curr.id()` when empty). No overlay queries.

### acquire_ledger (matching rippled)

```rust
fn acquire_ledger(&mut self, hash: &Uint256) -> Option<RclCxLedger> {
    // 1. Fast path: check shared cache
    if let Some(ledger) = self.ledger_cache.get(hash) {
        self.acquiring = None;
        return Some(consensus_ledger_from(ledger.value()));
    }

    // 2. Slow path: trigger async fetch (dedup)
    if self.acquiring.as_ref() != Some(hash) {
        self.acquiring = Some(*hash);
        self.trigger_ledger_fetch(hash); // sends TmGetLedger to peers
    }

    None // consensus stays in WrongLedger, retries next heartbeat
}
```

---

## Bootstrap Sequence for Quaxar

```
1. Load config, initialize node store
2. Create/load genesis ledger → insert into ledger_cache
3. Create OpenLedger from genesis
4. Create OverlayImpl (but do NOT start listening)
5. Create ConsensusDriver with all dependencies
6. driver.begin_consensus(now, genesis_hash, genesis_cx_ledger)
   ← consensus is now in OPEN phase
7. Spawn accept worker thread
8. Spawn heartbeat thread
9. Start overlay (peers can now connect)
10. Bootstrap loop: only handles peer I/O, ledger requests, RPC
    (NO consensus logic — that's all in the driver + heartbeat)
```

---

## Algorithm Fixes (in xrpld/consensus/src/algorithm/consensus.rs)

### 1. Remove `skip_check_ledger`

Delete the field entirely. `timer_entry` calls `check_ledger` unconditionally
every tick. Rippled does this — there is no skip mechanism.

### 2. Add data/hash mismatch check in `check_ledger`

```rust
fn check_ledger(&mut self) {
    let network_ledger = self.adaptor.get_prev_ledger(&prev_ledger_id, &previous_ledger, self.mode);
    if network_ledger != prev_ledger_id {
        self.handle_wrong_ledger(network_ledger);
    } else if self.adaptor.id(&previous_ledger) != prev_ledger_id {
        self.handle_wrong_ledger(network_ledger);
    }
}
```

### 3. No other algorithm changes needed

The generic `Consensus` state machine is correct. `shouldCloseLedger`,
`checkConsensus`, `phaseOpen`, `phaseEstablish`, `handleWrongLedger` — all
match rippled's logic. The bugs were in the APPLICATION layer (timer, proposal
delivery, ledger cache, validation emission), not in the algorithm itself.

---

## Race Condition Analysis

### Race 1: Accept worker vs heartbeat (both want consensus mutex)

- Accept worker: calls `engine.start_round()` at the end of `end_consensus`
- Heartbeat: calls `engine.timer_entry()` every 1s

**Safe because:** After `phase_ = accepted`, `timer_entry` is a no-op (returns
immediately). The accept worker is the ONLY thread that can reset the phase
via `start_round`. No race — they serialize naturally through the mutex.

### Race 2: Peer proposals arriving during accept

- Peer proposal: calls `engine.peer_proposal()`
- Accept worker: has NOT yet called `start_round`

**Safe because:** `peer_proposal` checks `if phase == accepted { return false }`.
Proposals for the old round are dropped. After `start_round` resets to Open,
new proposals are `playback`ed from `recent_peer_positions`.

### Race 3: Multiple heartbeats before accept completes

- Heartbeat fires at t=1s, t=2s, t=3s while accept is still building

**Safe because:** `timer_entry` sees `phase == accepted`, returns immediately.
Only when accept calls `start_round` (which resets phase to Open) will
`timer_entry` do actual work on the next heartbeat.

### Race 4: LedgerHistory insert (accept) vs read (acquire_ledger)

- Accept worker: `cache.insert(hash, ledger)`
- Heartbeat: `cache.get(hash)` in `acquire_ledger`

**Safe because:** `DashMap` operations are atomic at the shard level. An
`insert` followed by a `get` on another thread is guaranteed to see the value.

---

## Estimated Effort

| Phase | New Lines | Deleted Lines | Files Touched |
|-------|-----------|---------------|---------------|
| Phase 1: ConsensusDriver | ~300 | 0 | 1 new file |
| Phase 2: Direct proposals | ~20 | ~100 | overlay peer handler, remove PendingProposal |
| Phase 3: Heartbeat | ~30 | ~200 | bootstrap.rs (remove timer mess) |
| Phase 4: Cache unification | ~20 | ~150 | main.rs (remove channels), acquisition |
| Phase 5: Algorithm fixes | ~10 | ~15 | consensus.rs |
| Phase 6: Bootstrap simplify | ~50 | ~300 | bootstrap.rs |
| **Total** | **~430** | **~765** | Net: simpler by 335 lines |

---

## Summary: What Makes This Different from the Current Implementation

The current implementation tries to be "event-driven" with channels, async
runtimes, and polling loops. Rippled is NOT event-driven for consensus — it's
**timer-driven with a shared cache**. The timer fires, checks state, does work.
Between ticks, proposals arrive and update state through the mutex. Ledgers
arrive and land in the cache. Everything is discovered by polling on the next
tick. No callbacks, no notifications, no channels.

This simplicity is what makes rippled's consensus reliable. The redesign
adopts this exact model:
- **Timer-driven**: 1 second heartbeat, checks everything
- **Shared cache**: One `DashMap` for ledgers, always consistent
- **Mutex-serialized**: One `parking_lot::Mutex` for the state machine
- **Immediate proposals**: Lock, update, release (no batching delay)
- **Async accept**: Heavy work outside the lock, natural serialization via channel
