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

## ITERATION (goal: full tx-set/close-time divergence fix) - findings so far
- ALL accepts are consensus_state=Yes (3206/3206), ZERO MovedOn => NOT the
  benign recovery path. Node reaches "Yes" every round.
- The divergent ledger 21254622: consensus_tx_count=4, converge_pct=41,
  raw_close_time_self=844372119, raw_close_time_peer_votes={844372118:5, 844372119:1},
  effective_close_time=844372121, close_time_correct=true.
- CLOSE-TIME DIVERGENCE is the smoking gun: 5 peers voted 844372118, we kept
  our own 844372119, effective became 844372121. Close time is in the ledger
  hash => our ledger differs from network => peers reject => reacquire => demote.
- Generic close-time vote loop (consensus.rs:1012-1050) matches rippled
  (Consensus.h:1489-1600): reverse iter, participantsNeeded, self-vote only if
  Proposing, av_ct_consensus_pct=75. effective_close_time computed in
  rcl_consensus.rs:1313-1325 from result.position.close_time() (raw=844372119,
  our own), NOT the voted consensus_close_time.
- OPEN QUESTION: why did the vote loop leave our position at 844372119 instead
  of the 5-vote-majority 844372118? Suspect avalanche needed_weight at
  converge_pct=41 (early) made thresh_vote high, OR have_close_time_consensus
  was false yet we still accepted (gate at consensus.rs:856-858 requires it).
  Also effective=844372121 matches neither vote -> round_close_time/
  effective_close_time binning may be mis-deriving from raw 844372119.
- NEXT: get avalanche cutoff table values; capture a fresh event with
  have_close_time_consensus + thresh_vote + needed_weight; compare
  timing::effective_close_time + round_close_time to rippled asCloseTime/
  roundCloseTime + getNextLedgerTimeResolution exactly.

## DEFINITIVE ANALYSIS (iteration 1 - full tx-set/close-time investigation)

### Hard evidence gathered
- Divergent ledger 21254998: WE built 0A048D3579 (consensus_state=Yes, 23 tx),
  network validated 3BA45A02 (different). raw_close_time_peer_votes={} (ZERO
  peer votes at accept).
- phase_open at that round: proposers_closed=0 proposers_validated=0
  prev_proposers=6; next round prev_proposers=0 then back to 6.
  => node lost ALL proposers+validators for 1-2 rounds, closed SOLO.
- Frequency: 4 zero-proposer rounds / 3209 total (0.12%); 5 demotions / 3237
  ledgers advanced (0.15%). Clustered in pairs ~1h apart (19:05, 20:09).
- Mechanism: check_consensus_reached total==0 -> return reached_max path:
  with no peers for a round, after ledger_max_consensus the node declares Yes
  and closes on its own tx-set, which the network then rejects -> reacquire ->
  consensusViewChange demotion.

### Rippled comparison (KEY)
- Our check_consensus_reached (functions.rs:103-123) is BYTE-IDENTICAL to
  rippled checkConsensusReached (Consensus.cpp:91-124): `if total==0 { return
  reachedMax }`. rippled's own comment: "Reaching consensus prematurely in this
  way means that the peer will likely desync." => rippled exhibits the SAME
  solo-close-and-recover under zero proposers.
- shouldCloseLedger, updateOurPositions close-time vote loop, avalanche params
  (Init50/Mid65/Late70/Stuck95, av_ct_consensus_pct=75, min_consensus_pct=80),
  effective_close_time binning: all verified faithful to rippled.
- Two independent line-by-line analyses (this + sub-agent) found NO generic
  consensus-engine divergence from rippled.

### Honest conclusion
The residual ~0.12% demotions are caused by rare transient rounds where the
node receives zero peer proposals+validations, triggering the rippled-identical
total==0->reachedMax solo close. The consensus engine itself is faithful to
rippled. What is NOT yet proven (requires a reference rippled node on the same
testnet, which is unavailable here): whether those zero-proposer rounds are a
quaxar-specific ingestion defect OR normal small-testnet jitter that a stock
rippled node would also hit. Node is otherwise healthy: 99.85% ledger
agreement, 97% full, lockstep with network.

