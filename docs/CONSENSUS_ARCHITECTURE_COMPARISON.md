# Consensus Architecture Comparison: rippled vs rxrpl vs quaxar

## Executive Summary

The root cause of quaxar's divergence under multi-node load is a **timing misalignment in shouldCloseLedger across nodes**, caused by the relay→apply pipeline running on a different thread than the consensus timer. This creates a ~50ms to 3+ second window where different nodes have different open-ledger content at close time.

---

## rippled Architecture

### Transaction Relay Path
```
Network strand (peer):
  PeerImp::onMessage(TMTransaction)
    → NetworkOPs::processTransaction(tx, trusted=true, local=false)
      → doTransactionAsync(tx)
        → mutex_.lock()
        → transactions_.push_back(tx)     // O(1) queue add
        → if dispatchState_ == None:
            jobQueue_.addJob(JtBatch, "TxBatchAsync", transactionBatch)
            dispatchState_ = Scheduled
        → mutex_.unlock()                 // network strand FREE immediately
```

### Batch Apply Path (separate worker)
```
Job queue worker:
  transactionBatch()
    → mutex_.lock()
    → transactions_.swap(local_batch)
    → dispatchState_ = Running
    → mutex_.unlock()
    → masterMutex.lock() + ledgerMutex.lock()
    → openLedger.modify([&](view) {
        for tx in local_batch:
          txQ.apply(tx)                   // ALL pending in ONE modify
      })
    → masterMutex.unlock()
```

### Consensus Timer Path (1 second)
```
Timer fires (every 1s = ledgerGRANULARITY):
  NetworkOPsImp::processHeartbeatTimer()
    → masterMutex.lock()
    → mode checks, peer count checks
    → masterMutex.unlock()                // UNLOCKED before timerEntry!
    → consensus_.timerEntry(closeTime)
      → phaseOpen()
        → anyTransactions = openLedger.current()->txs.size() > 0
        → proposersClosed = curr_peer_positions.len()
        → shouldCloseLedger(...)
          → if proposersClosed > prevProposers/2: CLOSE (fast path!)
          → if !anyTransactions: wait idle_interval
          → if openTime < prevRoundTime/2: wait
          → else: CLOSE
```

### Proposal Path (immediate)
```
Network strand (peer):
  PeerImp::onMessage(TMProposeSet)
    → NetworkOPs::processTrustedProposal(peerPos)
      → consensus_.peerProposal(now, peerPos)  // DIRECTLY updates curr_peer_positions
                                                // NO queuing, NO job, IMMEDIATE
```

### Key Serialization Points
1. **transactionBatch holds masterMutex while modifying open ledger** → timerEntry waits if batch is in progress
2. **timerEntry does NOT hold masterMutex** during consensus logic → batch can run during establish phase
3. **peerProposal is called directly from network strand** → curr_peer_positions updates immediately → next timerEntry sees it

### Why rippled gets 0 disputes
- Relay arrives → queued → JtBatch fires within ~1ms → openLedger.modify applies all
- By the time timerEntry fires (up to 1s later), ALL relay transactions are in the open ledger
- ALL nodes have the same open ledger when they close because relay+apply completes within the 3s round
- proposersClosed fast-path ensures nodes close within ~100ms of each other even if timing drifts

---

## rxrpl Architecture

### Single Event Loop (tokio select!)
```
loop {
  tokio::select! {
    _ = tick_interval.tick() => {
      if timer.tick() == CloseLedger:
        tx_set = collect_consensus_tx_set(&ledger)  // reads ledger.tx_map
        consensus.close_ledger(tx_set)
      if timer.tick() == Converge:
        consensus.converge()
    }
    
    Some(msg) = consensus_rx.recv() => {
      match msg {
        ConsensusMessage::Transaction { hash, data } => {
          tx_engine.apply(&tx_json, &mut ledger)  // WRITES to ledger.tx_map
          tx_queue.submit(entry)
        }
        ConsensusMessage::Proposal(proposal) => {
          consensus.peer_proposal(proposal)       // updates peer_positions
        }
        ConsensusMessage::TxSetAcquired(set) => {
          // store for acquire_tx_set lookup
        }
      }
    }
  }
}
```

### Key Design
- **EVERYTHING on one async task** — no threading, no mutexes needed
- Transaction apply, timer tick, proposal processing: all sequential in one select! loop
- When CloseLedger fires, ALL previously received transactions are ALREADY in `ledger.tx_map`
- **Zero race condition by construction** — no two operations can interleave

### Timer: 100ms poll cadence
- `tick_interval = tokio::time::interval(Duration::from_millis(100))`
- Checks `timer.tick()` which returns CloseLedger when `open_duration` elapsed
- Default open_duration: ~2-5 seconds (adaptive based on previous convergence time)

### Proposal handling
- Peer proposals arrive via `consensus_rx` channel (mpsc from PeerManager)
- Processed in the SAME select loop → `peer_proposal()` updates state immediately
- Next timer tick sees updated `peer_positions` → shouldClose fires if peers closed

