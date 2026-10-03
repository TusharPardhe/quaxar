# Quaxar demotion-churn root cause investigation (2026-10-03)

## Symptom
Node (non-validator, pubkey_validator=none, quorum=5, VL count=1, 6 trusted
validators) demotes out of `full` ~4.3-4.6/hr. On phasefix-fde21eca binary:
36 full-transitions / 8.34h. Pattern: full->syncing->tracking->full, 2-7s each.

## NOT the cause (ruled out with evidence)
- Chain oscillation: NONE. validated LCL only advances forward, no backward/
  competing-hash. validated_seq steps +1 (1051 single steps vs 10 +2 jumps/hr).
- Operating-mode machine: the (Full,TargetRequired)=>Syncing fix (fde21eca,
  deployed+confirmed) was correct but minor. Dominant trigger is elsewhere.
- Acquisition "multiple nodes": live_sessions mostly 0; occasional 5-7 during
  the brief catch-up, not the driver.

## ROOT CAUSE (evidence-backed)
The demotions are rippled-faithful `consensusViewChange()` firings
(rcl_consensus.rs ~908: `mode != WrongLedger && preferred != prev_ledger_id`,
matches RCLConsensus.cpp:311-314 EXACTLY). So the mode demotion is correct GIVEN
the inputs. The real defect is UPSTREAM: `getPreferred` transiently returns the
PARENT (seq N-1) right after we close ledger N.

Evidence (lcl_audit preferred-LCL sampled, repeated):
- validation_trie_preferred = seq N-1  (what getPreferred/getPrevLedger sees)
- validation_working_preferred = seq N  (correct, our just-closed ledger)
- selected_trusted_validation_count distribution over 30m:
  6:537  4:137  2:99  3:77  5:62  1:55  0:11
  => ~44% of samples have < quorum(5) trusted validations counted for current.
- When trie_preferred(N-1) resolves and our LCL(N) descends from it, the
  working logic (validations.rs:1085-1109) returns N correctly. But when the
  N-1 trie node's winning child is not yet N (N's trusted validations haven't
  arrived+inserted), getPreferred returns the N-1 branch =>
  preferred(N-1) != prev_ledger(N) => consensusViewChange => demotion.

So: a VALIDATION INGESTION/TRIE-UPDATE TIMING race. We close N locally fast,
then ask getPreferred before enough trusted validations for N are in the trie.

## Key code
- getPreferred decision: xrpld/consensus/src/rcl_support/validations.rs:1064
  get_preferred_lcl_diagnostic; trie.get_preferred(largest) where
  largest = inner.local_seq_enforcer.largest().
- LedgerTrie: xrpld/consensus/src/model/ledger_trie.rs (ported line-for-line).
- view-change trigger: xrpld/app/src/consensus/rcl_consensus.rs:908.
- fact->mode: xrpld/acquisition/src/phase.rs (TargetRequired now phase-neutral).

## NEXT (hypothesis to confirm + fix)
1. Check how/WHEN the node inserts its OWN closed ledger N into the validation
   trie and advances local_seq_enforcer. rippled seeds the local ledger so
   getPreferred prefers N over N-1 even before external validations arrive.
   If quaxar inserts the local validation LATER than the getPrevLedger query
   (or not at all for a non-validator), getPreferred lags one ledger.
2. Compare rippled: for a non-validator (not issuing validations), does the
   trie still advance via TRUSTED PEER validations in time? rippled's
   checkAccept/handleNewValidation inserts trusted vals into the trie; the
   query in getPrevLedger happens on the next round. Determine the ordering
   difference.
3. Likely fix: ensure trusted validations for N are inserted into the trie
   before/at the getPreferred query (ordering), OR that getPrevLedger uses the
   same working-preferred resolution (which already returns N) rather than a
   raw trie read that returns N-1. The `working_preferred` already computes N
   correctly in the common case; confirm getPrevLedger consumes `selected`
   (which = working) not a separate stale trie read.

## FIX IMPLEMENTED (iteration 1, in progress)
Root cause confirmed by sub-agent: trusted validations took an extra async hop
(JtValidation job -> ConsensusEvent::Validation -> sync_channel(1024) -> single
shared event loop -> add) vs rippled's synchronous checkValidation->recvValidation
->add. The shared loop also handled LedgerDone => head-of-line blocking delayed
trusted validations past the next getPrevLedger read => getPreferred returned
N-1 => rippled-faithful consensusViewChange demoted the node ~4/hr.

Done:
- Added driver::process_validation_inline(app, queued) with the exact
  verify+ingress+relay logic (xrpld/app/src/consensus/driver.rs).
- bootstrap.rs JtValidationT/Ut job now calls process_validation_inline DIRECTLY
  (synchronous, matches rippled) instead of enqueuing ConsensusEvent::Validation.