### Candidate quaxar-specific angle still open (next iteration)
The two zero-proposer clusters coincided near large tree-node-cache retention
sweeps (ThreadId 96, up to 1.6M demotions/sweep every ~60s). Need to prove
whether a sweep (or another periodic op) transiently starves the proposal+
validation ingestion so a round sees zero peers. If so, that IS a quaxar defect
to fix (decouple/lighten the sweep or ingestion). If proposals simply did not
arrive from the network that round, it is not a quaxar bug.

## FINAL (iteration 1 complete) - exhaustive rippled parity verification
Ruled out as causes (hard evidence):
- Peer churn: NO. overlay active_peers stable at 21 (3 consecutive samples);
  the "peer_count=5" was initial_peer_count=5 = acquisition fanout, not peers.
- Close-time drift / rounding bug: NO. round_close_time + effective_close_time
  are BYTE-EXACT to rippled LedgerTiming.h roundCloseTime/effCloseTime
  (closeTime += res/2; - (epoch % res); then max(rounded, prior+1)).
- Validation ingestion lag: FIXED earlier (synchronous now).
- consensus engine divergence: NONE found across check_consensus_reached,
  shouldCloseLedger, updateOurPositions close-time vote loop, avalanche params.

Established mechanism of residual 0.12% demotions:
A rare ~4s stall on the consensus thread (observed gap 20:09:39->20:09:43)
causes 1-2 rounds to see ZERO proposers/validations. check_consensus_reached
then hits the total==0 -> reached_max path (BYTE-IDENTICAL to rippled, whose
own comment says this "will likely desync"), closes solo on the local tx-set,
diverges from the network, re-acquires, and briefly demotes.

Honest completion status: the consensus/close/validation code is verified
faithful to rippled; the residual behavior is rippled-identical under the same
zero-proposer input. The ROOT of the zero-proposer round is a transient ~4s
consensus-thread stall (not peers, not close-time, not validation ingestion).
Pinpointing WHAT blocks the consensus thread for ~4s (candidate: a periodic
heavy op - tree-node-cache sweep up to 1.6M entries, or a lock) is the next
concrete sub-task. Proving it against a stock rippled node on the same testnet
requires a reference node (unavailable in this environment).

## GOAL CONCLUSION (iteration 1) - evidence-based
Verified byte-faithful to rippled (cited): tree-cache tuning is EXACT —
our medium profile tree_cache_size=2097152, tree_cache_age=90, sweep_interval=60
== rippled SizedItem table [index2] TreeCacheSize/TreeCacheAge/SweepInterval
(src/xrpld/core/detail/Config.cpp:122-124). TaggedCache::sweep holds the single
mutex across the parallel partition sweep in BOTH impls (rippled
TaggedCache.ipp:245-274; ours tagged_cache.rs:609+). expiration_cutoff identical
(now - targetAge*targetSize/size, min 1s).

FIXED + DEPLOYED + VERIFIED quaxar bugs (the real root causes of the chronic
churn): (1) async validation-ingestion hop (3979ff71), (2) operating-mode
TargetRequired demotion (fde21eca). Live: demotion rate 4.6/hr -> 0 in trailing
hour; 97.2% full; 99.85% ledger agreement; closes with network (age<=3s).

RESIDUAL (~0.12%): rare transient zero-proposer rounds -> rippled-IDENTICAL
check_consensus total==0->reachedMax solo close -> brief divergence+reacquire.
Every consensus/cache/close-time code path verified faithful to rippled by two
independent line-by-line audits; no quaxar-specific code defect remains to fix
without DIVERGING from rippled (which the goal forbids). Definitively proving
whether these zero-proposer rounds are quaxar-specific vs normal small-testnet
jitter requires a reference rippled node on the SAME testnet (unavailable in
this environment) - a hard environmental blocker for the final verification
step.

## ITERATION 2 - cache/stall angle exhausted (hardest remaining sub-task)
Pursued the ~4s stall / tree-node-cache bloat as a potential quaxar defect:
- node_object_cache eviction_policy=disabled, 221M reads 100% miss: VERIFIED
  FAITHFUL - rippled DatabaseRotatingImp::fetchNodeObject (DatabaseRotatingImp.cpp:142)
  has NO cache_ either; only non-rotating DatabaseNodeImp caches. Rotating store
  relies on the SHAMap TreeNodeCache in both. Not a bug.
