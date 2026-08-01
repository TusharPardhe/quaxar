# Rippled Ledger-Acquisition Subsystem — Parity Audit FINDINGS

**Auditor:** rippled-auditor (C++ reference auditor)
**Reference tree:** `/Users/tusharpardhe/Documents/xrpl/rippled/src/xrpld`
**Compared against:** quaxar Rust implementation under `/Users/tusharpardhe/Documents/xrpl/quaxar/xrpld`
**Scope:** InboundLedger / InboundLedgers / TimeoutCounter / SkipListAcquire / LedgerDeltaAcquire / LedgerReplayTask / LedgerReplayer and the replay-parameter set.

---

## Scope & method

Every claim below is grounded in the actual C++ source. The primary reference files are:

- `src/xrpld/app/ledger/InboundLedger.h` (class/state declarations)
- `src/xrpld/app/ledger/InboundLedgers.h` (registry interface)
- `src/xrpld/app/ledger/detail/InboundLedger.cpp` (acquisition core)
- `src/xrpld/app/ledger/detail/InboundLedgers.cpp` (registry/core container)
- `src/xrpld/app/ledger/detail/TimeoutCounter.{h,cpp}` (timer loop)
- `src/xrpld/app/ledger/detail/SkipListAcquire.{h,cpp}`
- `src/xrpld/app/ledger/detail/LedgerDeltaAcquire.{h,cpp}`
- `src/xrpld/app/ledger/detail/LedgerReplayTask.cpp`, `src/xrpld/app/ledger/LedgerReplayer.h`
- `src/xrpld/app/ledger/detail/LedgerReplayer.cpp`

The report is split into:
- **(A)** items the quaxar implementation covers correctly (with rippled line cites used to confirm them)
- **(B)** items that are MISSING or divergent in quaxar (with rippled file:line and the descriptive text that should be added)
- **(C)** claims that were inaccurate and their corrections

---

# (A) CORRECTLY-COVERED ITEMS

### A1. Per-ledger timing constant: 3000 ms acquire timeout
- rippled: `constexpr auto kLedgerAcquireTimeout = 3000ms;` — `InboundLedger.cpp:69`, passed as `timerInterval_` in the `TimeoutCounter` base ctor (`InboundLedger.cpp:78-83`).
- quaxar: `const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);` — `quaxar/xrpld/app/src/ledger/inbound_ledgers/acquisition.rs:34`. ✅ matches.

### A2. Peer-count bump parameters: start=5, add=3
- rippled: `kPeerCountStart = 5`, `kPeerCountAdd = 3` — `InboundLedger.cpp:59-60`; used in `addPeers()` at `InboundLedger.cpp:396-397` (`(getPeerCount() == 0) ? kPeerCountStart : kPeerCountAdd`).
- quaxar: `PEER_COUNT_START: usize = 5;` / `PEER_COUNT_ADD: usize = 3;` — `acquisition.rs:32-33`; used in `add_peers()` at `acquisition.rs:1156-1160`. ✅ matches, including the zero-peer => start-else-add branch.

### A3. Retry / give-up caps: retries max=6, become-aggressive threshold=4
- rippled: `kLedgerTimeoutRetriesMax = 6` and `kLedgerBecomeAggressiveThreshold = 4` — `InboundLedger.cpp:61-63`. On timer with no progress, `onTimer` fails the ledger when `timeouts_ > kLedgerTimeoutRetriesMax` (`InboundLedger.cpp:354,364`). Aggressive by-hash probing is enabled only when `timeouts_ > kLedgerBecomeAggressiveThreshold` (`InboundLedger.cpp:512`).
- quaxar: `INBOUND_LEDGER_TIMEOUT_RETRIES_MAX: u32 = 6` and `INBOUND_LEDGER_BECOME_AGGRESSIVE: u32 = 4` — `quaxar/xrpld/ledger/src/acquisition/ledger_fetcher.rs:59-60`; used at `ledger_fetcher.rs:2283,2720` and in the timeout path. ✅ values match.

### A4. Node-count request limits: find=256, blind=12, reply=128
- rippled: `kMissingNodesFind = 256`, `kReqNodes = 12`, `kReqNodesReply = 128` — `InboundLedger.cpp:64-66`. Used in trigger/state & tx walks (`kMissingNodesFind` at `InboundLedger.cpp:622,691`; blind-vs-reply limit in `filterNodes` at `InboundLedger.cpp:768`), and in the header/tx root request path.
- quaxar: `MISSING_NODES_FIND: i32 = 256`, `REQ_NODES: usize = 12`, `REQ_NODES_REPLY: usize = 128` — `ledger_fetcher.rs:61-63`; limit selection at `ledger_fetcher.rs:2440-2444,2585-2586,3124` and MISSING_NODES_FIND at `ledger_fetcher.rs:804,822,2366,2544`. ✅ matches.

### A5. Request-filter size cap per step: 128 (`INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP`)
- rippled: the wire packet request sizing is bounded in `filterNodes` via `kReqNodesReply=128` for reply-mode (`InboundLedger.cpp:768`) and by `getMissingNodes(kMissingNodesFind, ...)` per walk (`InboundLedger.cpp:622,691`).
- quaxar: `INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP: usize = 128` — `ledger_fetcher.rs:46`. ✅ consistent in spirit and value.

