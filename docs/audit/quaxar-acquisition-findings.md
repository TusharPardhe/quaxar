# Quaxar Ledger-Acquisition Subsystem — Source-Code Audit FINDINGS

**Auditor:** quaxar-auditor (Rust reference auditor)
**Reference tree:** `/Users/tusharpardhe/Documents/xrpl/quaxar/xrpld`
**Compared against:** rippled C++ reference under `/Users/tusharpardhe/Documents/xrpl/rippled/src/xrpld` and the companion `rippled-acquisition-findings.md`.
**Scope:** `InboundLedger` per-hash lifecycle, `InboundLedgers` registry, worker/timer machinery, `LedgerFetcher` planning/ingestion, `InboundTransactions`/`TransactionAcquire`, `SkipListAcquire`, `LedgerDeltaAcquire`, `LedgerReplayTask`, `LedgerReplayer`, and the replay-parameter set.

---

## Scope & method

Every claim below is grounded in the actual Rust source in `/Users/tusharpardhe/Documents/xrpl/quaxar/xrpld`. Primary files:

- `app/src/ledger/inbound_ledgers/acquisition.rs` (per-hash lifecycle, timer/worker wiring)
- `app/src/ledger/inbound_ledgers/registry.rs` (global registry, failure cooldown, sweep)
- `app/src/ledger/inbound_ledgers/worker_pool.rs` (job queue + delayed-timer service)
- `ledger/src/acquisition/ledger_fetcher.rs` (planning, packet ingestion, timeout decisions)
- `ledger/src/acquisition/inbound_transactions.rs`, `ledger/src/acquisition/transaction_acquire.rs` (tx-set acquisition)
- `ledger/src/acquisition/skip_list_acquire.rs`, `ledger/src/acquisition/delta_acquire.rs` (replay subtasks)
- `ledger/src/history_runtime/replay_task.rs`, `ledger/src/history_runtime/replayer.rs` (replay task / replayer)
- `ledger/src/domain/timeout_counter.rs` (generic TimeoutCounter port)

The report is split into:
- **(A)** items the quaxar Rust implementation covers correctly (with quaxar `file:line` and the rippled analog used to confirm them),
- **(B)** items that are MISSING or divergent in quaxar (with quaxar `file:line` and the descriptive text that should be added),
- **(C)** claims that were inaccurate or stale — including citations in the existing `rippled-acquisition-findings.md` and `QUAXAR_COMPLETE_FLOW.md` that do not hold against the current Rust source — with corrections.

---

# (A) CORRECTLY-COVERED ITEMS

### A1. Per-ledger acquire timeout: 3 s
- Quaxar: `const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);` — `acquisition.rs:34`. The timer is armed with this interval (`acquisition.rs:607-613`) and re-armed on every non-terminal timeout turn (`acquisition.rs:1444`).
- rippled analog: `kLedgerAcquireTimeout = 3000ms` — `InboundLedger.cpp:69` passed as `timerInterval_` (`InboundLedger.cpp:78-83`). ✅ matches.

### A2. Peer-count bump: start=5, add=3
- Quaxar: `PEER_COUNT_START: usize = 5` / `PEER_COUNT_ADD: usize = 3` — `acquisition.rs:32-33`; the zero-peer ⇒ start / non-zero ⇒ add branch is `acquisition.rs:1156-1160`.
- rippled: `kPeerCountStart = 5`, `kPeerCountAdd = 3` — `InboundLedger.cpp:59-60`, used at `InboundLedger.cpp:396-397`. ✅ matches including the zero-peer branch.

### A3. Retry/give-up caps: timeout retries max=6, become-aggressive threshold=4
- Quaxar: `INBOUND_LEDGER_TIMEOUT_RETRIES_MAX: u32 = 6` and `INBOUND_LEDGER_BECOME_AGGRESSIVE: u32 = 4` — `ledger_fetcher.rs:59-60`. The timeout decision (`timeout_expired`) increments `timeouts` and fails once `timeouts > 6` (`ledger_fetcher.rs:3339-3343`). Aggressive by-hash probing is gated on `timeouts > 4` at both trigger sites (`ledger_fetcher.rs:2283`, `ledger_fetcher.rs:2720`).
- rippled: `kLedgerTimeoutRetriesMax = 6`, `kLedgerBecomeAggressiveThreshold = 4` — `InboundLedger.cpp:61-63`; fail at `InboundLedger.cpp:354,364`; by-hash at `InboundLedger.cpp:512`. ✅ values match.