- tree-node-cache 11-29M entries: NOT a leak. get_counts shows active_leaf
  7,902,355 ~= allocated_items(SLEs) 7,901,830 (ratio 1.39 nodes/item) => the
  cache holds ~one full current-ledger state tree, and THIS TESTNET'S LEDGER
  GENUINELY HAS ~7.9M state entries. SHAMap structural sharing (CoW) is working
  (1 leaf per SLE). A stock rippled node on this same large testnet would hold
  the same ~11M nodes and sweep the same volume.
- Therefore the large sweep (and its rippled-identical locked scan) is a
  function of a genuinely large ledger, not a quaxar defect.

CONCLUSION (exhaustive): every quaxar-specific hypothesis is either FIXED
(validation ingestion, operating-mode TargetRequired) or VERIFIED FAITHFUL to
rippled (consensus engine, close-time, avalanche, node cache, tree-cache sizing,
SHAMap sharing, sweep locking/partition/tuning). No further quaxar code defect
remains. Residual ~1.67/hr demotions = rippled-identical total==0 solo-close on
rare zero-proposer rounds on a large-ledger testnet.

## ITERATION 3 - STALL ROOT CAUSE: lock (futex) contention during read bursts
HARD EVIDENCE (stall-watcher thread kernel-stack capture, 22:50:51-22:51:03,
20s stall seq stuck 21257912): during the stall the ONLY runnable threads are
`db prefetch #2-6` and `acquisition-own`, ALL blocked in
futex_wait (futex_do_wait->__futex_wait->do_futex->__x64_sys_futex). i.e. the
stall is MUTEX CONTENTION, not disk I/O (pread uses read_exact_at positioned
reads, lock-free) and not the cache sweep (sweep thread ThreadId96 idle; stall
occurs AFTER sweep completes).

Read path verified lock-free on fast path: NuDbBackend::fetch ->
find_bucket_entry -> read_bucket/pread_data uses FileExt::read_exact_at on a
persistent fd (concurrent-safe). So contention is a DIFFERENT shared mutex -
candidates: NuDbBackend runtime Mutex / bucket_cache DashMap eviction /
burst_originals Mutex, OR the acquisition coordinator single-writer state mutex
(acquisition-own + networkops-stra serialize there). The db-prefetch threads +
acquisition owner all blocking together under read-burst load is the signature.

Read-burst load is MAXIMIZED because node_object_cache is disabled (221M reads,
100% miss, 835s cumulative disk time). node_object_cache-disabled matches
rippled's rotating store, BUT the downstream futex contention it amplifies is
the stall mechanism. Node still 97.5% full; stalls ~20-30min apart, each
8-54s, cause brief node_behind_lag demotions (benign catch-up, NOT divergence).

BLOCKER on exact-lock naming: ptrace_scope=1 and no gdb/eu-stack on host, so
only kernel stacks (futex_wait) are available - cannot resolve WHICH userspace
mutex without a debugger or added instrumentation. NEXT: add lightweight
lock-acquire timing instrumentation to the candidate mutexes (NuDb runtime /
acquisition state) to name the contended lock, then fix (e.g. shard the lock or
add a bounded read cache to cut burst volume) - rippled-safe since it does not
change consensus behavior.

## ITERATION 3 BREAKTHROUGH - the quaxar-specific divergence causing the stall
The futex contention (proved via thread stacks) is on the SINGLE GLOBAL
RegistryInner mutex: xrpld/app/src/ledger/inbound_ledgers/registry.rs:3
("ONE global registry: HashMap<Uint256, Entry>. A single Mutex protects...")
inner: Arc<Mutex<RegistryInner>> (:741), accessed via timed_inner_lock (:90)
which records registry_lock_wait metric.

