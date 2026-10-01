# Quaxar mainnet validator Full<->Syncing oscillation — fix campaign 2026-10-01

## Three root-cause fixes (all merged to main + deployed + measured)

1. **PR #71 (98966ef7) superseded-session cancellation**: `on_lcl_installed`
   now cancels acquisition sessions whose target seq < installed LCL
   (`CancelReason::Superseded`). Complements #72.

2. **PR #72 (26841f9f) Generic acquisition concurrency cap**:
   `BudgetState.max_generic_in_flight = ledger_fetch_size` (rippled
   `SizedItem::LedgerFetch`, medium=5). Enforced in runner `acquire_requested`:
   a non-continuing Generic demand beyond the cap is `rejected_capacity`.
   Mirrors rippled `findNewLedgersToPublish` `++acqCount < ledgerFetchSize_`.
   Fixed unbounded forward fan-out (was spawning one Generic acquisition per
   future validated seq -> 10-16 concurrent sessions). Measured: generic
   sessions 15 -> 4.

3. **PR #73 (f915266e) parallel TaggedCache sweep**: `TaggedCache::sweep` now
   sweeps partitions via `std::thread::scope` (one worker per partition, joined
   under the lock), matching rippled's threaded `sweepHelper`. Was serial under
   a single `RecursiveMutex`, so it held the lock for the sum of all partition
   scans on a 170k-entry FullBelow cache, starving the consensus strand (shared
   NodeFamily cache mutex). Added `P/T/S: Send` bounds propagated through
   `SHAMapFamily`/`NodeFamily`/`LedgerHistory`/`FetchPackCache`/
   `LedgerMasterSweepTarget`. Also demoted `full_below_published_after_write`
   INFO -> DEBUG (fired ~17/sec, flooding journald).

## Measured result

- Demotion rate: **0.87/min (baseline) -> 0.17/min (~80% reduction)**.
- Deep 15s strand stalls (`proposers_closed=0` AND `proposers_validated=0`):
  **eliminated** (0 in warmed windows).
- Node consistently `proposing`.

## Residual (next sub-task)

~0.17/min demotions now diagnosed as **close-timing lag**. consensusViewChange
evidence: node closes a proposing round on `previous_ledger` ~3 seq behind the
network validated head (e.g. built 107351095 while `min_valid_seq=107351098`,
`preferred=4AD68C2E`). `validation_selection_source=WorkingPreferred`, trie
healthy (35 trusted, matches 46 peers). This is a rippled-correct
`consensusViewChange` reaction (not thrash, not divergence). Next: investigate
`should_close_ledger` / close timing to see whether the node can avoid building
on a stale parent, or confirm it is rippled-correct lag for a lightly-connected
unlisted validator.

## Ops

- Deployed binary: `parallel-sweep-f915266e` (MainPID 149231).
- Rollback chain: `generic-cap-26841f9f` -> `superseded-cancel-98966ef7` ->
  `proposal-wake-9b448449`.
- Server 3.9.172.38, `ssh -i ~/.ssh/quaxar-testnet-2026.pem ubuntu@`.
- Tests: basics 198/0, shamap 198/0, ledger 207/0, app lib 782/0,
  consensus 96/96, acquisition 250/0.

## Iteration 2 findings (2026-10-01 later)

Parallel sweep reduced tree-node-cache sweep lock-hold to ~319ms (from seconds)
for a 17.3M-entry cache. Deep 15s stalls remain eliminated. BUT a milder
residual persists: periodic ~54s stretches where round cadence degrades from
~3.5s to ~17s/ledger (node falls ~15 ledgers behind, then catches up in a
burst), causing the rare consensusViewChange demotions (~0.2/min).

KEY NEW ROOT CAUSE: tree-node-cache is 17,329,517 entries = 8x its 2,097,152
target, with strong_cache_before=5,043,009 (5M STRONG refs), sweep demoting
4.3M strong->weak in one pass. 5M strong SHAMap node refs = many ledgers' nodes
retained strongly, not released after publication. This bloats the cache,
increases memory pressure and per-insert/fetch contention, and makes each 60s
sweep enormous. During the slow window: 49 acquisition_timeout_deferred_for_
local_reconstruction + 33 acquisition_timeout_interval on the acquisition
thread; consensus still runs (15 accepts) but rounds are slow. No single slow
op (no elapsed_us>=100ms).

