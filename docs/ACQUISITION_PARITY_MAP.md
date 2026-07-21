# Ledger Acquisition Parity Map

This document records the checked lifecycle correspondence between Quaxar and rippled. “Equivalent” means the observable acquisition control flow is matched; Rust uses a shared delayed-callback service instead of Boost.Asio.

## Lifecycle matrix

| rippled | Quaxar | Verified behavior |
| --- | --- | --- |
| `InboundLedgersImp::acquire` | `InboundLedgers::acquire` | One entry per hash; no acquisition-cap rejection; existing entries are touched; failures use a five-minute cooldown. A new entry is inserted before `start()`, so responses can be routed immediately. |
| `InboundLedger::init` | `AcquisitionState::start` / `process_init` | `checkLocal` first. If incomplete, `addPeers`, then queue the immediate timeout job. A locally complete ledger finalizes without peer requests. |
| `PeerSetImpl::addPeers` | `SimplePeerSet::add_peers` + `add_peers` | Start with five peers when empty, otherwise add three. New peers are selected using `has_ledger(hash, seq)`. `Added` triggers occur per new peer except for History. |
| `TimeoutCounter::queueJob` | `WorkerPool::try_submit_timeout` | Timer work uses the ledger-data queue count limit of five. A saturated queue re-arms the acquisition timer rather than dropping the timeout. |
| `TimeoutCounter::setTimer` | `AcquisitionState::arm_timer` / `TimerService` | Every nonterminal timeout job arms only its own next three-second callback. The timer thread only enqueues work; it does not run acquisition logic. |
| `TimeoutCounter::invokeOnTimer` | `InboundLedgerLocal::timeout_expired` | Clears recent-node deduplication, consumes progress when present, counts no-progress timeouts, and fails only after timeout seven (`timeouts > 6`). |
| `InboundLedger::onTimer` | `process_timeout_job` | On no progress: `checkLocal`, set by-hash mode, and preserve reference request ordering: Generic/Consensus triggers Timeout then adds peers; History adds peers then triggers Timeout. |
| `InboundLedgersImp::gotLedgerData` + `InboundLedger::gotData` | `route_response_with_seq` + `submit_data_job` | Packet routing validates sequence, buffers data, and coalesces one `runData` job per acquisition. New data arriving during a drain queues exactly one subsequent job. |
| `InboundLedger::runData` | `process_data_job` + `run_data_with_family_and_config_and_refill` | Drains all received packets, tracks useful peers, samples the useful set, and immediately sends `Reply` / high-latency `Reply` triggers. No reply-time `recentNodes` clearing or extra fan-out exists. |
| `InboundLedgersImp::gotFetchPack` | `notify_fetch_pack_ready` | Each active acquisition runs `checkLocal`; it does not send an unrelated request. |
| `InboundLedgersImp::sweep` | `InboundLedgers::sweep` | Removes entries idle for more than one minute. Only failed acquisitions enter `recent_failures`; successful or abandoned entries do not become failures merely because they were swept. |
| `InboundLedger::done` | `finalize_acquisition` | Completed ledgers are finalized, made immutable/full, given a NuDB node fetcher, and sent to the application completion channel. Failed acquisitions stop timer and data work. |

## Persistence and SHAMap path

| rippled | Quaxar | Verified behavior |
| --- | --- | --- |
| `SHAMap::addRootNode` / `addKnownNode` | `SyncTree::add_root_node_with_family` / `add_known_node_with_family` | Accepted nodes invoke the sync filter before the call returns. |
| `AccountStateSF::gotNode`, `TransactionStateSF::gotNode` | `AccountStateSF::got_node`, `TransactionStateSF::got_node` | The filter stores accepted state/transaction nodes through `WorkerStore`. |
| `DatabaseNodeImp::store` | `WorkerStore::store_object` → `SHAMapStoreNodeStore::store` | Writes are synchronous. The removed background channel, pending-write mirror, flush protocol, and eviction predicate cannot race memory release with persistence. |
| `NuDBBackend::store/fetch` | `NuDbBackend::store/fetch` | Not changed by this lifecycle rewrite. No unverified `RwLock` conversion is claimed or applied. |

## Deliberately removed non-reference behavior

- Registry-wide one-second submission of every active acquisition.
- Direct header-send bypass that skipped `trigger` and `tryDB`.
- Reply-time `recent_nodes` clearing.
- Extra state-peer fan-out and tick self-resubmission.
- Cold-start 1024-node scan expansion and environment-controlled timeout limit.
- Run-data semaphore, progressive deep-tree spill, and post-acquisition map release.
- Background node-store writer, pending-write reads, writer flush messages, and bootstrap writer setup.

## Relevant files

**Quaxar:** `xrpld/app/src/ledger/inbound_ledgers/{acquisition,registry,worker_pool}.rs`; `xrpld/app/src/bootstrap/bootstrap.rs`; `xrpld/ledger/src/acquisition/ledger_fetcher.rs`; `xrpld/ledger/src/domain/{account_state_sf,transaction_state_sf}.rs`; `xrpl/shamap/src/{owners/sync,operations/fetch,owners/family}.rs`; `xrpld/nodestore/src/{database_runtime/database_node_imp,backends/nudb_backend}.rs`; `xrpld/overlay/src/{peer/peer_set,transport/{router,session,inbound},runtime/overlay_impl}.rs`; `xrpld/app/src/{consensus/rcl_consensus,network/network_ops_strand,state/application_root}.rs`.

**rippled:** `src/xrpld/app/ledger/{detail/TimeoutCounter,detail/InboundLedger,detail/InboundLedgers,LedgerMaster,AccountStateSF,TransactionStateSF}`; `src/xrpld/overlay/{PeerSet,detail/PeerImp}`; `src/libxrpl/{shamap/SHAMapSync,nodestore/DatabaseNodeImp,nodestore/backend/NuDBFactory}`; `src/xrpld/app/misc/NetworkOPs.cpp`.

## Validation

All of the following completed successfully during this implementation:

- `cargo check -p app`
- `cargo test -p ledger inbound_timeout_counter_fails_only_after_six_no_progress_retries`
- `cargo test -p ledger acquisition::inbound_dispatch` (4 passed)
- `cargo build --release -p xrpld-main`

The builds retain pre-existing unused-code warnings, but no errors.
