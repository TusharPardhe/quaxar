# Consensus Single-Strand Execution Plan

## Problem Statement
Quaxar's consensus rounds take 16-22s under multi-node 100 TPS load instead of rippled's 3-4s.
Root cause: transaction relay processing (network thread) races with on_close capture (loop thread),
causing 3+ disputes that trigger slow avalanche voting resolution.

## How Rippled Solves This
Rippled uses a **single-strand (io_service::strand)** model where:
- `PeerImp::onMessage(TMTransaction)` → `NetworkOPs::processTransaction()` → `openLedger.modify()`
- `timerEntry()` → `onClose()` → `openLedger.current()`

These **cannot interleave** because they're posted to the same strand. When `timerEntry` runs,
ALL pending `processTransaction` calls have already completed. Result: 0 disputes, 3-4s rounds.

## How rxrpl Solves This
rxrpl processes ALL messages on a **single async event loop** (tokio select):
- Incoming TMTransaction → applied to open ledger immediately on the event loop
- Timer tick → close_ledger called on the same event loop
- No parallelism between relay and consensus → 0 disputes

## Our Fix: Sequential Message Processing on Loop Thread

### Architecture Change
Replace the current multi-threaded model:
```
BEFORE:
  Network Thread: receives TMTransaction → router callback → process_transaction (adds to pending)
  Loop Thread:    drain pending → apply batch → tick → on_close captures

AFTER:
  Network Thread: receives TMTransaction → pushes raw bytes to queue (no processing)
  Loop Thread:    drain raw queue → process_transaction → apply → tick → on_close captures
                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                  ALL of this is ONE sequential block — no interleaving possible
```

### Key Invariant
**All transaction processing AND consensus ticking happen on the SAME thread (loop thread).**
The network thread's ONLY job is to buffer raw messages. It never calls process_transaction.

---

## Implementation Steps

### Stage 1: Remove router, use raw queue only
- [ ] 1.1 Remove `set_transaction_router` from bootstrap.rs consensus setup
- [ ] 1.2 Let TMTransaction messages accumulate in overlay's `take_transactions()` queue
- [ ] 1.3 In the loop, drain `take_transactions()` → process each → apply ONE batch → tick
- [ ] 1.4 The drain+apply+tick must be ONE sequential block (no sleep between them)
- [ ] 1.5 Keep main.rs router for pre-consensus catchup phase (cleared on consensus_started)

### Stage 2: Move apply_pending INTO the tick call
- [ ] 2.1 Remove the drain block that runs before `maybe_tick_consensus!()`
- [ ] 2.2 Instead, pass the overlay reference to the consensus timer_tick
- [ ] 2.3 Inside timer_tick (which calls phase_open → close_ledger → on_close), drain+apply FIRST
- [ ] 2.4 This guarantees: drain → apply → on_close all happen atomically in one function call

### Stage 3: Make drain+apply+close truly atomic
- [ ] 3.1 In on_close adaptor: drain overlay queue + process + apply + capture (all inline)
- [ ] 3.2 Remove separate drain block from the loop (it's now inside on_close)
- [ ] 3.3 The loop's only job: call timer_tick every iteration (no 1s cadence needed for drain)

### Stage 4: Verify and test
- [ ] 4.1 cargo check + build
- [ ] 4.2 Docker rebuild
- [ ] 4.3 Multi-node stress test (100 TPS, 5 nodes, round-robin submission)
- [ ] 4.4 Target: 0 disputes, all rounds 3-4s, ≥20 validated ledgers in 60s
- [ ] 4.5 Compare with rippled results (40 rounds in 60s at same load)

### Stage 5: Parity review
- [ ] 5.1 Verify vs rippled: same shouldCloseLedger timing
- [ ] 5.2 Verify vs rippled: same checkConsensus thresholds
- [ ] 5.3 Verify vs rxrpl: same single-loop processing model
- [ ] 5.4 Clean up dead code (close_gate, unused router setup, diagnostic logs)
- [ ] 5.5 Commit and push final state

---

## Critical Design Decisions

1. **No router callback at all during consensus** — raw bytes queue only
2. **process_transaction happens on loop thread** — same as rippled's strand
3. **apply_pending happens ONCE right before on_close's capture** — same as rippled's applyHeldTransactions
4. **Timer tick cadence stays at 1s** (matching rippled's LEDGER_GRANULARITY) — but drain happens every loop iteration (50ms) so transactions are always fresh
5. **on_close does NOT apply** — it just captures. The apply already happened in the drain immediately before.

## File Changes Required
- `xrpld/app/src/bootstrap/bootstrap.rs` — main loop restructure
- `xrpld/app/src/consensus/rcl_consensus.rs` — simplify on_close (just capture)
- `xrpld/overlay/src/transport/inbound.rs` — already has clear_transaction_router
- `xrpld/app/src/state/application_root.rs` — can remove close_gate (no longer needed)