### Why rxrpl gets 0 disputes
- Single event loop = no interleaving
- When timer fires CloseLedger, ALL previously received relay transactions are in ledger.tx_map
- collect_consensus_tx_set reads the SAME ledger that was modified by relay processing
- No window for "in-flight" transactions to be missed

---

## quaxar Architecture (current)

### Transaction Relay Path
```
Network thread:
  overlay inbound handler receives TMTransaction
    → router callback fires (network thread)
      → process_transaction(tx)           // adds to PENDING QUEUE (O(1))
      → apply_network_ops_pending()       // calls openLedger.modify() PER TX
      → notify_tx_pending()               // wake loop thread
```

### Loop Thread (bootstrap loop, 50ms + condvar wake)
```
Loop thread:
  wait_tx_or_timeout(50ms)                // wakes on condvar or timeout
  drain_proposals()                       // peer_proposal for each queued proposal
  drain overlay queue + process + apply   // batch apply leftover
  maybe_tick_consensus!()                 // calls timer_tick → phase_open/establish
```

### Consensus Timer (1 second gate)
```
maybe_tick_consensus:
  if elapsed >= 1s:
    timer_tick(now, run_timer=true)
      → drain proposals
      → runner.timer_tick(now)
        → phase_open()
          → shouldCloseLedger(...)
        → phase_establish()
          → update_our_positions()
          → have_consensus()
```

### Proposal Path (QUEUED, not immediate)
```
Network thread:
  overlay inbound handler receives TMProposeSet
    → queued to pending_proposals vec (mutex-protected)

Loop thread (every 50ms):
  drain_proposals()
    → timer_tick(now, run_timer=false)
      → drain pending_proposals
      → for each: runner.peer_proposal()  // updates curr_peer_positions
      → does NOT run phase_open/shouldCloseLedger
```

---

## Critical Differences

| Aspect | rippled | rxrpl | quaxar |
|--------|---------|-------|--------|
| Relay → open ledger | ~1ms (JtBatch) | ~0ms (same loop) | ~1ms (per-tx apply in router) |
| Proposal → curr_peer_positions | IMMEDIATE (network strand) | IMMEDIATE (same loop) | **DELAYED up to 50ms** (queued) |
| shouldCloseLedger check cadence | Every 1s (timer) | Every 100ms (tick_interval) | **Every 1s** (maybe_tick_consensus gate) |
| proposersClosed visibility | Sees latest (proposals are immediate) | Sees latest (same loop) | **Stale up to 1s** (proposals queued, checked on 1s tick) |
| Close alignment mechanism | proposersClosed > prevProposers/2 | N/A (timer-based only) | Same check but **1s latency** makes it ineffective |

---

## Root Cause Analysis

### The 3+ second close time misalignment

1. Node-A's timer fires at T=0 → `phase_open` → `shouldCloseLedger` → closes (has transactions)
2. Node-A proposes its position → broadcasts to peers
3. Node-B receives the proposal at T=0.001s → **queued to pending_proposals**
4. Node-B's `drain_proposals` runs at T=0.05s → proposal enters `curr_peer_positions`
5. Node-B's `maybe_tick_consensus` last fired at T=-0.5s → next fires at T=0.5s
6. At T=0.5s, Node-B finally runs `shouldCloseLedger` → sees `proposersClosed=1` → not enough (need >2)
7. **Node-B waits another full second** until T=1.5s → maybe `proposersClosed=3` now → closes

Total delay between Node-A and Node-B closing: **1.5 seconds**
At 20 TPS relay per peer: **30 transactions** arrive in that window → 30 disputes

### Why the cascade happens
- First slow round sets `prev_round_time = 22s`
- Guard: `open_time < prev_round_time / 2 = 11s` → must stay open 11+ seconds
- But each node's prev_round_time differs slightly → they close at different times
- More disputes → slower next round → worse prev_round_time → permanent divergence

---

## Required Fixes (in order of impact)

### Fix 1: Run shouldCloseLedger every 50ms (not gated by 1s)
The `proposersClosed > prevProposers/2` fast-path WORKS, but only if we check it
frequently enough. If we check every 50ms instead of every 1s, nodes align within 100ms.

### Fix 2: Process proposals immediately (not queued)
Like rippled, call `consensus.peerProposal()` DIRECTLY from the network thread
(or at minimum, drain proposals into consensus state before shouldCloseLedger runs).
This ensures `proposersClosed` is always up-to-date when shouldCloseLedger checks it.

### Fix 3: Cap prev_round_time to prevent cascading
Add: `prev_round_time = min(prev_round_time, 10s)` to prevent cascading slowdowns.
If a single slow round produces prev_round_time=22s, cap it at 10s so the next round's
minimum open time is 5s max (not 11s).

---

## Validation

After fixes, expected behavior:
- All 5 nodes close within 100ms of each other (proposersClosed fast-path)
- 0-2 disputes maximum (from ~2 transactions in the 100ms window at 20 TPS)
- Rounds close in 3-4 seconds under 100 TPS multi-node load
- No cascading slowdown (prev_round_time capped)
