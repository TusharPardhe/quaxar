# Quaxar Consensus Debugging Prompt

## Context

You are debugging quaxar, a Rust implementation of the XRP Ledger protocol, located at `/Users/tusharpardhe/Documents/xrpl/quaxar`. The rippled C++ reference implementation is at `/Users/tusharpardhe/Documents/xrpl/rippled`. Another Rust implementation (rxrpl) is at `/Users/tusharpardhe/Documents/xrpl/rxrpl`.

Branch: `fix/consensus-fork-dispute-resolution` (latest commit: `8d738f6`)

## What Was Done

Over multiple sessions, we fixed the consensus pipeline to achieve 3-second rounds with 5-node agreement under load. The key architectural changes (all committed):

1. **Two-worker thread model** (matching rippled's JtBatch + JtNetopTimer):
   - Thread 1 (`tx-batch-apply`): Dedicated thread that wakes instantly via condvar when relay transactions arrive. ONLY job: drain overlay queue → process_transaction → apply_network_ops_pending_to_open_ledger. Loops until pending is empty (matching rippled's `while(!transactions_.empty()) { apply(); }` in NetworkOPs.cpp:1515-1525).
   - Thread 2 (main consensus loop): 50ms poll for proposals, 1-second gated `handle_consensus_timer` (matching rippled's `ledgerGRANULARITY=1s` in ConsensusParms.h:79).

2. **Instant relay wake** — overlay's `on_transaction` (when no router is set) calls a notify callback that wakes the batch thread immediately (matching rippled's `doTransactionAsync` scheduling JtBatch via `workers_.addTask()` → semaphore in Workers.cpp:112).

3. **Single-batch apply for relay** — relay router only calls `process_transaction` (O(1) queue push), no `apply_pending`. The batch thread and RPC submit's inline `run_sync_batch` are the only apply points (matching rippled's `transactionBatch` → `apply()` in NetworkOPs.cpp:1528-1566).

4. **prev_round_time cap at 10s** — prevents cascading slowdown after one disputed round (consensus.rs around line 729).

5. **Fat-leaves tx-set serving** — serves ALL SHAMap nodes on root request for 1-round-trip acquisition (bootstrap.rs serve_one_get_ledger_request).

## Current Results

At 30 TPS (realistic XRPL load):
- 5/5 nodes validate to seq 21
- 3-second rounds consistently  
- Only 2 disputes
- 690 transactions applied

## The Remaining Bug

**After the stress test ends, validation stops advancing.** Nodes diverge at the last loaded round (where 2-3 disputes occurred) and never recover. They continue closing empty rounds independently at 3s cadence but val_seq is permanently stuck.

### Root Cause

When disputes occur (even just 2-3), some nodes end up with slightly different final tx-sets after dispute resolution. This produces different ledger hashes. After that:

1. Each node starts the next round with a different `prev_ledger_id`
2. Peer proposals reference a different `prev_ledger_id` → rejected by `peer_proposal` (consensus.rs line ~450: `if *new_peer_prop.prev_ledger() != self.prev_ledger_id { return false; }`)
3. `proposersClosed = 0` → no agreement → close independently → permanently diverged

### Why Disputes Don't Fully Resolve

The dispute resolution mechanism (vote flipping via `update_vote` in `disputed_tx.rs`) works but doesn't always converge to the SAME final set across all nodes because:

- Different nodes have slightly different vote counts (due to relay timing of peer proposals)
- The `converge_percent` thresholds may cause nodes to flip votes at different times
- With only 1-second timer ticks for `update_our_positions`, there may not be enough voting rounds for full convergence before `haveConsensus` declares done

### What Rippled Does

In rippled, this scenario almost never occurs because relay is fast enough to prevent disputes entirely. But IF it did occur, rippled has a **wrong-ledger recovery mechanism** (`Consensus.h` lines 580-591, `checkLedger` → `handleWrongLedger`):

```cpp
void checkLedger() {
    auto net_lgr = adaptor_.getPrevLedger(prev_ledger_id, previous_ledger, mode);
    if (net_lgr != prev_ledger_id || previous_ledger.id() != prev_ledger_id) {
        handleWrongLedger(adaptor, net_lgr);
    }
}
```

This calls `getPrevLedger` which queries the validation trust trie to find the ledger the MAJORITY of validators have validated. If it's different from ours, we switch to it.

Our `get_prev_ledger` implementation exists (rcl_consensus.rs line ~482) but may not be triggering recovery correctly because after divergence, NO node validates further (val_seq stuck), so the trust trie has no new information to indicate which branch is correct.

## What Needs to Be Fixed

**Option A: Prevent disputes entirely**
- Ensure all nodes produce the SAME tx-set at close time (0 disputes)
- This requires relay to be fast enough that fee escalation decisions are identical
- The fee escalation ordering issue (TxQ.cpp:182, `scaleFeeLevel` reads `view.txCount()`) means nodes that received different transaction ORDERS may accept/reject different sets

**Option B: Make dispute resolution always converge**
- Ensure `update_our_positions` runs enough voting rounds for ALL nodes to reach the same final set
- Verify our `update_vote` thresholds match rippled's exactly
- Ensure proposals are exchanged fast enough during establish phase

**Option C: Fix wrong-ledger recovery**
- After divergence, nodes should detect they're on the minority fork
- `get_prev_ledger` should identify the majority's validated ledger
- `handle_wrong_ledger` should force acquisition of the correct ledger and restart consensus from there

## Key Files

| File | Purpose |
|------|---------|
| `xrpld/app/src/bootstrap/bootstrap.rs` | Main loop, batch thread, router setup, timer |
| `xrpld/app/src/consensus/rcl_consensus.rs` | Adaptor (on_close, get_prev_ledger) |
| `xrpld/consensus/src/algorithm/consensus.rs` | State machine (phase_open, phase_establish, have_consensus, check_ledger, handle_wrong_ledger, update_our_positions) |
| `xrpld/consensus/src/algorithm/functions.rs` | should_close_ledger, check_consensus |
| `xrpld/consensus/src/model/disputed_tx.rs` | update_vote, stalled |
| `xrpld/app/src/state/application_root.rs` | apply_network_ops_pending_to_open_ledger, relay closures |
| `xrpld/overlay/src/transport/inbound.rs` | on_transaction, transaction_notify |

## Rippled Reference Files

| File | What to look at |
|------|-----------------|
| `src/xrpld/app/misc/NetworkOPs.cpp` | doTransactionAsync (1380), transactionBatch (1515), apply (1528), processHeartbeatTimer (1100) |
| `src/xrpld/consensus/Consensus.h` | timerEntry (1131), phaseOpen (1160), updateOurPositions (1445), haveConsensus, checkLedger (580) |
| `src/xrpld/consensus/Consensus.cpp` | shouldCloseLedger (18) |
| `src/xrpld/consensus/ConsensusParms.h` | All timing parameters |
| `src/xrpld/app/consensus/RCLConsensus.cpp` | onClose (330), getPrevLedger |
| `src/xrpld/app/misc/detail/TxQ.cpp` | scaleFeeLevel (182), tryDirectApply (1654) |

## How to Test

```bash
cd /Users/tusharpardhe/Documents/xrpl/quaxar
cargo build -p xrpld-main --release
# Bust docker cache:
echo "# rebuild $(date +%s)" >> Cargo.toml
docker build -t quaxar:latest .
sed -i '' '/^# rebuild/d' Cargo.toml

cd /Users/tusharpardhe/Documents/xrpl/quaxar-docker-test
docker compose down -v && rm -rf ./db && bash boot.sh
sleep 20

# Verify nodes validating:
for port in 5005 5006 5007 5008 5009; do
  curl -s -X POST http://127.0.0.1:$port -d '{"method":"server_state","params":[{}]}' | python3 -c "
import json,sys;d=json.load(sys.stdin)['result']['state'];vl=d.get('validated_ledger',{})
print(f'port $port: val_seq={vl.get(\"seq\")}')"
done

# Run stress test:
TARGET_TPS=30 NUM_ACCOUNTS=20 DURATION_SEC=60 node stress.js

# Check results:
# - val_seq should keep advancing (not stuck)
# - All 5 nodes should have same val_seq
# - Disputes should be 0
# - docker logs quaxar-0 2>&1 | grep "Ledger closed" should show 3s rounds
```

## Success Criteria

1. Under 30 TPS multi-node stress test, ALL 5 nodes validate the same ledgers continuously (val_seq keeps advancing, never gets stuck)
2. 0 disputes
3. 3-second round cadence maintained throughout
4. If disputes DO occur, nodes recover and resume validating together (wrong-ledger recovery works)