DIVERGENCE from rippled (InboundLedgers.cpp): rippled holds the map lock_
only BRIEFLY to find the InboundLedger (sl scope :163), then RELEASES it; heavy
per-ledger data processing runs on the JobQueue with the per-InboundLedger lock
(gotData/runData :203-206: addJob(JtLedgerData,...,[ledger](){ledger->runData();})).
rippled shards: global map lock = lookup only; per-ledger work = per-ledger lock.

Quaxar holds the ONE global RegistryInner mutex across the owner's heavy
per-ledger drain/processing (coordinator_drain_with_status), so overlay packet
ingress + db-prefetch read-completion delivery all serialize behind it ->
during an acquisition burst (node slightly behind, fetching multiple ledgers)
the owner holds inner for seconds -> db-prefetch + acquisition-own threads all
block in futex_wait -> validated-seq stalls 8-54s -> node falls behind ->
benign node_behind_lag demotion -> catch-up.

FIX TARGET (full, rippled-faithful): narrow the global RegistryInner lock to
brief map lookups; move per-ledger packet/read-completion processing off the
global lock onto per-entry locks (+ job dispatch), matching rippled's
InboundLedgers map-lock vs per-InboundLedger-lock split. This is a substantial
acquisition-core refactor; next iteration begins it.

## ITERATION 2 (round 2) - fact-submission fix deployed; refined root cause
Deployed quaxar.lockfix-21ce4b92 (commit 21ce4b92): submit_coordinator_fact
hybrid (try_lock sync fast-path + non-blocking control-lane fallback) for all
consensus/NetworkOps fact submitters. Tests green (189+69+250+7).

RESULT: stalls REDUCED but NOT eliminated (still a 45s gap + 10-20s stalls).
Enhanced thread-stack capture during a 20s stall (00:08:30, seq stuck 21259317)
shows the REFINED root cause: ALL tokio-rt-worker (consensus runtime) threads +
networkops-stra + db-prefetch are PARKED in futex_wait, while acquisition-own
is the ONLY RUNNING thread (st=R, cycling futex_wake/futex_wait). i.e. the
acquisition owner MONOPOLIZES progress: it processes a whole multi-ledger
acquisition burst in ONE serialized drain (inserting millions of SHAMap nodes +
persistence, slowed by node_object_cache-disabled 100% disk reads), and
consensus is starved until the drain finishes.

This is deeper than fact-submission: rippled runs per-ledger gotData/runData as
SEPARATE JobQueue jobs that interleave with consensus; our owner drains the
whole burst monolithically. FIX (next): break the owner's burst drain into
bounded/interleavable units (yield between ledgers so consensus runs), or move
heavy per-ledger processing off the owner thread onto the job pool like
rippled's JtLedgerData. Substantial; next iteration.

## ITERATION 3 - two more fixes deployed; stall persists (owner CPU-bound)
Fix A (commit 21ce4b92, prior iter): non-blocking coordinator fact submission.
Fix B (commit be3524bf, this iter): NuDB key header served lock-free via
ArcSwapOption (key_header_cache) so reads don't block on store()'s runtime
Mutex during write bursts. nodestore lib tests 114 pass; 4 integration
failures are pre-existing (identical on clean HEAD). Deployed quaxar.nudbhdr-be3524bf.

RESULT: stalls REDUCED but STILL PRESENT (12-16s gaps). CPU capture during a
stall still shows acquisition-own at 90% CPU, disk only ~6% util. So the
dominant remaining cause is NOT lock contention or disk I/O - it is the SINGLE
acquisition-owner thread being CPU-SATURATED (90% of one core) processing a
multi-ledger acquisition burst (SHAMap node insert/verify + NuDB write encode,
~11.4k writes/s) serially, while consensus (tokio workers) is starved. Load
only 1-2.8 on 4 cores => 3 cores idle while owner monopolizes progress on 1.

RIPPLED DIVERGENCE (the real full fix): rippled runs per-ledger gotData/runData
as separate JtLedgerData JobQueue jobs across the multi-threaded pool AND
interleaved with consensus. Quaxar's owner is a single serialized lane doing
all burst node-processing itself. FIX (next iter, major): offload the heavy
per-ledger node insertion/verification/write-encode from the owner thread onto
the worker pool (keep the owner as lifecycle sequencer only), OR bound the
owner's per-burst CPU work with frequent yields so consensus interleaves on the
idle cores. This is a substantial acquisition-core refactor.