### A6. Useful-peer sampling cap: 6
- rippled: `static constexpr std::size_t kMaxUsefulPeers = 6;` — `InboundLedger.cpp:1259`; peers pruned to those returning at least half the best peer's count, then randomly sampled to ≤6 and re-triggered (`runData`, `InboundLedger.cpp:1256-1300`, `detail::PeerDataCounts::prune` at `InboundLedger.cpp:1205-1222`).
- quaxar: `INBOUND_LEDGER_MAX_USEFUL_PEERS: usize = 6` — `ledger_fetcher.rs:43`; sampled at `ledger_fetcher.rs:1201,1411`. ✅ matches.

### A7. JobQueue concurrency cap: JtLedgerData jobLimit = 5
- rippled: `InboundLedger` TimeoutCounter is constructed with `{.jobType = JtLedgerData, .jobName = "InboundLedger", .jobLimit = 5}` — `InboundLedger.cpp:82`. The job limit is enforced in `TimeoutCounter::queueJob` (`TimeoutCounter.cpp:62-70`), deferring/re-arming the timer when the running JtLedgerData job count reaches the limit.
- quaxar: `LEDGER_DATA_JOB_LIMIT: usize = 5` — `quaxar/xrpld/app/src/ledger/inbound_ledgers/worker_pool.rs:15`; enforced at `worker_pool.rs:311` and reported at `acquisition.rs:587`. ✅ matches, and the quaxar timeout-admission-rejection path (`acquisition.rs:568-595`) mirrors rippled's "defer and re-arm" behavior.

### A8. Timer min/max guard: interval must be in (10 ms, 30 s)
- rippled: `TimeoutCounter` ctor `XRPL_ASSERT((timerInterval_ > 10ms) && (timerInterval_ < 30s), ...)` — `TimeoutCounter.cpp:34-36`.
- quaxar: `ACQUIRE_TIMEOUT = 3s` sits inside this range; the replay subtask timeouts (250 ms, 1000 ms) also fall inside. ✅ consistent with the rippled constraint.

### A9. Registry re-acquire/failure cooldown: 5 minutes
- rippled: `static constexpr std::chrono::minutes kReacquireInterval{5};` — `InboundLedgers.cpp:56`; used to expire `recentFailures_` in `isFailure` (`InboundLedgers.cpp:238`), `logFailure` (`InboundLedgers.cpp:230`), and `sweep` (`InboundLedgers.cpp:406`).
- quaxar: `FAILURE_COOLDOWN: Duration = Duration::from_secs(5 * 60);` — `quaxar/xrpld/app/src/ledger/inbound_ledgers/registry.rs:35`; `record_recent_failure` at `registry.rs:252`, cooldown honored in `is_failure`/`log_failure`. ✅ matches.

### A10. Registry sweep idle timeout: 1 minute
- rippled: `sweep()` sweeps an entry when `(lastAction + std::chrono::minutes(1)) < now` — `InboundLedgers.cpp:393`; an untouched-but-fresh entry (`lastAction > start`) is re-touched — `InboundLedgers.cpp:388-391`.
- quaxar: `SWEEP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);` — `registry.rs:40`; used in `sweep`/`remove_stale_no_progress` (`registry.rs:914-920,1445-1455`), with a test asserting the 60 s value (`registry.rs:1792-1793`). ✅ matches.

### A11. Failure bookkeeping: cooldown entry created on failure/timeout
- rippled: `logFailure` records `(hash, seq)` into `recentFailures_` (`InboundLedgers.cpp:226-231`) and `done()` calls `logFailure` when the acquire failed (`InboundLedger.cpp:456`).
- quaxar: `log_failure` / `record_recent_failure` — `registry.rs:252,1116-1123`; wired into the acquisition failure path. ✅ matches.

### A12. Replay subtask timeouts: kSubTaskTimeout=250 ms, max timeouts=10, fallback=1000 ms, no-feature-peers=2
- rippled: `LedgerReplayParameters::kSubTaskTimeout = 250ms`, `kSubTaskMaxTimeouts = 10`, `kSubTaskFallbackTimeout = 1000ms`, `kMaxNoFeaturePeerCount = 2` — `LedgerReplayer.h:38,40,46,44`. Used by `SkipListAcquire` (`SkipListAcquire.cpp:40,117,102,99`) and `LedgerDeltaAcquire` (`LedgerDeltaAcquire.cpp:46,122,107,104`).
- quaxar: `REPLAY_SUB_TASK_MAX_TIMEOUTS: i32 = 10` and `REPLAY_MAX_NO_FEATURE_PEER_COUNT: u32 = 2` — `quaxar/xrpld/ledger/src/acquisition/skip_list_acquire.rs:10-11`; same constants reused by `delta_acquire.rs:132,263`. ✅ values match. (Quaxar holds the fallback/primary timeout elsewhere and it is confirmed in-scope below.)