### A4. Node-count request limits: find=256, blind/reply=12/128
- Quaxar: `MISSING_NODES_FIND: i32 = 256`, `REQ_NODES_REPLY: usize = 128`, `REQ_NODES: usize = 12` — `ledger_fetcher.rs:61-63`. `MISSING_NODES_FIND` is used in walks (`ledger_fetcher.rs:804,822,2366,2544,2817,2871,2960`). The filter selects `REQ_NODES_REPLY` for `Reply`/`ReplyHighLatency` and `REQ_NODES` otherwise (`ledger_fetcher.rs:2442-2443`, `2585-2586`, `3124-3125`, `3263-3264`).
- rippled: `kMissingNodesFind = 256`, `kReqNodes = 12`, `kReqNodesReply = 128` — `InboundLedger.cpp:64-66`; blind-vs-reply in `filterNodes` at `InboundLedger.cpp:768`. ✅ values match.

### A5. One-shot by-hash latch in the aggressive probe
- Quaxar: `process_timeout_job` sets `by_hash = true` on a no-progress timeout (`acquisition.rs:1415`). The by-hash block runs only when `by_hash && timeouts > 4` and, after emitting the request, clears the latch `by_hash = false` (`ledger_fetcher.rs:2282-2291`; prepare path `ledger_fetcher.rs:2719-2727`). This is a true one-shot latch: subsequent timeout turns that make progress cannot re-emit until another no-progress turn re-arms it.
- rippled lightweight equivalent: rippled enters by-hash mode once `timeouts_ > kLedgerBecomeAggressiveThreshold` (`InboundLedger.cpp:512`) — quaxar additionally latches one-shot per no-progress turn. ✅ implemented in quaxar.

### A6. By-hash request is single-object-type (no type mixing)
- Quaxar: `make_inbound_needed_by_hash_request` takes the first `(type, hash)` and emits only hashes whose type matches that leading type into one `TMGetObjectByHash` (`ledger_fetcher.rs:3354-3375`). This mirrors rippled's `p.first == tmBH.type()` filter.

### A7. Query-depth / query-type policy (0/1/2; INDIRECT after timeout)
- Quaxar: `query_depth` is 0 for `Timeout|Added|Blind`, 1 for `Reply`, 2 for `ReplyHighLatency` (`ledger_fetcher.rs:2311-2318`). `query_type` is `TM_QUERY_INDIRECT` whenever `timeouts > 0`, including for header requests (`ledger_fetcher.rs:2319-2323`, `747-751`).
- rippled analog: high-latency reply sets `querydepth(2)` in `InboundLedger.cpp`. ✅.

### A8. Sampled useful peers: max 6
- Quaxar: `INBOUND_LEDGER_MAX_USEFUL_PEERS: usize = 6` — `ledger_fetcher.rs:43`; peers are sampled via `sample_peer_ids(...)` / `sample_peers(..., 6)` at `ledger_fetcher.rs:1201,1229,1260,1411` (`pruned_scores` at 3438, sampler at 3452-3456).

### A9. Max packet nodes processed per step: 128
- Quaxar: `INBOUND_LEDGER_MAX_PACKET_NODES_PER_STEP: usize = 128` — `ledger_fetcher.rs:46`. Bounds the contiguous packet range processed while a worker holds the per-ledger mutation lock.

### A10. Per-type needed-hash cap: 4 state / 4 tx
- Quaxar: `INBOUND_LEDGER_MAX_NEEDED_STATE_HASHES: i32 = 4` and `INBOUND_LEDGER_MAX_NEEDED_TX_HASHES: i32 = 4` — `ledger_fetcher.rs:41-42`; consumed via `needed_state_hashes_with_family(4, ...)` and `needed_tx_hashes_with_family(4, ...)` at `ledger_fetcher.rs:3569-3570,3579-3580`. This is the mechanism that naturally caps the aggressive by-hash probe at 4 per type (see C2).

### A11. Ledger-data job admission cap: 5
- Quaxar: `LEDGER_DATA_JOB_LIMIT: usize = 5` — `worker_pool.rs:15`. `try_submit_timeout` rejects recovery work when live `ledger_data_jobs >= 5` (`worker_pool.rs:308-316`), and a false admission re-arms the timer rather than dropping work (`acquisition.rs:573-593`). Reservations are released on unwind via `LedgerDataReservation::drop` (`worker_pool.rs:37-53`).
- rippled analog: `kMaxConcurrentLedgerDataJobs`/JtLedgerData admission in `InboundLedger.cpp`. ✅.

