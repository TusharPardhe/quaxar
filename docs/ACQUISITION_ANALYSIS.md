# Deep Analysis: rippled vs quaxar InboundLedger Acquisition

## A. Threading Model

### Rippled
- **Timer thread** (boost::asio): Fires every 3s, queues a `JtLedgerData` job via `queueJob()`.
- **Job queue workers** (thread pool): Execute `invokeOnTimer()` → `onTimer()` → `trigger()`.
- **Data processing**: `gotLedgerData()` receives data on I/O thread, stashes it, dispatches a `JtLedgerData` job that calls `runData()`.
- **Concurrency**: Multiple threads touch one InboundLedger — timer jobs, data jobs, and the `trigger()` calls from `runData()`. Protected by `mtx_` (recursive mutex) + `receivedDataLock_` (separate lock for data queue).
- **Job limit**: `jobLimit = 5` prevents flooding the job queue. If 5+ JtLedgerData jobs exist, timer defers by re-arming.
- **Key insight**: Data arrival → job dispatch → process data → trigger peers — all happens asynchronously. A single InboundLedger can have its timer fire while data processing is queued.

### Quaxar
- **Single worker thread** (`run_acquisition_worker`): One dedicated OS thread per active acquisition.
- **Receiver thread** (`xrpld-sacq-recv`): Bridges mpsc channel → `shared_queue` + condvar notification.
- **Main loop**: Drain messages → check conditions → process data → scan missing → send requests → check timer.
- **Serialization**: ALL work for one ledger happens sequentially on one thread. Processing data BLOCKS sending new requests.
- **RunDataLimiter**: Limits concurrent `run_data` calls across ALL workers (not per-worker).

### Parallelism Gap
1. Rippled processes data on thread A while timer fires trigger on thread B. Quaxar serializes these.
2. Rippled can process multiple data packets via different job invocations. Quaxar batches all in one `run_data` call.
3. In rippled, `runData()` finishing immediately triggers up to 6 peers — this happens on the job thread without waiting. In quaxar, after `run_data`, the scan and send happen on the same thread before the next loop iteration.

---

## B. Request Dispatch Pattern

### Rippled's `trigger(peer, reason)`
1. If `!haveHeader_`: sends `liBASE` request → `peerSet_->sendRequest(tmGL, peer)` (to specific peer or ALL peers).
2. Sets `queryDepth`:
   - `0` for Blind/Timeout/Added
   - `1` for Reply (normal latency)
   - `2` for Reply (high latency peer)
3. For state map: calls `getMissingNodes(kMissingNodesFind=256, &filter)`, releases lock during scan, then calls `filterNodes()`.
4. `filterNodes()`:
   - Partitions nodes: fresh (not recently requested) first, duplicates last.
   - If ALL duplicates and not Timeout → clears and returns (no request sent).
   - Limits: `kReqNodesReply=128` for Reply, `kReqNodes=12` for all others.
   - Inserts sent node hashes into `recentNodes_` set.
5. Sends the request to the specific peer (or all peers if peer=nullptr).
6. **Aggressive mode** (timeouts > 4): Sends `TMGetObjectByHash` to ALL peers with specific needed hashes.

### Quaxar's dispatch
1. `trigger_with_family()`: Used only for initial header request and on-timer fallback.
2. Primary path: `scan_missing_nodes_for_fanout()` → chunks nodes into `REQ_NODES_REPLY=128` sized groups → round-robins across peers.
3. After `run_data`: scans missing nodes, fans out ALL missing (up to `MISSING_NODES_FIND_COLD_START=1024`) across all available peers.
4. No equivalent to rippled's `kReqNodes=12` blind limit — always sends full 128-node chunks.
5. **No aggressive mode**: No `TMGetObjectByHash` fallback after repeated timeouts.
6. **No queryDepth differentiation**: Most paths pass `query_depth=1` regardless of trigger reason.

### Critical Differences
- Rippled: conservative (12 nodes blind, 128 on reply) → avoids flooding peers.
- Quaxar: aggressive (all missing nodes chunked into 128) → can overwhelm peers but maximizes parallelism.
- Rippled's per-peer trigger means each peer gets exactly one request per cycle.
- Quaxar's fan-out means a peer can receive multiple 128-node chunks in one cycle.