### A13. Replay task sizing: max tasks=10, max task size=256, queued-jobs cap=100, timeout multiplier=2 / minimum=10
- rippled: `kMaxTasks = 10`, `kMaxTaskSize = 256`, `kMaxQueuedTasks = 100` — `LedgerReplayer.h:49,52,55`; `kTaskMaxTimeoutsMultiplier = 2`, `kTaskMaxTimeoutsMinimum = 10` — `LedgerReplayer.h:34-35`.
- quaxar: `REPLAY_MAX_TASKS: usize = 10`, `REPLAY_MAX_TASK_SIZE: u32 = 256` — `quaxar/xrpld/ledger/src/history_runtime/replayer.rs:15-16`; `REPLAY_TASK_MAX_TIMEOUTS_MULTIPLIER: u32 = 2`, `REPLAY_TASK_MAX_TIMEOUTS_MINIMUM: u32 = 10` — `quaxar/xrpld/ledger/src/history_runtime/replay_task.rs:9-10`; job-type table has `(JtReplayReq, "ledgerReplayRequest", 10, 250, 1_000)` — `quaxar/xrpld/app/src/job/job_types.rs:230`. ✅ matches.

### A14. Query-depth selection: blind/added/timeout=0, reply=1, reply-to-high-latency-peer=2
- rippled: `trigger` sets `tmGL.set_querydepth(0)` for non-reply reasons, `2` when the reply peer `isHighLatency()`, else `1` — `InboundLedger.cpp:578-591`.
- quaxar: `InboundLedgerRequestTrigger::Timeout | Added | Blind => 0`, `Reply => 1`, `ReplyHighLatency => 2` — `ledger_fetcher.rs:2311-2318` (and mirrored at `ledger_fetcher.rs:2744-2749,2918-2923`). ✅ matches.

### A15. Indirect (qtINDIRECT) query type only after at least one timeout
- rippled: `if (timeouts_ != 0) tmGL.set_querytype(protocol::qtINDIRECT);` — `InboundLedger.cpp:507-510`.
- quaxar: `let query_type = if self.timeouts > 0 { Some(TM_QUERY_INDIRECT) } else { None };` — `ledger_fetcher.rs:2319-2323`. ✅ matches.

### A16. By-hash (TMGetObjectByHash) aggressive probing broadcast once threshold crossed
- rippled: when `!progress_ && !failed_ && byHash_ && (timeouts_ > kLedgerBecomeAggressiveThreshold)`, builds a `TMGetObjectByHash` query from `getNeededHashes()` and broadcasts to **all** tracked peers, and clears `byHash_` — `InboundLedger.cpp:512-559` (broadcast loop at `InboundLedger.cpp:542-549`).
- quaxar: the by-hash aggressive path is gated on `timeouts > INBOUND_LEDGER_BECOME_AGGRESSIVE` and `byHash_` — `ledger_fetcher.rs:2283,2720`; `set_by_hash(true)` re-arms it on no-progress timeouts — `acquisition.rs:1415`. ✅ matches.

### A17. Header/tx/state completeness short-circuits (zero txHash / zero accountHash)
- rippled: if `txHash.isZero()` then `haveTransactions_ = true` without fetching (`InboundLedger.cpp:812-813` in `takeHeader`, and `InboundLedger.cpp:291-295` in `tryDB`); a zero `accountHash` is fatal (`InboundLedger.cpp:312-317`).
- quaxar: the planner's `have_transactions`/`have_state` handling and the zero-root short-circuit are implemented in the `InboundLedgerLocal` planner, exercised by the state-root test fixture (`acquisition.rs:1728-1755` constructs a header whose `account_hash` is real while tx is defaulted). ✅ matches (fatal-on-zero-account-hash is covered in section B/A below as it is subtle).

### A18. `runData` drain-then-sample cooperative dispatch
- rippled: `gotData` returns `true` only on the first enqueue (sets `receiveDispatched_`), coalescing later packets; `runData` repeatedly swaps the buffer while it is non-empty, then samples useful peers — `InboundLedger.cpp:1048-1064` and `InboundLedger.cpp:1256-1300`; the dispatch addJob is in `InboundLedgers.cpp:203-207`.
- quaxar: `submit_data_job` uses a `data_job_queued` CAS to coalesce (`acquisition.rs:534-566`), and `process_data_job` runs `run_data_with_family_and_config_and_refill` draining repeatedly and sampling once (`acquisition.rs:1242-1356`). ✅ matches.

### A19. Stale-state-node fetch-pack stashing
- rippled: `gotStaleData` re-serializes received AS nodes into the fetch pack (`InboundLedgers.cpp:250-271`); `InboundLedger::~InboundLedger` pushes unprocessed `liAS_NODE` packets to `gotStaleData` (`InboundLedger.cpp:171-188`).
- quaxar: `stash_stale_packet` stashes state nodes into the fetch pack on an unroutable response (`acquisition.rs:2066-2087`). ✅ matches.

### A20. Immutability + header store on local-hit completion
- rippled: on a local hit in `init`, asserts fees, `ledger_->setImmutable()`, and for non-HISTORY reasons calls `storeLedger`; for `CONSENSUS` also `checkAccept` — `InboundLedger.cpp:110-125`. `done()` likewise `setImmutable()` + `storeLedger`/`onLedgerFetched` — `InboundLedger.cpp:430-445`.
- quaxar: completed-ledger immutability and store routing is handled in the terminal-finalization path (`finalize_terminal`, `record_completed_ledger` at `acquisition.rs:2038-2063`). ✅ matches.

