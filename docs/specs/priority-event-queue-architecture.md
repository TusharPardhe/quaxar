# Priority Event Queue Architecture — Consensus Strand Improvement

## Status: Design Specification (not yet implemented)

## Problem Statement

Quaxar's consensus strand runs all consensus-critical and transaction-processing
work on a single thread in a sequential loop. While this simplifies reasoning
about state (no locks needed), it creates priority inversion risks:

1. Transaction application (low priority) can delay consensus timer ticks (high priority)
2. Unbounded channel drains can starve later loop stages
3. Background work (history fetch, sweep) competes with consensus-critical operations

rippled solves this with a multi-threaded JobQueue where each job type has a
priority, max concurrency limit, and timeout. The heartbeat timer (JtNetopTimer)
runs on its own job with guaranteed scheduling independent of transaction batch
processing (JtBatch).

## Current Quaxar Architecture

```
Thread: networkops-strand (single thread)
Channels:
  - command_rx: mpsc::Receiver<ConsensusCommand>     (StartRound, Stop)
  - proposal_rx: mpsc::Receiver<QueuedProposal>      (peer proposals)
  - txset_rx: mpsc::Receiver<(Uint256, TxSet)>       (completed tx sets)
  - shared_completed_rx: mpsc::Receiver<Arc<Ledger>> (acquired ledgers)

Loop structure:
  1. Drain commands (unbounded)
  2. Mode demotion check
  3. Drain proposals (unbounded)
  4. Drain tx-sets (unbounded)
  5. Timer tick (1s interval check)
  6. Accept phase → start next round
  7. checkAccept + advance
  8. Drain completed ledgers
  9. History fetch tick (10s interval)
  10. Sleep 10ms if idle

Separate threads (already exist):
  - Overlay peer I/O threads (read/write network)
  - Acquisition worker pool (64 threads for ledger data)
  - Timer service thread (acquisition timeouts)
  - Transaction router (called on overlay threads, feeds TxQ)
```

## rippled's Architecture (for reference)

```
JobQueue with typed priorities:
  JtNetopTimer (heartbeat)     — priority 1, max 1 concurrent, 999ms timeout
  JtBatch (tx application)     — priority maxLimit, 250ms timeout
  JtAdvance (ledger advance)   — priority maxLimit, immediate
  JtLedgerData (acquisition)   — priority 3, immediate
  JtTransaction (single tx)    — priority maxLimit, 250ms timeout
  JtSweep (cache maintenance)  — priority 1, immediate
  JtClient* (RPC)              — priority maxLimit, 2000ms timeout

Key property: JtNetopTimer (heartbeat) has max=1 and always gets scheduled
independently. JtBatch can't starve it because they're on different workers.
```

## Proposed Architecture: Priority-Aware Event Loop

### Design Principles

1. **Single logical thread for consensus state** — no locks on consensus data
2. **Priority dispatch within the loop** — P0 always runs before P3
3. **Bounded work per priority level per iteration** — prevents starvation
4. **Async I/O offloaded to worker threads** — results fed back as events
5. **Cooperative yielding** — long-running work checks for higher-priority interrupts

### Priority Levels

```
P0: CRITICAL (consensus correctness, max latency: 10ms)
  - timer_tick() — 1-second consensus heartbeat
  - check_ledger() / switchLCL detection
  - mode transitions (WrongLedger, Observing)
  - start_round() command
  - stop() command

P1: HIGH (consensus participation, max latency: 50ms)
  - peer_proposal processing
  - validation relay/acceptance
  - got_tx_set processing
  - execute_accept (ledger close finalization)

P2: NORMAL (ledger state advancement, max latency: 200ms)
  - completed_inbound_ledger arrival
  - check_accept + advance
  - publish new ledger

P3: LOW (throughput work, max latency: 1s)
  - apply_network_ops_pending_to_open_ledger (transaction application)
  - transaction relay results
  - fee change notifications

P4: BACKGROUND (housekeeping, runs only when idle)
  - history_fetch_tick
  - cache_sweep
  - peer endpoint broadcast
  - metrics collection
```