---

## C. Data Processing Flow

### Rippled
1. Overlay thread receives `TMLedgerData` → calls `InboundLedgers::gotLedgerData()`.
2. `gotLedgerData()` finds the InboundLedger, calls `ledger->gotData(peer, packet)`.
3. `gotData()` stashes in `receivedData_` (under `receivedDataLock_`). If `!receiveDispatched_`, sets flag and returns true.
4. Caller dispatches `JtLedgerData` job → `runData()`.
5. `runData()` loops: swap `receivedData_`, process each packet via `processData()`, track peer scores.
6. After all data processed: prune low-scoring peers, sample up to 6 best peers, call `trigger(peer, Reply)` for each.
7. **Key**: `receiveDispatched_` ensures only ONE `runData` job at a time. New data arriving while runData is active just stashes — runData loops until empty.

### Quaxar
1. Overlay I/O thread → `route_response()` → sends `AcqMsg::LedgerData` to mpsc channel.
2. Receiver thread drains channel → pushes to `shared_queue` → notifies condvar.
3. Worker main loop: drains queue, calls `inbound.got_data()` to stash, then processes via `run_data_with_family_and_config_and_refill()`.
4. `run_data` processes packets, has a `refill` closure that drains more from `shared_queue` mid-processing.
5. After processing: `scan_missing_nodes_for_fanout()` → send requests.

### Serialization Bottleneck
- The `pending_writes` mutex is locked on EVERY `store_object()` call AND every `fetch_node_object()` call (for the pending check).
- During a large state tree download: thousands of nodes arrive per second, each requiring a lock/unlock cycle on `pending_writes`.
- The `RunDataLimiter` further serializes processing across workers — only N workers can process data simultaneously.
- The single-thread model means the worker cannot send new requests while processing a batch.

---

## D. Timer and Retry Logic

### Rippled
1. `TimeoutCounter` arms a boost::asio timer for 3000ms.
2. On fire: `invokeOnTimer()` checks `progress_` flag.
3. If progress: clears flag, calls `onTimer(true, sl)` (which does nothing for InboundLedger when progress=true), re-arms timer.
4. If no progress: increments `timeouts_`, calls `onTimer(false, sl)`.
5. `onTimer(false)`:
   - Clears `recentNodes_` (allows re-requesting previously sent nodes).
   - If `timeouts_ > 6`: mark failed, done().
   - Otherwise: `checkLocal()`, set `byHash_=true`, trigger(nullptr, Timeout) + addPeers().
6. Timer always re-arms after `onTimer` unless done.

### Quaxar
1. Main loop checks `last_timer.elapsed() >= Duration::from_secs(3)`.
2. If progress (`inbound.progress()` returns true): clears progress + recent_nodes, resets `last_timer`.
3. If no progress:
   - Calls `on_timer_with_family()` (which sends requests via `send_fn`).
   - Adds peers via `peer_set.add_peers()`.
   - Scans and fans out to newly added peers.
4. **But**: the main loop's condvar timeout varies from 1ms to 1s (line: `Duration::from_secs(1).saturating_sub(...).min(Duration::from_millis(50))`).
5. After processing data: `last_timer` is NOT reset. Only reset when timer fires OR on first_add_peers.

### Issues
- Timer precision: rippled's boost::asio timer is exact (3000ms ± scheduling jitter). Quaxar's is approximate (checked every loop iteration, which fires at varying intervals).
- **When progress=true in rippled**: timer still fires every 3s, just doesn't increment timeouts. Quaxar resets `last_timer` on progress=true path in the 3s check, but the timer check only happens at the END of the main loop — if data processing takes 2s, the effective timer period is 5s.
- **Missing**: Quaxar doesn't call `checkLocal()` on timeout (rippled does to see if data arrived via other paths like fetch packs).

---

## E. PeerSet Management