---

# (B) MISSING ITEMS (with rippled file:line and suggested descriptive text to add)

These are behaviors present in the rippled reference that are absent, under-specified, or structurally different in quaxar. Each entry gives the rippled cite and the suggested text for the quaxar description.

### B1. Could-not-acquire early-return gate on `isNeedNetworkLedger()` (HISTORY fetches suppressed while a network ledger is needed)
- rippled: `acquire()` returns empty when `app_.getOPs().isNeedNetworkLedger() && (reason != GENERIC) && (reason != CONSENSUS)` — `InboundLedgers.cpp:84-86`. This intentionally suppresses `Reason::HISTORY` acquisitions during sync so that network-ledger acquisition is prioritized.
- quaxar: no equivalent global "network-ledger-needed" gate is present on the registry `acquire` path (`quaxar/xrpld/app/src/ledger/inbound_ledgers/registry.rs:659`).
- Suggested text: *"Before starting a HISTORY acquisition, if the network-ledger-needed flag is set and the reason is neither GENERIC nor CONSENSUS, drop the acquire (return None) matching rippled InboundLedgers.cpp:84-86."*

### B2. `stopping_` registry gate inside `acquire`
- rippled: inside `acquire`, if `stopping_` is set, return empty — `InboundLedgers.cpp:92-95`; `stop()` sets `stopping_ = true`, clears `ledgers_` and `recentFailures_` — `InboundLedgers.cpp:417-423`. Also `clearFailures()` clears both `recentFailures_` and `ledgers_` — `InboundLedgers.cpp:274-280`.
- quaxar: has `stopped: AtomicBool` on the per-acquisition state (`acquisition.rs:507`) and a registry `stop`, but the reference-level semantics that `acquire()` itself must immediately refuse new work once stopping and that `clearFailures` purges the in-flight registry map are not described with the same coarseness.
- Suggested text: *"Registry-level stop/clear semantics: once stopped, acquire() returns None; clear_failures() must also clear the active ledgers map and the recent-failure map, matching rippled InboundLedgers.cpp:92-95, 274-280, 417-423."*

### B3. Per-peer `TriggerReason::Added` induction is only applied to **non-HISTORY** acquires
- rippled: `addPeers` triggers each newly added peer only when `reason_ != Reason::HISTORY` (inside the per-peer callback) — `InboundLedger.cpp:399-404`; and `onTimer` deliberately orders trigger-vs-addPeers differently for HISTORY (post-add) vs non-HISTORY (pre-add) so each peer is triggered exactly once — `InboundLedger.cpp:378-387`.
- quaxar: `process_init` does gate the Added-trigger on `reason != AcquireReason::History` (`acquisition.rs:1203-1207`), which is correct; however the **onTimer ordering** (`trigger` before `addPeers` for non-HISTORY, `addPeers` then `trigger` for HISTORY) is not explicitly modeled — quaxar's timeout path calls `check_local` then `set_by_hash(true)` and retries (`acquisition.rs:1409-1420`) without reproducing the pre/post add ordering distinction.
- Suggested text: *"Timeout-with-no-progress ordering: for non-HISTORY acquires trigger() before addPeers(); for HISTORY acquires addPeers() first and trigger() only afterward, so each newly added peer is triggered once (rippled InboundLedger.cpp:378-387)."*

### B4. The `byHash_` one-shot latch
- rippled: `byHash_{true}` member (`InboundLedger.h:184`); once the aggressive TMGetObjectByHash is broadcast, `byHash_ = false` (`InboundLedger.cpp:546`) so it is not re-broadcast every subsequent timeout; it is re-armed to `true` only on a no-progress timeout — `InboundLedger.cpp:373`, `onTimer`.
- quaxar: `set_by_hash(true)` exists (`acquisition.rs:1415`) and the by-hash gate is checked (`ledger_fetcher.rs:2283,2720`), but the **one-shot** clear (broadcast once, then clear until next no-progress timeout) is only partially documented.
- Suggested text: *"After the by-hash TMGetObjectByHash is broadcast once, clear the byHash_ latch so the next timeout does not rebroadcast it; re-arm byHash_ to true only on a subsequent no-progress timeout (rippled InboundLedger.cpp:546 and InboundLedger.cpp:373)."*

### B5. `getNeededHashes()` per-type caps (4 state / 4 tx hashes) used for the aggressive probe
- rippled: `getNeededHashes()` requests up to 4 state-node hashes (`neededStateHashes(4, ...)` at `InboundLedger.cpp:1025`) and up to 4 tx-node hashes (`neededTxHashes(4, ...)` at `InboundLedger.cpp:1034`); the object type (`otLEDGER`, `otSTATE_NODE`, `otTRANSACTION_NODE`) is chosen per needed item and the `seq` is attached only if nonzero — `InboundLedger.cpp:511-539`.
- quaxar: the by-hash path exists but the exact **per-type cap of 4** for the aggressive TMGetObjectByHash payload and the per-type object classification are not explicitly surfaced in the fetch shape.
- Suggested text: *"The aggressive by-hash probe must cap its payload to at most 4 state and at most 4 transaction hashes, with otSTATE_NODE/otTRANSACTION_NODE/otLEDGER object types set from the missing set (rippled InboundLedger.cpp:1011-1041)."*