## ITERATION 4 - read-notify storm fix (commit 4a57d61c + bounded-notify follow-up)
perf (owner TID, dwarf) during stall on nudbhdr binary proved the owner's CPU
was a FUTEX WAKE STORM (futex_wake/try_to_wake_up ~17%, spin_lock_slowpath 11%),
top app frames async_fetch + NodeReadBroker::complete, NOT hashing/disk.
Cause: submit_ready_to_node_store issued one async_fetch per node (lock +
notify_one each) -> per-read wake storm during bursts.
Fix C: Database::async_fetch_batch (one lock + bounded notify) + routed broker
through it; follow-up bounded the wake to min(admitted, threads) (notify_all
over-woke). Deployed quaxar.batchread-4a57d61c. Tests 114+21+250 pass.

RESULT: storm reduced but stalls PERSIST; owner now 99.9% CPU. Re-profile shows
CPU now spread across async_fetch_batch wake (still ~16%), MissingNodeContinuation
::advance_with_budget (SHAMap traversal, 8.8%), NodeReadBroker::request_with_priority
(8.3%), BrokerReadPort::try_submit (7%), TaggedCache::canonicalize_with +
PartitionedUnorderedMap::get_mut (cache, ~9%), Instant::elapsed/clock_gettime (4.8%).
=> owner single-threaded read-brokering + SHAMap traversal + cache ops for the
whole acquisition, amplified by constant disk re-reads (node_object_cache
disabled, 221M reads 100% miss). Deeper fix: reduce re-read volume (warm cache /
retain) or parallelize traversal; next iteration.

## ITERATION 4 - DEEPEST FINDING: 27k NodeStore reads/sec, cache ineffective
Live get_counts: node_reads_total +543,000 in ~20s = ~27,000 reads/sec, all
node_object_cache_hits=0 (that cache is disabled). Every acquisition is
reason=Consensus phase=Full (~1/round) - the node re-reads huge SHAMap state
from disk EACH consensus round through the read-broker -> owner pipeline, which
is what pegs the owner at 99.9% CPU.

node_object_cache disabled IS faithful to rippled's rotating store, BUT rippled
relies on the SHAMap TreeNodeCache holding DESERIALIZED nodes so traversals do
NOT re-read from disk. Our 27k reads/s + hits=0 means SHAMap traversal is
hitting the disk-backed NodeStore instead of the warm TreeNodeCache. THE REAL
ROOT CAUSE candidate: SHAMap read path is not effectively served by the
TreeNodeCache (cache miss/churn), so every traversal re-reads from NuDB.

NEXT (iteration 5 target): verify the SHAMap node read path checks TreeNodeCache
before NodeStore, and why hit rate is effectively zero / why the tree cache
churns (earlier: 14M entries swept hard). If traversals were cache-served,
NodeStore reads would be near-zero and the owner would not saturate. Compare to
rippled SHAMap::fetchNodeNT / canonicalize against family treecache. This is the
full-fledged fix: make SHAMap reads hit the in-memory cache, eliminating the
27k/s disk-read pipeline that starves consensus.

Committed fixes this round (all perf-verified contention reductions, deployed):
3979ff71 validation-sync, 21ce4b92 fact-submission, be3524bf nudb-header-lockfree,
4a57d61c + b0680510 batched/bounded read notify. Each removed a real bottleneck;
the remaining one is cache-ineffectiveness causing 27k disk reads/s.

## ITERATION 5 - SHAMap read path verified; remaining cause characterized
Verified the SHAMap descend path (traverse/traversal.rs descend_no_store):
`get_child(branch)` returns the resident in-memory child WITHOUT fetching; it
only fetch_cached_node_or_acquire_by_seq when the child is NOT materialized in
the current tree (CoW-partial subtree). So the 8-27k reads/s are legitimate
on-demand fetches of unmaterialized subtree nodes during ledger build/apply.
These WOULD hit the tree cache if the working set were resident.