### Rippled's PeerSetImpl
- `addPeers(limit, hasItem, onPeerAdded)`:
  - Iterates ALL overlay peers.
  - Scores each: `peer->getScore(hasItem(peer))` — higher score for peers that have the ledger.
  - Sorts by score descending.
  - Picks top `limit` peers not already in the set.
  - Calls `onPeerAdded` for each (which triggers the peer).
- `sendRequest(message, peer)`:
  - If peer specified: sends to that one peer.
  - If peer=nullptr: sends to ALL peers in the set.
- Peer set is **append-only** — peers are never removed (even if disconnected).
- `getPeerCount()` in InboundLedger filters by `findPeerByShortID != nullptr` to count only alive peers.

### Quaxar's SimplePeerSet
- `refresh_peers()`: REPLACES the peer list entirely (called when `AcqMsg::Peers` arrives).
- `add_peers(limit, has_item, on_added)`: Similar scoring logic.
- `send_request(msg, peer)`: Sends to specific peer or all.
- `get_peers()`: Returns all tracked peers.
- **No disconnect detection**: Peers stay until next `refresh_peers` call.

### Gaps
1. **No scoring persistence**: Quaxar's `refresh_peers` replaces the entire set — existing good peers are lost.
2. **No alive check**: rippled's `getPeerCount()` verifies peers are still connected. Quaxar sends to potentially-disconnected peers.
3. **No per-peer trigger on add**: In rippled, `onPeerAdded` calls `trigger(peer, Added)` — each new peer gets its own individual request immediately. Quaxar does a single scan and fans out.

---

## F. Cold Start Behavior

### Rippled
- No explicit cold-start gate. Multiple InboundLedgers can run concurrently.
- `InboundLedgersImp` has no hard limit on concurrent acquisitions (bounded only by sweep).
- On first acquisition from empty DB: `tryDB()` finds nothing, sets up maps, `addPeers()` + `queueJob()` → first trigger sends header request.
- Natural pacing: each acquisition only requests from its own peer set, timer-driven.

### Quaxar
- **Explicit cold-start gate**: `has_validated_ledger` AtomicBool.
- Before first validated ledger: ONLY ONE acquisition at a time. All others are refused.
- `MAX_CONCURRENT_ACQUISITIONS = 8` after first completion.
- Eviction policy: when at capacity (post-first-complete), evicts lowest-seq worker.
- Cold start sends peers to existing worker to keep it supplied.

### Assessment
- Quaxar's gate is a defensive measure against OOM (observed 34GB from 13 concurrent full-tree downloads).
- Correct for the current architecture where each worker holds its tree in memory.
- BUT: means the node cannot chase multiple consensus targets during initial sync — it's locked to one hash until complete.
- Rippled doesn't have this problem because its SHAMap nodes are stored immediately in NuDB (not held in worker memory).

---

## G. All Potential Issues (PESSIMISTIC)

### Correctness Bugs
1. **Timer drift**: Main loop condvar timeout is min(50ms, remaining) — actual timer period can be 3s to 4s+ if processing is slow.
2. **Progress flag race**: `inbound.progress()` is checked in the timer section, but `just_processed` (from run_data) happens earlier in the loop. If run_data sets progress but the 3s check happens in the same iteration, progress is immediately consumed without the timer actually resetting properly.
3. **Completion detection**: The check `have_header && have_state && have_transactions` is done at multiple points in the loop. If state completes during run_data but the check happens before the next loop iteration's early-exit, there's a 1-loop-iteration delay.
4. **Stale peer references**: `AcqMsg::Peers` replaces the set, but in-flight requests to old peers are never cancelled. If a peer disconnects and the node doesn't get a Peers update, requests go to dead connections.

### Performance Issues
5. **Single-thread serialization**: Processing 1000 node packets takes time T. During T, no new requests are sent. Rippled would send requests on a different thread.
6. **pending_writes mutex contention**: Every `store_object` and every `fetch_node_object` locks this HashMap. With 10+ concurrent workers, this is a global bottleneck.
7. **No 12-node blind limit**: When quaxar triggers on timeout, it sends ALL missing nodes (chunked 128). Rippled sends only 12. This floods peers on timeout.
8. **TreeNodeCache(100M) per worker**: Each acquisition worker creates a 100M-entry cache. With 8 concurrent workers, that's potentially 800M cache entries consuming significant RAM.
9. **SHAMapFamily recreated every loop tick**: `SHAMapFamily::new()` is called every iteration of the main loop. While lightweight, it creates a new `WorkerNodeFetcher` struct each time (with Arc clones).
10. **No batched NuDB writes**: Each node is sent individually to the writer thread. NuDB performs best with batch inserts.