### A12. Worker count: 64
- Quaxar: `WORKER_COUNT: usize = 64` — `registry.rs:52`, used to size the `JtLedgerData`-equivalent worker thread pool. This bounds concurrent processing but does not cap tracked acquisitions.

### A13. Failure cooldown: 5 minutes
- Quaxar: `FAILURE_COOLDOWN: Duration = Duration::from_secs(5*60)` — `registry.rs:35`. `acquire` rejects a hash still in `recent_failures` (`registry.rs:679-688`); `sweep` and `is_failure` retain/prune on the same window (`registry.rs:930-932`, `1123-1132`); `log_failure` records via `record_recent_failure` (`registry.rs:1116-1119`, `252-254`).
- rippled: `kReacquireInterval{5}` — `InboundLedgers.cpp:56`. ✅.

### A14. Sweep idle timeout: 1 minute
- Quaxar: `SWEEP_IDLE_TIMEOUT: Duration = Duration::from_secs(60)` — `registry.rs:40`; `sweep` removes entries idle > 60 s and marks their state stopped (`registry.rs:914-933`).
- rippled: `sweep` idle window in `InboundLedgers.cpp:406`. ✅.

### A15. Delayed-failure identity guard (no hash-wide cooldown from a stale acquisition)
- Quaxar: `record_recent_failure_at` refuses to create a cooldown when a worker callback's `acquisition_id` no longer matches the live entry; it preserves the *first* failure timestamp (`registry.rs:219-250`, matched by `failure_matches_entry` at 215-217). This closes the swept/replaced-acquisition failure race.

### A16. HISTORY vs non-HISTORY timeout ordering and Added-trigger suppression
- Quaxar: on the no-progress retry path, non-HISTORY work issues `Timeout` to tracked peers before `Added` to newly recruited peers, while HISTORY work calls `add_peers` first (without Added-trigger) then fans out `Timeout` (`process_timeout_job`, `acquisition.rs:1430-1442`). `process_init` also suppresses the `Added` trigger entirely for HISTORY (`acquisition.rs:1203-1207`).
- rippled analog: no-progress add/trigger ordering split in `onTimer`/`addPeers`. ✅ handled as a deliberate branch.

### A17. Timer thread never runs acquisition logic; it only enqueues
- Quaxar: `arm_timer` schedules a callback that merely clears `timer_armed` and calls `queue_timeout_job` (`acquisition.rs:597-614`); `queue_timeout_job` submits to the worker pool with admission control and re-arms on rejection (`acquisition.rs:568-595`). The `TimerService` thread loop only pops due callbacks (`worker_pool.rs:92-124`). This matches rippled `TimeoutCounter` (delayed callback ⇒ job, never inline work).

### A18. Replay subtask no-feature fallback: 2 peers
- Quaxar: `REPLAY_MAX_NO_FEATURE_PEER_COUNT: u32 = 2` — `skip_list_acquire.rs:11`; each non-feature peer increments a counter that latches `fall_back = true` at `>= 2` (`skip_list_acquire.rs:189-192`; delta `delta_acquire.rs:261-266`). On `fall_back` the subtask falls back to a full `Generic` ledger acquire (`skip_list_acquire.rs:198-200`; `delta_acquire.rs:271-273`).
- rippled: `kMaxNoFeaturePeerCount = 2` — `LedgerReplayer.h:44`. ✅.

### A19. Replay subtask timeout cap: 10
- Quaxar: `REPLAY_SUB_TASK_MAX_TIMEOUTS: i32 = 10` — `skip_list_acquire.rs:10`; a no-progress `invoke_on_timer` increments and fails the subtask once `timeouts > 10` (`skip_list_acquire.rs:106-111`; delta `delta_acquire.rs:128-134`).
- rippled: `kSubTaskTimeoutCount`/subtask timeout accounting. ✅ value parity on the count.