### Event Loop Structure

```rust
use std::collections::VecDeque;
use std::time::{Duration, Instant};

struct PriorityEventQueue {
    queues: [VecDeque<ConsensusEvent>; 5],  // P0..P4
    timer_due: Instant,
}

impl PriorityEventQueue {
    fn run_iteration(&mut self, state: &mut ConsensusState) {
        // ── Always check timer first (P0 injection) ──
        if Instant::now() >= self.timer_due {
            self.queues[0].push_back(ConsensusEvent::TimerTick);
            self.timer_due = Instant::now() + Duration::from_secs(1);
        }

        // ── Drain channels into priority queues ──
        self.drain_channels_bounded();

        // ── Process by priority with bounded work per level ──
        // P0: drain ALL (these are critical, always few items)
        while let Some(event) = self.queues[0].pop_front() {
            state.handle_critical(event);
        }

        // P1: max 32 per iteration
        for _ in 0..32 {
            let Some(event) = self.queues[1].pop_front() else { break };
            state.handle_high(event);
            // Yield to P0 if new critical event arrived
            if !self.queues[0].is_empty() { break; }
        }

        // P2: max 16 per iteration
        for _ in 0..16 {
            let Some(event) = self.queues[2].pop_front() else { break };
            state.handle_normal(event);
            if !self.queues[0].is_empty() || !self.queues[1].is_empty() { break; }
        }

        // P3: max 64 per iteration (transaction batches)
        for _ in 0..64 {
            let Some(event) = self.queues[3].pop_front() else { break };
            state.handle_low(event);
            if !self.queues[0].is_empty() || !self.queues[1].is_empty() { break; }
        }

        // P4: only when higher queues empty
        if self.queues[0..4].iter().all(|q| q.is_empty()) {
            if let Some(event) = self.queues[4].pop_front() {
                state.handle_background(event);
            }
        }
    }

    fn drain_channels_bounded(&mut self) {
        // Drain each channel into its priority queue with a per-drain cap
        // to prevent a flooded channel from monopolizing the drain phase
        for _ in 0..64 {
            match self.command_rx.try_recv() {
                Ok(cmd) => self.queues[0].push_back(cmd.into()),
                Err(_) => break,
            }
        }
        for _ in 0..64 {
            match self.proposal_rx.try_recv() {
                Ok(prop) => self.queues[1].push_back(prop.into()),
                Err(_) => break,
            }
        }
        // ... etc for each channel
    }
}
```

### Transaction Application: Separate Worker with Feedback

```
┌──────────────────────────────────────────────────────────────┐
│ MAIN LOOP (consensus strand)                                  │
│                                                                │
│  Owns: consensus state, phase, mode, prev_ledger              │
│  Never blocked by: tx application, NuDB reads, RPC            │
│                                                                │
│  on_close() →  sends TxBatchRequest to tx_worker_tx channel  │
│              ←  receives TxBatchResult from tx_result_rx      │
│                                                                │
└──────────────────────────────────────────────────────────────┘
        │                              ▲
        │ TxBatchRequest               │ TxBatchResult
        ▼                              │
┌──────────────────────────────────────────────────────────────┐
│ TX WORKER THREAD (matches rippled's JtBatch)                  │
│                                                                │
│  Receives: list of transactions to apply                      │
│  Does: apply each to OpenLedger (with catch_unwind)           │
│  Returns: applied count, fee changes, relay decisions         │
│                                                                │
│  Can be slow (panics, NuDB reads) without affecting consensus │
└──────────────────────────────────────────────────────────────┘
```

### Key Difference from rippled's JobQueue