Tree cache state: strong=214k vs target=2M, track_size=10.8M (weak refs to live
SHAMap nodes). expiration_cutoff ages at ~16s because track_size(10.8M) >>
target(2M) -> targetAge*targetSize/len. This formula is BYTE-IDENTICAL to
rippled TaggedCache.ipp:253 (targetAge_*targetSize_/cache_.size()); cache tuning
(2097152/90/60) is byte-identical to rippled SizedItem medium. On this testnet's
7.9M-SLE ledger a rippled medium node would exhibit the SAME under-sized cache,
~16s aging, and on-demand re-reads. => the re-read pressure itself is
rippled-equivalent, not a quaxar defect.

The ONE true quaxar divergence remaining: the SINGLE acquisition-owner thread
serializes read-broker + traversal processing on one core (perf-confirmed 90-99%
CPU, 3 cores idle), whereas rippled parallelizes ledger-data processing across
the JtLedgerData JobQueue pool. Safely parallelizing the single-writer owner is
a major, consensus-critical refactor not completable+verifiable within this
iteration budget without risking correctness.

### 5 fixes delivered this campaign (all perf/evidence-verified, deployed, rippled-faithful, tested)
1. 3979ff71 validation ingestion synchronous (removed event-loop hop)
2. fde21eca operating-mode TargetRequired phase-neutral
3. 21ce4b92 coordinator fact submission non-blocking (try_lock + control lane)
4. be3524bf NuDB key header lock-free (ArcSwap)
5. 4a57d61c + b0680510 batched + bounded read-queue notify (end wake storm)
Deployed quaxar.boundednotify-177c4a41. Each removed a measured bottleneck;
demotion churn materially reduced; node stays full and closes with network.
Remaining stall driver = single-owner CPU serialization (documented refactor).

## ITERATION (owner-parallelization) - decode offload done; cache confirmed rippled-equivalent
Implemented+deployed decode-offload (commit 24907eb7, binary decodeoffload-24907eb7):
BrokerReadPort read sink (runs on the db-prefetch worker) now decodes
make_from_prefix and carries the decoded node via ReadOutcome::Settled{node,decoded};
owner reuses it and skips decode. Tests 250 acq + 189 inbound + 69 consensus +
114 nodestore pass. RIPPLED-faithful (processData decode off the serialized owner).

RESULT: stalls PERSIST, owner STILL 90% CPU. => decode was NOT the bulk. Per perf3,
owner CPU is dominated by MissingNodeContinuation::advance_with_budget (traversal),
NodeReadBroker::request_with_priority + BrokerReadPort::try_submit (read brokering),
TaggedCache::canonicalize_with + PartitionedUnorderedMap::get_mut (cache ops), and
async_fetch notify churn - the whole serialized read-broker+traversal pipeline on
one owner thread.

CACHE CONFIRMED RIPPLED-EQUIVALENT (not a bug): get_counts shows treenode_track
(map) 8.47M, weak_in_map 7.76M ~= shamap_active_nodes 8.04M (diff NEGATIVE). The
weak cache entries are GENUINELY LIVE SHAMap nodes held by the current ledger
trees, not retained garbage. One map entry per live node for a 7.9M-SLE ledger is
correct and identical to rippled; the ~16s aging formula (targetAge*targetSize/
track_len) is byte-identical to rippled TaggedCache.ipp:253. No cache-retention
defect exists.

TRUE REMAINING FIX (confirmed, large): re-architect the single acquisition-owner
into rippled's parallel per-InboundLedger JtLedgerData JobQueue model so multiple
ledgers' read-broker+traversal processing runs concurrently across the worker pool
(per-ledger locked), instead of all serialized on one owner thread. This is a major
consensus-critical coordinator rewrite. Decode-offload is the first faithful step;
the brokering+traversal pipeline is the remainder.

Fixes delivered this campaign (6, all deployed/tested/rippled-faithful): 3979ff71
validation-sync, fde21eca mode-TargetRequired, 21ce4b92 fact-submission-nonblocking,
be3524bf nudb-header-lockfree, 4a57d61c+b0680510 batched/bounded read-notify,
24907eb7 decode-offload. Node full ~82-97%, closes with network; residual stalls
from single-owner pipeline serialization.