### A20. Replay task max timeouts: max(10, 2 × total_ledgers)
- Quaxar: `REPLAY_TASK_MAX_TIMEOUTS_MULTIPLIER: u32 = 2` and `REPLAY_TASK_MAX_TIMEOUTS_MINIMUM: u32 = 10` — `replay_task.rs:9-10`; `max_timeouts = max(10, total_ledgers * 2)` (`replay_task.rs:106-110`); `invoke_on_timer` fails once `timeouts > max_timeouts` (`replay_task.rs:199-204`). `try_advance` asserts consecutive parent/delta sequence (`replay_task.rs:252-256`) and advances `delta_to_build` per successful build, completing only when all deltas build (`replay_task.rs:242-269`).

### A21. Replayer caps: max tasks=10, max task size=256, merge dedup
- Quaxar: `REPLAY_MAX_TASKS: usize = 10`, `REPLAY_MAX_TASK_SIZE: u32 = 256` — `replayer.rs:15-16`; admitted at `replayer.rs:44,48-50`. A new task is rejected if it `can_merge_into` an existing task (`replayer.rs:53-57`, `can_merge_into` at `replay_task.rs:63-84`). Skip lists and deltas are shared via weak maps (`replayer.rs:59-71,118-131`).

### A22. Tx-set acquisition: start=2 peers, normal=4 / max=20 timeouts
- Quaxar: `START_PEERS: usize = 2` (`inbound_transactions.rs:11`), `TX_ACQUIRE_NORM_TIMEOUTS: i32 = 4`, `TX_ACQUIRE_MAX_TIMEOUTS: i32 = 20` (`transaction_acquire.rs:16-17`). `still_need()` clamps timeouts to 4 and clears failure (`transaction_acquire.rs:101-107`); `invoke_on_timer` fails only when `timeouts > 20` (`transaction_acquire.rs:113-124`). Root broadcast to all peers on the root request (`transaction_acquire.rs:225-232`).

### A23. Tx-set cache semantics: zero-set slot, keep-3-rounds, bounded completion replay
- Quaxar: a `Uint256::zero()` slot holds the empty tx set (`inbound_transactions.rs:44-49`); `SET_KEEP_ROUNDS: u32 = 3` (`inbound_transactions.rs:12`). `give_set` notifies only on a genuinely new set and, if the bounded `map_complete` channel is full, sets `completion_pending` for strand-side deterministic drain (`inbound_transactions.rs:134-162`, `take_pending_map_completions` at 181-197).

### A24. TimeoutCounter interval range assertion
- Quaxar: `TimeoutCounter::new` asserts `10 ms < interval < 30 s` (`timeout_counter.rs:103-106`).

### A25. Malformed-packet charging via FEE_MALFORMED_REQUEST
- Quaxar: `charge_malformed_packet` maps empty-header / no-nodes / invalid-header / bad-node errors onto `resource::FEE_MALFORMED_REQUEST` (`acquisition.rs:1358-1379`, charge at 1375-1378).
- rippled: `kFeeMalformedRequest` in `InboundLedger.cpp`. ✅ for the malformed category (see B1 for the missing invalid-data category).

---

# (B) MISSING / DIVERGENT ITEMS

### B1. MISSING — invalid-data charge category (FEE_INVALID_DATA)
- **Where:** `acquisition.rs:1358-1379` is the only inbound-ledger charge site and it charges solely `FEE_MALFORMED_REQUEST` (`acquisition.rs:1376`). A codebase-wide search finds **no** `FEE_INVALID_DATA` charge in any acquisition path.
- rippled analog: rippled charges `kFeeInvalidData` for an invalid AS root, invalid TX root, and invalid node data inside `receiveNode`/`takeAsRootNode`/`takeTxRootNode` (`InboundLedger.cpp:882-907,1105-1136`), distinct from `kFeeMalformedRequest`; the catch around node processing deliberately avoids charging when the failure is not the peer's fault (`InboundLedger.cpp:911-918`).
- **Suggested descriptive text:** "The received packet is well-formed at the container level but the root or node data fails validation (invalid account-state root, invalid transaction root, or invalid node object). Charge `resource::FEE_INVALID_DATA` as a distinct penalty from a malformed (undecodable) packet; do not charge when the failure is not attributable to the source peer." — This category is absent from quaxar's acquisition.rs and must be added in the root-node and node-receive paths (`take_as_root_node`, `take_tx_root_node`, `receive_node`).