### Missing Features
11. **No TMGetObjectByHash aggressive mode**: After 4+ timeouts, rippled falls back to direct hash requests. Quaxar never does this.
12. **No stale data salvage**: Rippled saves AS node data from abandoned acquisitions to the fetch pack. Quaxar drops everything.
13. **No fetch pack integration on timer**: Rippled calls `checkLocal()` on timeout to check if fetch packs populated data. Quaxar only checks on explicit `FetchPackReady` message.
14. **No peer scoring**: Peers that return more data should be preferred. Rippled's `runData` prunes low-scoring peers and samples best ones.
15. **No queryDepth=0 for blind requests**: Quaxar always sends queryDepth=1, asking peers to include child data. On blind requests (timeout/added), this wastes peer CPU.
16. **No queryDepth=2 for high-latency peers**: Missing optimization for high-latency connections.

### Race Conditions
17. **Registry race on stop**: If `AcqMsg::Stop` is sent but the worker hasn't processed it yet, new data can still arrive and be processed.
18. **Sweep vs. progress**: Sweep thread checks `STUCK_TIMEOUT=30s`. If a worker is making progress but hasn't touched the entry (no mechanism to touch from worker), sweep kills it.
19. **Multiple scan_missing_nodes_for_fanout calls**: The same scan can happen in both the run_data path and the timer path in the same loop iteration if timing aligns.

### Memory Concerns
20. **pending_writes unbounded**: If the NuDB writer is slow, pending_writes grows without bound. Each entry holds full node data (hundreds of bytes to KB).
21. **shared_queue unbounded**: If the worker is slow processing, the shared_queue can grow as peers flood responses.
22. **acquisition_cache 100M entries**: No size-based eviction during acquisition — just a TTL of 3600s. A full state tree scan can fill this.
23. **Worker thread leak**: `_acq_handle` is detached (not joined). If the worker panics, the thread leaks and the entry stays in `inner.entries` until sweep.

### Observed Behavior Causes (slow sync, timeouts)
24. **Single-thread bottleneck**: The biggest cause. While processing a batch of 50 packets (each with 128 nodes = 6400 node insertions), no new requests go out. This creates a stop-start pattern.
25. **Timer masked by processing**: If processing takes >3s, the timer never fires because `last_timer` is checked after processing completes.
26. **Over-requesting**: Sending 1024 missing nodes across peers generates a flood of responses that then queue up, creating another long processing cycle.
27. **STUCK_TIMEOUT=30s too aggressive**: A cold-start full state tree (33GB) takes minutes. If the worker doesn't receive data for 30s (e.g., peers are rate-limiting), sweep kills it.

---

## H. Implementation Plan

### Phase 1: Fix Timer Precision (Risk: Low, Impact: Medium)
**What**: Decouple timer from main processing loop. Use a separate timer mechanism (e.g., `Instant` checked at loop TOP, not bottom, with forced yield after 3s).
**Why**: Current timer can drift 3-6s+ during heavy processing.
**How**: Check timer FIRST in each loop iteration. If due, handle timer logic BEFORE processing data.
**Expected Impact**: More consistent retry behavior, fewer false stuck-timeouts.

### Phase 2: Implement queryDepth Parity (Risk: Low, Impact: Low)
**What**: Set queryDepth=0 for Timeout/Added/Blind triggers, 1 for Reply, 2 for high-latency.
**Why**: Reduces peer CPU load on blind requests, increases data depth from high-latency peers.
**How**: Pass trigger reason through to `make_get_ledger_with_node_ids`, compute depth.
**Expected Impact**: ~10% reduction in peer load, slightly faster response from high-latency peers.