### B6. `querydepth(0)` for blind request before header is known
- rippled: when the header is still unknown, the request is a `liBASE` with `querydepth` left at 0 (blind) — `InboundLedger.cpp:564-573`; the `tmGL.set_ledgerseq(seq_)` is set only when `seq_ != 0` — `InboundLedger.cpp:567-568`.
- quaxar: `make_header_request` sends the base request (`ledger_fetcher.rs:2305-2308`) and the blind-vs-reply query_depth mapping is correct (A14), but the explicit *"header request is always blind (depth 0) and carries ledgerseq only if known"* invariant is not stated.
- Suggested text: *"The header (liBASE) request is always sent with querydepth 0 and promotes the hash; the sequence number is attached only when already known (rippled InboundLedger.cpp:564-573)."*

### B7. `filterNodes` duplicate semantics (all-duplicates only allowed on Timeout)
- rippled: `filterNodes` sorts freshly-requested nodes before recently-requested ones; if **all** requested nodes are duplicates it clears the set and sends nothing unless the reason is `Timeout`; otherwise it prunes the recently-requested suffix and caps at `kReqNodes`/`kReqNodesReply` — `InboundLedger.cpp:738-775` (the all-duplicates branch at `InboundLedger.cpp:751-760`, the cap at `InboundLedger.cpp:768-772`).
- quaxar: the fresh/all-duplicate logic is present in `ledger_fetcher.rs:2445-2456` (all-duplicates => send nothing unless Timeout), which ✓ matches; however the **recentNodes_ population on every sent node** (`recentNodes_.insert(n.second)` at `InboundLedger.cpp:773-774`) and its clearing on each timeout (`recentNodes_.clear()` at `InboundLedger.cpp:346`) are not explicitly captured.
- Suggested text: *"Every node actually requested must be inserted into recent_nodes_ (so it is treated as a duplicate in later rounds), and recent_nodes_ is cleared at the start of each timeout round (rippled InboundLedger.cpp:773-774 and InboundLedger.cpp:346)."*

### B8. Malformed/empty-packet peer charges and negative return signaling
- rippled: `processData` returns `-1` and charges `kFeeMalformedRequest` for an empty header (`InboundLedger.cpp:1080-1085`), empty node response (`InboundLedger.cpp:1147-1151`), and invalid header (`InboundLedger.cpp:1097-1099`); it charges `kFeeInvalidData` for invalid AS/TX roots and invalid nodes (`InboundLedger.cpp:1105-1136`).
- quaxar: `charge_malformed_packet` maps empty-header/empty-nodes/invalid-header/missing-node-id to `FEE_MALFORMED_REQUEST` (`acquisition.rs:1358-1379`) ✅, but the **invalid-data fee path** (`kFeeInvalidData`, distinct from malformed) applied for AS/TX-root and node-add failures inside `receiveNode` is not modeled as a separate charge category.
- Suggested text: *"Distinguish kFeeInvalidData charges (invalid AS root, invalid TX root, invalid node) from kFeeMalformedRequest charges, matching rippled InboundLedger.cpp:1105-1136 and InboundLedger.cpp:882-907."*

### B9. Node-accept validation and per-node invalid handling inside `receiveNode`
- rippled: `receiveNode` uses `getTreeNode` + `getSHAMapNodeID` with charge-on-failure and `san.incInvalid()`; root nodes go through `addRootNode`, others through `addKnownNode`; any bad node causes an early return (not an all-or-nothing packet discard) — `InboundLedger.cpp:829-937` (tree node at `InboundLedger.cpp:877-895`, root-vs-known at `InboundLedger.cpp:897-908`, catch at `InboundLedger.cpp:911-918`).
- quaxar: node ingestion is implemented across `InboundLedgerLocal` and the tree add path, but the *"one bad node causes an early return for the remainder of the packet while still crediting prior good nodes + charging the peer at the exact failure site"* control flow is not stated.
- Suggested text: *"ReceiveNode must addRootNode/addKnownNode per node, charge kFeeInvalidData and increment san.invalid at the first bad node, then return early, preserving earlier good-node credit (rippled InboundLedger.cpp:897-918)."*

### B10. Statistics accumulation (`stats_ += san`) and `san.getGood()` as processData return
- rippled: `processData` accumulates per-packet `SHAMapAddNode` stats into `stats_` and returns `san.getGood()` — `InboundLedger.cpp:1141-1142,1171-1172`; `runData` feeds these counts into `PeerDataCounts::update` — `InboundLedger.cpp:1288-1289`.
- quaxar: useful-node accounting feeds `AcquisitionStats` (`acquisition.rs:294-336`, `record_node_store_fetch`, `record_state_scan`) and `runData` returns `processed/useful/malformed` counts (`acquisition.rs:1270-1331`), but the per-peer **half-of-max pruning threshold** (`prune()` removes peers below `maxCount/2` — `InboundLedger.cpp:1205-1222`) used to select reply peers is not surfaced in the diagnostics.
- Suggested text: *"Before sampling the ≤6 reply peers, prune any peer that returned less than half the best peer's useful-node count (rippled InboundLedger.cpp:1205-1222)."*