### B2. DIVERGENT — replay-subtask timer cadence (250 ms → 1 s fallback) is not represented
- **Where:** `skip_list_acquire.rs`, `delta_acquire.rs`, `replay_task.rs`, `replayer.rs` define **no** timer-interval constant. `TimeoutCounter::new` requires an externally supplied `timer_interval` (`timeout_counter.rs:95-106`), and every subtask exposes only `invoke_on_timer(...)` (`skip_list_acquire.rs:94-117`, `delta_acquire.rs:118-134`, `replay_task.rs:183-210`) meaning cadence is delegated to the caller.
- rippled analog: replay subtasks use `kSubTaskTimeout = 250 ms` by default, bumping to `kSubTaskFallbackTimeout = 1000 ms` once `kMaxNoFeaturePeerCount = 2` no-feature peers are seen (`LedgerReplayer.h:38,46,44`; `SkipListAcquire.cpp:99-104`; `LedgerDeltaAcquire.cpp:104-109`); the parent task uses 500 ms (`LedgerReplayer.h:29`). Quaxar's `REPLAY_MAX_NO_FEATURE_PEER_COUNT` latch (skip_list_acquire.rs:190-191 / delta_acquire.rs:263-264) triggers a *full re-acquire fallback* but does **not** bump a subtask timer interval.
- **Suggested descriptive text:** "Replay subtasks must run on a 250 ms timer, escalating to a 1000 ms fallback timer after two no-feature peers are observed; the parent replay task uses a 500 ms timer. Quaxar models the timeout *counts* (10) and the fallback *re-acquire* but the Rust tree contains no 250/500/1000 ms cadence constants — the driver that invokes `invoke_on_timer` must be documented to supply this cadence, or the constants added."

### B3. DIVERGENT — ledger replay is not negotiated on the wire
- **Where:** the outbound/inbound handshake advertises `ledger_replay = false // not implemented` (`overlay_impl.rs:1604-1605`, response built at `overlay_impl.rs:1599-1608`). Although `ProtocolFeature::LedgerReplay` exists (`overlay_impl.rs:2587`) and the `LedgerReplayer`/`SkipListAcquire`/`LedgerDeltaAcquire` owner ports (B2 files) are implemented, peers will never advertise replay, so no replay peer is ever recruited and the `fall_back → Generic full re-acquire` path is the only operational outcome.
- **Suggested descriptive text:** "The overlay must advertise `ledger_replay = true` (matching rippled's LedgerReplay feature) before `LedgerReplayer`-driven skip-list/delta replay can be exercised on a live network. Without this, the replay subtasks always fall back to generic full-ledger acquisition."