### Phase 3: Implement filterNodes Parity (Risk: Low, Impact: Medium)
**What**: Add kReqNodes=12 limit for Timeout/Added/Blind triggers.
**Why**: Current approach sends 128-node chunks on timeout, flooding peers.
**How**: In `scan_missing_nodes_for_fanout`, accept a `limit` parameter. Use 12 for non-reply triggers.
**Expected Impact**: Reduces peer flooding on timeouts. Peers respond faster to smaller requests.

### Phase 4: Implement Aggressive Mode (TMGetObjectByHash) (Risk: Medium, Impact: Medium)
**What**: After 4+ timeouts with no progress, fall back to direct hash-based requests to ALL peers.
**Why**: The normal TMGetLedger path may fail if peers don't have the requested SHAMap subtree view. Direct hash requests bypass this.
**How**: Port the `byHash_` + `getNeededHashes()` + `TMGetObjectByHash` path from rippled.
**Expected Impact**: Recovery from stuck acquisitions without hitting the 6-timeout failure limit.

### Phase 5: Add Peer Scoring (Risk: Medium, Impact: High)
**What**: Track per-peer data usefulness during runData. Prefer high-scoring peers for subsequent requests.
**Why**: Some peers return more useful data (closer, faster, have the data). Preferring them speeds acquisition.
**How**: Port `PeerDataCounts` logic — track good-node counts per peer, prune low scorers, sample top 6.
**Expected Impact**: 20-40% faster acquisition by focusing on responsive peers.

### Phase 6: Async Request Dispatch (Risk: High, Impact: High)
**What**: Decouple data processing from request sending. After processing each batch, immediately dispatch requests before processing the next batch.
**Why**: Current serial model means 2-5s processing gaps where no requests are outstanding.
**How**: After each `processData` call within run_data, check if enough new nodes were learned to justify an immediate request dispatch (threshold: 64+ new nodes). Send requests inline before processing next packet.
**Expected Impact**: Eliminates stop-start pattern. Continuous pipeline of requests/responses.

### Phase 7: Reduce pending_writes Contention (Risk: Medium, Impact: Medium)
**What**: Replace `Mutex<HashMap>` with a lock-free structure or batch-insert pattern.
**Why**: Every store and every fetch locks this map. With high throughput, this is a bottleneck.
**How**: Option A: Use `DashMap` (concurrent HashMap). Option B: Thread-local pending buffer, flushed periodically. Option C: Eliminate pending_writes entirely by using synchronous NuDB writes (NuDB is already crash-safe).
**Expected Impact**: 15-30% throughput improvement during heavy acquisition.

### Phase 8: Implement Stale Data Salvage (Risk: Low, Impact: Low)
**What**: When an acquisition is stopped/evicted, save unprocessed AS_NODE data to fetch pack.
**Why**: This data may be useful for other acquisitions (state trees overlap significantly between adjacent ledgers).
**How**: On worker stop, drain remaining queued data, extract AS_NODE packets, add to fetch pack cache.
**Expected Impact**: Faster subsequent acquisitions for nearby ledgers.

### Phase 9: Increase STUCK_TIMEOUT (Risk: Low, Impact: Medium)
**What**: Increase STUCK_TIMEOUT from 30s to 120s (matching rippled's 6×3s=18s minimum + generous margin).
**Why**: 30s is too aggressive for cold-start downloads that can have legitimate 30s+ gaps.
**How**: Change constant. Add progress-based touch from within the worker (send a heartbeat message back to SharedInboundLedgers).
**Expected Impact**: Prevents premature killing of slow but progressing acquisitions.

### Phase 10: Worker Heartbeat (Risk: Low, Impact: Medium)
**What**: Worker periodically sends progress updates back to SharedInboundLedgers to prevent sweep.
**Why**: Currently only `last_touched` is updated on acquire() calls. Worker progress is invisible to sweep.
**How**: Add `AcqResult::Heartbeat` message. Worker sends every 5s. SharedInboundLedgers updates `last_touched`.
**Expected Impact**: Prevents false stuck-timeout kills during active acquisition.