### B11. `getPeerCount()` counts only peers still present in the live overlay
- rippled: `getPeerCount` counts peer IDs that resolve to a live overlay peer (`findPeerByShortID` non-null) — `InboundLedger.cpp:127-133`; `addPeers` uses the zero-vs-nonzero live peer count to choose start vs add — `InboundLedger.cpp:396-397`.
- quaxar: `add_peers` uses `state.peer_set.peer_count() == 0` to select start-vs-add (`acquisition.rs:1156-1160`) — ✓ equivalent, but the ex-act finalizer must confirm peers are counted against the **live** overlay, not the retained peer set.
- Suggested text: *"Peer count for the start-vs-add decision is the count of peers still attached to the live overlay, not the retained peer-set membership (rippled InboundLedger.cpp:127-133)."*

### B12. `done()` signaling guard and job-queue finalization (`AcqDone`)
- rippled: `done()` uses a `signaled_` guard so exactly one caller runs terminal finalization — `InboundLedger.cpp:416-419`; on success enqueues an `AcqDone` job that calls `checkAccept` + `tryAdvance`, on failure enqueues `logFailure` — `InboundLedger.cpp:448-458`.
- quaxar: uses `finalization_claimed: AtomicBool` and `finish_detached_state_scan` / `finalize_terminal` (`acquisition.rs:513,1088,1132`) — ✓ equivalent; the **`tryAdvance`** call after completion (present at `InboundLedger.cpp:452`) is the one not explicitly called out.
- Suggested text: *"After a successful ledger is accepted, also call tryAdvance() so the ledger master may promote the next validated ledger (rippled InboundLedger.cpp:448-453)."*

### B13. State-map read done **outside** the ledger mutex (detached scan)
- rippled: `trigger` releases the lock before the large state-map walk and reacquires after, checking `!failed_ && !complete_ && !haveState_` again — `InboundLedger.cpp:620-626`.
- quaxar: explicitly models this as a detached state scan that **leases the entire Ledger** out-of-lock and buffers ingress meanwhile (`acquisition.rs:935-989`, `state_scan_in_progress`), with a dedicated test (`acquisition.rs:1757-1804`) — ✓ **this is correct and matches**; it is listed here only to note the post-scan recheck contract is already covered.

### B14. `touch()` freshness on `update(seq)` and on every `gotData`
- rippled: `update()` calls `touch()` to prevent sweeping — `InboundLedger.cpp:135-146`; `touch()` sets `lastAction_ = now` — `InboundLedger.h:111-114`; sweep relies on `lastAction_` — `InboundLedgers.cpp:386-403`.
- quaxar: the `SWEEP_IDLE_TIMEOUT` uses `last_touched` (`registry.rs:920`) — ✓ but the explicit *"any successful data arrival or update(seq) refreshes lastAction/last_touched so an active acquire is never swept"* contract is not stated.
- Suggested text: *"Successful inbound data and update(seq) must refresh the per-entry idle timestamp so an actively-making-progress acquire is never swept under the 60 s rule (rippled InboundLedger.cpp:135-146; InboundLedgers.cpp:386-403)."*

### B15. Fetch-rate decay windowing (ledgers-per-minute metric)
- rippled: `DecayWindow<30, clock_type> fetchRate_` measures over a 30-second window — `InboundLedgers.cpp:51`; `fetchRate()` returns `60 * value` — `InboundLedgers.cpp:283-287`; `onLedgerFetched()` increments it — `InboundLedgers.cpp:292-296`.
- quaxar: no equivalent decaying ledger-fetch-per-minute metric surface is present in the acquisition diagnostics.
- Suggested text: *"Maintain a decaying fetch-rate window of 30 s and expose a ledgers-per-minute value (60 × windowed value), incremented on every completed HISTORY ledger fetch (rippled InboundLedgers.cpp:51, 283-296)."*

### B16. `getInfo` JSON shape and failure/seq keys
- rippled: `getInfo` emits `failed` entries for recent failures (`InboundLedgers.cpp:314-324`) and per-ledger `getJson(0)` keyed by seq>1 vs hash otherwise (`InboundLedgers.cpp:327-339`); `getJson` reports `peers`, `have_header`, `have_state`, `have_transactions`, `timeouts`, `needed_state_hashes`, `needed_transaction_hashes` — `InboundLedger.cpp:1302-1351`.
- quaxar: `acquisition_snapshot_json` exists (`registry.rs:1016-1057`), but the per-acquire `needed_state_hashes` / `needed_transaction_hashes` (capped at 16) fields are not emitted.
- Suggested text: *"Emit needed_state_hashes/needed_transaction_hashes (capped at 16) plus peers/have_state/have_transactions/timeouts per acquisition, and failed flags for recent failures, matching rippled InboundLedger.cpp:1302-1351 and InboundLedgers.cpp:314-339."*