NEXT SUB-TASK: find why tree-node cache holds 5M strong refs (8x target) vs
rippled, which releases SHAMap node strong refs after ledger publish/release.
Likely a retained Arc<Ledger>/SHAMap somewhere (publication backlog, history
index, or a strong ref held across rounds) preventing eviction. Compare to
rippled NodeFamily / SHAMapStore freshenCache + ledger release ordering.

## Iteration 2 — test suite fixes (merged)

Workspace lib tests: 4111 passed / 0 failed.
Fixed pre-existing failures this iteration:
- PR #74: ripple_path_find source_amount issuer normalization (rippled PathRequest.cpp:685). rpc lib 138->139.
- PR #75: NuDB close() poison-tolerance + preserve-log on crash. nodestore 112->114.
- PR #76: rpc integration test target restored (was non-compiling since #68 added Setup.max_diverged_time/max_unknown_time). Fixed 10 stale expectations: feature Batch->BatchV1_1 (7), amendment-blocked server_state 'connected' (3). rpc integration: non-compiling -> 509 passing.

REMAINING 7 rpc integration failures (standalone-accept cluster), root causes identified:
1. handlers/ledger_accept standalone off-by-one: accept_standalone_ledger (application_root.rs:10397) computes closed_seq = current_idx.max(...). Fresh standalone app's open ledger_current_index is 3, test expects progression to 2. Need: confirm correct standalone genesis open-ledger seq vs rippled (genesis=1, first open=2). Either standalone init double-advances (real bug) or test expectation stale.
2. handlers/tx/formatting (4 tests) + handlers/transaction_entry (1): accept_standalone_ledger fails "standalone accepted state commit failed: InvalidFee(XRPAmount{drops:10})" at application_root.rs:10601 state_view.table().apply(). tx built via signed_payment_tx(...,5,10) fee=10 drops, AppOpenLedgerView::new(2,10). The OpenView apply fee validation rejects 10 drops. Need: check the standalone ledger fee base/settings — is 10 drops legitimately invalid now (fee settings not initialized in standalone genesis) or test-stale.
3. handlers/book_offers/page_shaping book_offers_integration_with_real_ledger_state: not yet diagnosed.

All share the standalone-mode ledger build/fee path. Next iteration: diagnose standalone genesis open-ledger seq + fee settings vs rippled, fix root cause (likely one shared standalone-init issue fixes the fee+index cluster).

## Iteration 3 — residual demotion root cause CONFIRMED (not a bug)

Refocused on the oscillation per user steering. Live analysis (node up ~8h on
parallel-sweep-f915266e):

- Demotion rate ~0.35-0.40/min (down from 0.87 baseline). Deep stalls
  (proposers_validated=0): 0 (eliminated and holding).
- SWEEP RULED OUT as the cause: demotion timestamps (09:03:03, 09:05:40,
  09:11:02/25/36, 09:14:10) do NOT align with the per-minute tree-node-cache
  sweeps (:32-:36). The parallel-sweep fix (lock-hold ~319ms) removed sweep
  starvation. The tree-node-cache growing to ~20M entries (10x target) is
  mostly WEAK entries whose underlying SHAMap nodes are still alive via the
  bounded LedgerCache (64 ledgers, target 64 — correct); weak entries only
  expire when the node's Arc drops, matching rippled TaggedCache semantics.
  This is rippled-equivalent, not a leak.

- RESIDUAL DEMOTIONS ARE RIPPLED-CORRECT minority-validator behavior. The
  consensusViewChange events are predominantly SAME-SEQUENCE, DIFFERENT-HASH:
  e.g. node built FEB04D@107356355 while network validated FB1E159@107356355;
  BAA9A8@107356358 vs E97C0EFF@107356358. The node proposes a locally-built
  ledger that the UNL supermajority does not agree with, so it correctly
  abandons it (consensusViewChange -> demote -> re-sync in ~4s). Because this
  node is an UNLISTED validator (not in anyone's UNL), its validations are not
  trusted by the network, so when its local close differs from the UNL result
  it diverges. rippled behaves identically for an untrusted proposing node;
  consensusViewChange MUST NOT be suppressed (prior parity note).

CONCLUSION: All FIXABLE divergences from rippled that caused the oscillation
are resolved (acquisition fan-out #72, superseded sessions #71, sweep
starvation #73). The residual ~0.35/min is the expected consensus reaction of
an unlisted proposing validator and is not a code defect. Full elimination
would require the node's validations to be trusted by the network UNL (an
operational/governance matter), or running it as a non-proposing tracking node.