| Aspect | rippled JobQueue | Proposed Priority Loop |
|--------|-----------------|----------------------|
| Threading | Many threads, mutex-protected shared state | Single consensus thread + dedicated tx worker |
| Priority | Implicit (thread scheduling) | Explicit (array of queues, strict ordering) |
| Starvation | Possible under load (thread contention) | Impossible (P0 always drains first) |
| Complexity | High (locks, condition vars, thread pool sizing) | Low (single loop, channel-based communication) |
| Latency guarantee | Probabilistic (depends on OS scheduler) | Deterministic (bounded work per level) |

### Migration Path

Phase 1 (current — already done):
  - Batch limit on tx application (256 per cycle)
  - WrongLedger early return in timer_entry
  - These provide correctness without architectural change

Phase 2 (priority ordering within existing loop):
  - Reorder the strand loop to check timer FIRST
  - Add bounded drain caps to proposal/txset channels
  - Move history fetch and sweep to only run when idle
  - Estimated: 1-2 days, low risk

Phase 3 (separate tx worker thread):
  - Extract apply_network_ops_pending into a dedicated thread
  - Communicate via channel (request/response pattern)
  - on_close sends tx list, waits for result (bounded timeout)
  - If worker is slow, consensus can proceed without it
  - Estimated: 3-5 days, medium risk

Phase 4 (full priority event queue):
  - Replace the sequential loop with PriorityEventQueue
  - All incoming data (proposals, validations, txsets, ledgers) routed to priority queues
  - Each priority level has explicit bounded processing per iteration
  - Add metrics: queue depths, processing time per level, yield counts
  - Estimated: 1-2 weeks, medium risk

### Files to Modify

| File | Phase | Change |
|------|-------|--------|
| `xrpld/app/src/network/network_ops_strand.rs` | 2-4 | Main loop restructure |
| `xrpld/app/src/network/network_ops_runtime.rs` | 3 | Extract tx application to worker |
| `xrpld/app/src/network/network_ops.rs` | 3 | Channel between strand and tx worker |
| `xrpld/app/src/consensus/rcl_consensus.rs` | 3-4 | on_close uses async tx result |
| `xrpld/app/src/state/application_root.rs` | 3 | apply_pending becomes async |
| NEW: `xrpld/app/src/network/priority_queue.rs` | 4 | PriorityEventQueue implementation |
| NEW: `xrpld/app/src/network/tx_worker.rs` | 3 | Dedicated tx application thread |

### Metrics to Track After Implementation

- `consensus_timer_tick_latency_us` — time between scheduled and actual tick (target: < 10ms)
- `p0_queue_depth` — should always be 0 or 1
- `p1_queue_depth` — proposals waiting, should be < 50
- `p3_queue_depth` — transactions waiting, can be high under load
- `tx_worker_batch_time_ms` — how long tx application takes per batch
- `priority_yields_per_second` — how often lower priority yields to higher

### Comparison with Other Blockchains

| Blockchain | Consensus isolation | Tx processing | Priority model |
|-----------|--------------------|--------------|----|
| rippled | Separate JtNetopTimer job | JtBatch on pool | Implicit (OS threads) |
| Solana | Dedicated PoH thread | Banking stage thread pool | Explicit pipeline stages |
| Reth | Tokio task with priority channel | Separate execution task | Channel priority drain |
| Aptos | Leader thread isolated | Block-STM parallel workers | Scheduler-based |
| **Quaxar (proposed)** | P0 in priority loop | Dedicated tx worker thread | Explicit priority array |

### Why This is Better Than rippled

1. **Deterministic latency** — rippled's timer can be delayed by thread contention;
   our P0 is guaranteed to drain first every iteration
2. **Simpler reasoning** — one thread owns consensus state, no mutex dance
3. **Observable** — priority queue depths directly show system health
4. **Tunable** — per-priority batch limits are config-driven, not hardcoded
5. **Rust-safe** — no shared mutable state between consensus and tx threads
   (channel-based communication only)