- Removed the dead validation-forwarder thread + val_notify channel +
  set_validation_notify + direct_event_tx (overlay on_validation early-returns
  when a router is installed, so queue/forwarder path was dead in production).
- Event-loop arm now calls the shared fn (no dup).
- Builds green (-p app). Only warning: event_tx unused (next step).

DISCOVERY: ConsensusEvent::LedgerDone is NEVER emitted anywhere in the workspace
=> the entire consensus event loop (spawn_event_loop/consensus_event_channel/
ConsensusEvent) is dead infra. Acquisition/durable-handoff path does its own
ledger persistence (inbound_completion_persisted etc.).

## REMAINING (this iteration if possible)
1. Remove dead event loop: spawn_event_loop call + channel in bootstrap.rs;
   simplify driver.rs to process_validation_inline + parse_validation only
   (drop ConsensusEvent enum, spawn_event_loop, LedgerDone arm).
2. Build full workspace + run consensus/validation/driver tests.
3. Add regression test: saturate/deny old path, inject 5 trusted N-1 vals,
   assert get_prev_ledger returns our N child (close-with-network), node-type
   agnostic.
4. Rebuild release on server, redeploy (new tag), monitor full-transition rate
   over >=1h. SUCCESS = full-transition rate drops materially below 4.3/hr and
   demotions no longer correlate with validation-ingest lag.

## VERIFIED RESULT (deployed valsync-3979ff71)
Rebuilt+redeployed to testnet (3.9.172.38), symlink quaxar.valsync-3979ff71,
rollback target quaxar.phasefix-fde21eca.

Over 41 min uptime after reaching full:
- full transitions = 1 (clean startup only) vs 4.3-4.6/hr before.
- full->syncing demotions in 35-min steady window = 0 (was ~2/25min).
- Once full (~324s), stayed full 2136s continuously, zero demotions.
- Lockstep with network: seq 21253361(age4) -> 21253364(age2) in 9s, state=full.
- selected_trusted_validation_count=0 occurrences ELIMINATED (was 11; now min 1),
  confirming the ingestion delay closed.

CONCLUSION: root cause = asynchronous validation ingestion hop delaying trusted
validations past the consensus getPrevLedger read. Fixed by processing
validations synchronously on the job thread (rippled parity). Demotion churn
effectively eliminated; node closes with network regardless of node type.

Commits on branch sync/rippled-aug-sept-2026:
- fde21eca: phase.rs TargetRequired phase-neutral for Full/Tracking (partial).
- 3979ff71: synchronous validation ingress (ROOT CAUSE fix) + dead event-loop removal.

## REMAINING ISSUE (what the residual ~5 demotions are) - 2026-10-03 22:xx
After the valsync fix, the few remaining full->syncing demotions (5 in 3h,
0 in the last 2h) are a DIFFERENT, deeper cause than validation ingestion:

Mechanism (traced at 19:48:38-19:49:12):
- Node closes ledger N locally in consensus_mode=Observing on its own tx-set
  (e.g. 21254622, tx_set_id=B35D6F17, open_tx_count=6).
- Peers do NOT agree: selected_peer_lcl_support collapses 16 -> 3 -> 1 for the
  node's local LCL within the same second (184 preferred_lcl_selected evals in
  19:48:41 alone).
- Node launches acquisition for the network's actual ledger/tx-set (F5B58FC4),
  stalls ~33s at 21254621 while peers advance to ~21254627, then catches up a
  BURST out of order (inbound_completion_persisted seq 22,24,25,27 then 23;
  10 out-of-order completions in 3h).
- During the stall the preferred LCL legitimately diverges -> rippled-faithful
  consensusViewChange / PreferredLclDivergence demotion.

Root cause class: the node (non-proposing, Observing) intermittently builds a
ledger on a local tx-set that diverges from the network consensus tx-set, then
must discard+reacquire. This is the "is our transaction hash right" angle:
tx-set selection for an Observing node sometimes != network's agreed set.
validation ingestion is NOT the cause here (selected_trusted_validation_count=6
throughout).

Impact now: rare and self-correcting (0 in last 2h, node stays 96.9% full,
closes with network in lockstep seq+1/3s). Lower severity than the fixed bug.

NEXT (future iteration): investigate why an Observing round closes on a local
tx-set diverging from the network. Candidates: (a) we start on_close/on_accept
before peer proposals/tx-set are incorporated (timing), (b) tx-set acquisition
(acquire_tx_set) returning our open set instead of the agreed one, (c)
close-time / establish-phase threshold causing premature local close. Compare
RCLConsensus onClose/onAccept + Consensus::closeLedger establish timing.