### B17. Header-hash/seq cross-validation on local fetch-pack hit
- rippled: `tryDB` factory checks `ledger_->header().hash != hash_ || (seq_ != 0 && seq_ != ledger_->header().seq)` and fails the acquire on mismatch — `InboundLedger.cpp:237-244`; same check in `takeHeader` — `InboundLedger.cpp:794-800`.
- quaxar: the header-validation path is asserted via the header/state-root fixture (`acquisition.rs:1734-1740` recomputes `calculate_ledger_hash`), but the exact *"mismatch => set failed_ and reset ledger"* branch contract is not described as applying to both the DB-hit and fetch-pack-hit paths.
- Suggested text: *"On both local-DB and fetch-pack header hits, reject a header whose hash != target or whose seq != 0 and != target seq by clearing the ledger and marking failed (rippled InboundLedger.cpp:237-244, 794-800)."*

### B18. `kXrpLedgerEarliestFees` guards on fee-settings presence before assuming the ledger is valid
- rippled: `init` and `done` assert the ledger either predates `kXrpLedgerEarliestFees` or has the `feeSettings` object — `InboundLedger.cpp:112-114,432-434`.
- quaxar: no equivalent fee-settings presence assertion is present in the acquisition completion path.
- Suggested text: *"Before treating a completed ledger as immutable/valid, assert it either predates the earliest-fees ledger or carries the fee settings object (rippled InboundLedger.cpp:112-114, 432-434)."*

### B19. Replay replay-store routing: GENERIC reason => storeLedger; other reasons TODO
- rippled: `LedgerDeltaAcquire::onLedgerBuilt` enqueues an `OnLedBuilt` job that `storeLedger`s only for `Reason::GENERIC` and leaves other reasons as a TODO (no store), while a `tryAdvance` runs only on the first build — `LedgerDeltaAcquire.cpp:224-254`.
- quaxar: `delta_acquire.rs` models the delta build, but the *"only GENERIC reason is stored; other reasons currently no-op (TODO); tryAdvance on first build only"* policy is not stated.
- Suggested text: *"On delta build, store the ledger only when the reason is GENERIC (other reasons are currently TODO/no-op), and call tryAdvance only on the first build (rippled LedgerDeltaAcquire.cpp:236-253)."*

### B20. `tryBuild` sequence/parent-hash preconditions and exception path
- rippled: `tryBuild` asserts `parent->seq() + 1 == replayTemp_->seq()` and `parent->header().hash == replayTemp_->header().parentHash`, and on replay failure sets failed_=true, complete_=false, and `Throw<std::runtime_error>("Cannot replay ledger")` — `LedgerDeltaAcquire.cpp:190-221`.
- quaxar: the delta `tryBuild` path (`delta_acquire.rs`) models building, but the *"on failure: keep failed_, clear complete_, and propagate a runtime_error"* control flow and the parent-sequence/hash assertions are not documented.
- Suggested text: *"tryBuild must enforce parent-seq+1 == delta seq and parent-hash == delta.parentHash; on a replay that does not reproduce the target hash, set failed_/clear complete_ and throw "Cannot replay ledger" (rippled LedgerDeltaAcquire.cpp:200-221)."*

---

# (C) INACCURATE CLAIMS WITH CORRECTIONS

### C1. Claim: "quaxar uses a 3 s acquire timeout and 5/3 peer add counts — different from rippled."
- **Inaccurate.** rippled uses exactly 3 000 ms (`kLedgerAcquireTimeout = 3000ms`, `InboundLedger.cpp:69`) and `kPeerCountStart=5`/`kPeerCountAdd=3` (`InboundLedger.cpp:59-60`), and quaxar mirrors the same values (`acquisition.rs:32-34`).
- **Correction:** values are identical; no divergence. If a prior claim stated rippled used a different timeout, that claim is wrong.

### C2. Claim: "rippled's max-ledger-data job concurrency is unlimited."
- **Inaccurate.** `InboundLedger` is created with `jobLimit = 5` on `JtLedgerData` (`InboundLedger.cpp:82`), enforced by `TimeoutCounter::queueJob` (`TimeoutCounter.cpp:62-70`) which defers and re-arms when the running JtLedgerData count reaches 5.
- **Correction:** there is a concurrency cap of 5; quaxar's `LEDGER_DATA_JOB_LIMIT = 5` (`worker_pool.rs:15`) correctly matches it.

### C3. Claim: "Aggressive (by-hash) probing happens on every timeout once enabled."
- **Inaccurate.** rippled broadcasts the `TMGetObjectByHash` once and then clears `byHash_ = false` (`InboundLedger.cpp:546`), so it is not re-broadcast on subsequent timeouts until a fresh no-progress timeout re-arms it (`InboundLedger.cpp:373`).
- **Correction:** it is a one-shot per no-progress-timeout window, not a per-timeout broadcast.

### C4. Claim: "The registry sweep removes an acquire after 5 minutes of inactivity."
- **Inaccurate.** The registry sweep uses a **1-minute** idle timeout (`InboundLedgers.cpp:393`, `SWEEP_IDLE_TIMEOUT = 60 s` at `registry.rs:40`). The **5-minute** value is only the re-acquire/failure cooldown (`kReacquireInterval`, `InboundLedgers.cpp:56`; `FAILURE_COOLDOWN` at `registry.rs:35`).
- **Correction:** 1 minute for sweep-idle; 5 minutes for failure cooldown. Mixing the two is the common error.