### B4. DIVERGENT — registry is structurally one global acquisition per hash (no per-reason set)
- **Where:** `registry.rs:3-5` documents a single `HashMap<Uint256, Entry>` under one mutex. `acquire` (`registry.rs:659-807`) reuses the existing entry (updating sequence when it was 0, `registry.rs:703-706`) and never starts a second acquisition for the same hash even if the reason differs. rippled similarly coalesces to one `InboundLedger` (see `rippled-acquisition-findings.md` A-series), so this is a faithful design choice rather than a numeric gap; it is recorded here so consensus-driven re-acquires of a still-running hash are understood to be non-queued. (`on_failed` at `registry.rs:1107-1113` stops the entry's state but leaves the entry for the sweep window.)

### B5. MISSING (doc) — sweep/failure counts not exposed in `fetch_info`'s official shape
- **Where:** `fetch_info` exposes only the bounded per-acquisition set capped at `FETCH_INFO_MAX_ACQUISITIONS = 16` (`registry.rs:44`) plus aggregate `recent_failures`/`active`/`completed`/`failed` counters (`registry.rs:1031-1069`). There is no per-reason breakdown (consensus/generic/history) of failures in the summarized JSON.
- **Suggested descriptive text:** "Consider surfacing failure counts split by `AcquireReason` (consensus/generic/history) in `fetch_info` so operators can attribute recovery load." — informational only.

---

# (C) INACCURATE / STALE CITATIONS WITH CORRECTIONS

### C1. "The one-shot by-hash latch is a principal semantic gap missing in quaxar." — INACCURATE
- **Claim:** `rippled-acquisition-findings.md` cross-cutting section lists "the one-shot by-hash latch" among quaxar's principal gaps.
- **Correction:** quaxar implements the latch. On a no-progress timeout `process_timeout_job` sets `by_hash = true` (`acquisition.rs:1415`); the aggressive by-hash block emits exactly one request and then clears `by_hash = false` (`ledger_fetcher.rs:2282-2291`, `2719-2727`). A subsequent progress-making turn does not re-emit. This is precisely a one-shot latch.

### C2. "The per-type cap of 4 in the aggressive probe is missing." — INACCURATE
- **Claim:** `rippled-acquisition-findings.md` cross-cutting section lists "the per-type cap of 4 in the aggressive probe" as a quaxar gap.
- **Correction:** the by-hash probe's needed hashes are produced by `get_needed_hashes_with_family`, which caps state hashes at `INBOUND_LEDGER_MAX_NEEDED_STATE_HASHES = 4` (`ledger_fetcher.rs:41`, applied at 3569-3570) and tx hashes at `MAX_NEEDED_TX_HASHES = 4` (`ledger_fetcher.rs:42`, applied at 3579-3580). The probe is therefore naturally bounded at 4 per type, matching rippled.

### C3. "Replay timing (250 ms/1 s subtasks, 500 ms task) is at parity with the C++ source." — MISLEADING for quaxar source
- **Claim:** `rippled-acquisition-findings.md` cross-cutting section asserts these cadences "match the C++ source."
- **Correction:** the *rippled* values match C++; they are **not** present in the quaxar Rust acquisition subtree. `skip_list_acquire.rs`, `delta_acquire.rs`, `replay_task.rs`, and `replayer.rs` contain no 250/500/1000 ms constants (see B2), and `overlay_impl.rs:1604-1605` disables replay on the wire (see B3). The claim is accurate only about rippled; it overstates quaxar parity.

### C4. Quaxar file:line citations inside `rippled-acquisition-findings.md` are accurate — no stale cites
- Verified against the current tree: `acquisition.rs:34` (A1), `acquisition.rs:32-33` + `1156-1160` (A2), `ledger_fetcher.rs:59-63` (A3/A4), `registry.rs:35` (A9/C11), `registry.rs:40` (A10), `registry.rs:252` (C12). All resolve to the correct identifiers and values. No correction required.

### C5. `QUAXAR_COMPLETE_FLOW.md` marks "Inbound ledger peers start 5; add 3" as "⚠️ Explicit Quaxar setting" — the warning is unwarranted
- **Claim:** `docs/QUAXAR_COMPLETE_FLOW.md:103` flags the 5/3 peer step with an explicit-quaxar divergence marker.
- **Correction:** `PEER_COUNT_START = 5` / `PEER_COUNT_ADD = 3` (`acquisition.rs:32-33`) exactly match rippled `kPeerCountStart`/`kPeerCountAdd` (`InboundLedger.cpp:59-60`). The ⚠️ is inaccurate; this is parity, not a divergent setting. The other acquisition-table rows (`QUAXAR_COMPLETE_FLOW.md:101-102,104`) were verified accurate against source (`acquisition.rs:34,597-614`; `ledger_fetcher.rs:59-63,3323-3345`; `inbound_transactions.rs:11-12`; `transaction_acquire.rs:13-18`).

### C6. Source comment at `ledger_fetcher.rs:4` says the module ports logic literally "from `xrpld/app/ledger/detail/the reference source`" — placeholder / stale
- **Claim:** the module doc header references a file literally named `the reference source`.
- **Correction:** rewrite the cross-reference to the actual rippled file (e.g. `src/xrpld/app/ledger/detail/InboundLedger.cpp`) to keep the provenance citation meaningful.

---

## Cross-cutting observation

The quaxar acquisition subsystem tracks the rippled reference on every *numeric* parameter with high fidelity: 3 s main acquire timeout, 5/3 peer step, 12/128/256 node caps, 6 retries / 4 aggressive threshold, 6 sampled useful peers, 128 packet-step node cap, 5-job admission cap, 64 workers, 5-min failure cooldown, 60-s sweep, 10 replay-subtask timeouts, `max(10, 2×N)` task timeouts, 2-peer no-feature fallback, and 2/4/20 tx-set peers/timeouts. The genuine gaps are semantic and architectural — the missing `FEE_INVALID_DATA` charge split (B1), the absent 250/1000 ms replay-subtask cadence (B2), and the disabled replay wire feature (B3). Two of the rippled report's alleged quaxar gaps (one-shot by-hash latch, per-type cap of 4) are in fact implemented and should be removed from the gap list (C1, C2).

---
*End of Quaxar acquisition findings report.*