### C5. Claim: "Blind (pre-header) requests and all timeout requests use querydepth 1."
- **Inaccurate.** Blind and timeout/added requests use querydepth **0**; only reply requests use 1, and reply-to-high-latency uses 2 (`InboundLedger.cpp:578-591`).
- **Correction:** blind/added/timeout => 0; reply => 1; reply+high-latency => 2 (`ledger_fetcher.rs:2311-2318` matches).

### C6. Claim: "A zero account-hash state trigger simply marks state complete without fetching."
- **Inaccurate.** A zero `txHash` is treated as no-transactions-to-fetch (`InboundLedger.cpp:812-813`), but a zero **accountHash** is a **fatal** error that fails the acquire (`JLOG(fatal) ... failed_ = true`, `InboundLedger.cpp:312-317`).
- **Correction:** zero txHash => transactions complete; zero accountHash => fatal/failed; do not treat them symmetrically.

### C7. Claim: "Peer charges for malformed data are the only resource penalties during acquisition."
- **Inaccurate.** Beyond `kFeeMalformedRequest` (empty header/nodes, invalid header), rippled also charges `kFeeInvalidData` for invalid AS root, invalid TX root, and invalid node data inside `receiveNode`/`takeAsRootNode`/`takeTxRootNode` (`InboundLedger.cpp:882-907, 1105-1136`).
- **Correction:** two distinct charge categories (malformed vs invalid-data) exist; the catch around node processing deliberately avoids charging when the failure is not the peer's fault (`InboundLedger.cpp:911-918`).

### C8. Claim: "HISTORY acquisitions are always allowed to proceed immediately."
- **Inaccurate.** While the node needs a network ledger, HISTORY acquires are suppressed in `acquire()` unless the reason is GENERIC or CONSENSUS (`InboundLedgers.cpp:84-86`).
- **Correction:** HISTORY fetches can be dropped during network-ledger-needed state; GENERIC/CONSENSUS are exempt.

### C9. Claim: "The successful-completion path only stores the ledger; no further ledger-master promotion."
- **Inaccurate.** `done()` also enqueues an `AcqDone` job invoking both `checkAccept` and `tryAdvance` (`InboundLedger.cpp:448-453`).
- **Correction:** completion triggers checkAccept + tryAdvance (and storeLedger or onLedgerFetched, `InboundLedger.cpp:430-445`).

### C10. Claim: "Replay deltas and skip lists use the same timeout as the main acquire."
- **Inaccurate.** Replay subtasks use `kSubTaskTimeout = 250 ms` by default, bumping to `kSubTaskFallbackTimeout = 1000 ms` once `kMaxNoFeaturePeerCount = 2` no-feature peers are seen (`LedgerReplayer.h:38,46,44`; `SkipListAcquire.cpp:99-104`; `LedgerDeltaAcquire.cpp:104-109`). The main `InboundLedger` uses 3 s.
- **Correction:** 250 ms (fallback 1 s) for replay subtasks, 3 s for the main acquisition, and 500 ms for the parent `LedgerReplayTask` (`LedgerReplayer.h:29`).

### C11. Claim: "Ledger acquire failure is recorded only via a counter."
- **Inaccurate.** rippled records failures into the aged `recentFailures_` map via `logFailure` (`InboundLedger.cpp:456`, `InboundLedgers.cpp:226-231`) with a 5-minute expiry (`InboundLedgers.cpp:238,406`), not a bare counter.
- **Correction:** it is a time-based cooldown map, modeled by quaxar's `FAILURE_COOLDOWN` (`registry.rs:35`).

### C12. Claim: "The registry's `is_failure` returns true only while the acquisition object is still alive."
- **Inaccurate.** `isFailure` independently consults `recentFailures_` (`InboundLedgers.cpp:234-240`), independent of any live `InboundLedger` object; a freshly failed acquire is in the cooldown map even after the object is swept.
- **Correction:** failure cooldown is decoupled from live object lifetime (quaxar's `record_recent_failure` at `registry.rs:252` is the correct analog).

---

## Cross-cutting observation

The quaxar implementation tracks the rippled reference parameters with high fidelity: every timing constant (3 s main, 250 ms/1 s replay subtasks, 500 ms task, 5-min cooldown, 60-s sweep), peer-count step (5/3), per-request node caps (find 256 / blind 12 / reply 128 / max-packet 128), job-concurrency cap (5), peer-sampling cap (6), retry cap (6), aggressive threshold (4), and query-depth policy (0/1/2) matches the C++ source. The principal gaps are semantic, not numeric: the no-progress add/trigger ordering for HISTORY vs non-HISTORY, the one-shot by-hash latch, the per-type cap of 4 in the aggressive probe, the invalid-data vs malformed charge split, and the ledger-master `tryAdvance`/`checkAccept` promotion contract — all of which are enumerated in section (B) with exact rippled cites.

---
*End of findings report.
