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

## Iteration 3 — ACTUAL DIVERGENCE ROOT CAUSE FOUND (txn-level)

The ledger-hash divergences are NOT minority-validator noise. Confirmed via
per-txn hash comparison against the live network (xrplcluster) for divergent
ledger 107356694 (node built 9532FEFC, network E840E5EB; identical parent 693
468D8EF8, identical close_time 844162362, identical 60-tx set and canonical
order for 0-56):

ROOT CAUSE: transaction 8CD880D2A11D028603353992D76CF081F8282D9DE88B57793A524560F7E55D08
diverges in RESULT:
- Network: tesSUCCESS, partial payment, delivered 19309377 drops XRP.
- Node:    tecPATH_DRY on pass 0/1/2 (then accepted as tecPATH_DRY at idx 59).

The tx is a self-directed tfPartialPayment (Flags=131072): Account==Destination
rfgtQeBdxp8yn7N2XAxGstMkPzSG9Pf8N3, DeliverMax=42664917 drops XRP,
SendMax=28.7587 USDC (issuer rcEGREd8NmkKRE8GE424sksyt1tJVFZwu). Network meta
touches 2 AccountRoot + 2 RippleState (direct IOU rippling / book path, no
Offer node). Quaxar's flow engine returns tecPATH_DRY (no liquidity) where
rippled finds partial liquidity and delivers 19309377 drops.

Consequences cascade: tecPATH_DRY vs tesSUCCESS => different metadata +
different state (RippleState/AccountRoot balances) + the failed tx retries,
reordering it from idx 57 to idx 59 => different tx tree hash + account_hash
=> divergent ledger hash => consensusViewChange demotion.

NEXT: fix Quaxar's payment flow engine so a tfPartialPayment with available
partial liquidity through trust lines/book delivers (tesSUCCESS partial)
instead of tecPATH_DRY. Compare strand_flow / flow_engine partial-payment
liquidity + path-dry determination to rippled Flow.cpp / StrandFlow.

## Iteration 3 cont. — divergence mechanism refined

The divergent tx 8CD880D2 (idx 57) is a self-directed tfPartialPayment swapping
USDC->XRP. Confirmed:
- The USDC->XRP CLOB has liquidity (node book_offers RPC sees 3 offers).
- An XRP/USDC AMM exists (38763 XRP / 57500 USDC, fee 54).
- Mainnet metadata: AMM swap (AccountRoot+RippleState, NO Offer consumed) => AMM won.
- AMM keylet (sorted pair), Issue/Asset Ord (XRP first), Quality(Amounts)=getRate(out,in),
  and the spot<=clob AMM-skip gate ALL match rippled exactly.
- Computed AMM spot quality 0.6741 > CLOB tip 0.6677 => AMM should win and deliver.

So the per-step logic is rippled-faithful. The node's tecPATH_DRY at idx 57 is
therefore a CASCADE: tx 57 sees an AMM/book state produced by the 56 prior
in-ledger transactions (many OfferCreate/OfferCancel + possible AMM touches).
If Quaxar applied ANY earlier offer/AMM tx with a different fill/rounding
(same result code, different state delta), the AMM pool balance or CLOB tip at
idx 57 differs, flipping spot<=clob and skipping the AMM, then the partial
payment finds no satisfying CLOB quality => tecPATH_DRY.

ROOT CAUSE CLASS: a rounding/fill divergence in offer or AMM execution in an
earlier transaction of the same ledger, cascading to the observable
tecPATH_DRY. NEXT: find the FIRST per-tx metadata divergence (affected-node
balances) in ledger 694 by comparing node-applied deltas vs network deltas for
the OfferCreate/OfferCancel/AMM txns before idx 57, then fix the specific
rounding in book_step/offer execution.

## Iteration 3 — DIRECT BUG confirmed, isolated to AMM numeric execution

Verified via network metadata that NEITHER the XRP/USDC AMM
(rGHt6LT5v9DVaEAmFzj5ciuxuj41ZjLofs) NOR the USDC->XRP CLOB offers were touched
by any prior tx (idx<57) in ledger 694. Therefore tx 8CD880D2 (idx 57) sees the
EXACT parent-693 state, which the node has correct. So its tecPATH_DRY (vs
network tesSUCCESS AMM swap delivering 19309377 drops) is a DIRECT determinism
bug in Quaxar's AMM-in-payment execution, not a cascade.

Exhaustively verified these match rippled line-for-line (NOT the bug):
- AMM keylet amm(in,out) sorts the pair (XRP first); Issue/Asset Ord.
- Quality(Amounts)=getRate(out,in); CLOB dir quality same orientation.
- AMM spot(0.6741) > CLOB tip(0.6677) => AMM should win (verified by hand).
- spot<=clob skip gate (AMMLiquidity.cpp:175) identical.
- side select outIntegral && (!inIntegral||rate>=1) -> StartWithTakerGets.
- getAMMOfferStartWithTakerGets quadratic (b,c), constraint, reduce: identical.
- solveQuadraticEqSmallest citardauq form: identical.
- fee_mult=1-getFee, getFee=tfee/100000: identical.
- amm_trading_fee auction-slot logic matches (account not slot owner -> base 54).
- swapAssetOut fixAMMv1_1 staged rounding dirs (Up num, Down denom, Up ratio-pool,
  Down feeMult): match.

REMAINING SUSPECT (the only unverified layer): the RuntimeNumber primitive
(multiply/divide/root2) or number_to_amount/toAmount rounding producing a
sub-ULP difference that flips the final `Quality{amounts} >= target` check in
amm_offer_for_clob_quality, causing get_amm_offer to return None => AMM skipped
=> partial payment finds no satisfying CLOB quality => tecPATH_DRY.

REPRODUCTION (deterministic, offline-capable): apply tx
8CD880D2A11D028603353992D76CF081F8282D9DE88B57793A524560F7E55D08 against ledger
693 (468D8EF80B4B7CAE5C7643A5316481766C5B592F42B1C67ADF4EB85101144171) state.
Expected: tesSUCCESS, deliver 19309377 drops via AMM. Quaxar: tecPATH_DRY.
A focused unit test comparing get_amm_offer's computed (in,out,quality) for
pool (38763.602643 XRP / 57500.75485900167 USDC, fee 54) at target quality
0.6677 against rippled changeSpotPriceQuality will expose the ULP divergence.

## Iteration 5 — AMM path VERIFIED CORRECT via live tracing (AMM was a red herring)

Deployed a temporary amm_divergence trace to the live validator and captured the
real book-step AMM decisions. Findings:
- 210/298 book steps generated an AMM synthetic offer (AMM works for most pools).
- get_amm_offer None reasons: spot_le_clob (rippled-correct skip when CLOB better),
  amm_sle_not_found (no AMM for that pair), and clob_quality_offer_none.
- Reproduced a clob_quality_offer_none case (RLUSD/XRP AMM, pool 2269641.503657631
  RLUSD / 1532052325896 drops XRP, fee 197, target quality 5694034051977024039)
  as a unit test. amm_offer_starting_with_gets returns None because the
  getAMMOfferStartWithTakerGets CONSTRAINT (pool.out - pool.in/(q*fee_mult)) is
  GENUINELY NEGATIVE (-247,593,049 in exact 50-digit Decimal arithmetic), so the
  AMM legitimately cannot reach the target quality via StartWithTakerGets. This
  matches rippled: changeSpotPriceQuality returns nullopt and the CLOB is used.
  NOT a precision bug, NOT a divergence.
- The USDC->XRP AMM primitive test (amm_offer_for_usdc_to_xrp_clob_tip_is_not_skipped)
  PASSES. So the AMM path is rippled-correct.

CONCLUSION: The AMM synthetic-offer generation is verified correct against
rippled (both the quadratic/changeSpotPriceQuality primitive and the live
decisions). The tecPATH_DRY on tx 8CD880D2 is therefore NOT caused by the AMM
skip; the divergence lies elsewhere in the tfPartialPayment flow (strand
execution / rev-fwd liquidity / partial-delivery handling). The AMM was a red
herring. Diagnostic traces and the incorrect RLUSD assertion were reverted;
the valid USDC regression test is retained on main. Node restored to
parallel-sweep-f915266e (proposing, healthy).

NEXT: re-examine the tfPartialPayment flow rev/fwd execution for 8CD880D2 (the
strand builds and the AMM/CLOB have liquidity, yet total_out=0). Capture the
per-strand rev/fwd amounts for this payment to find where output becomes zero.

## Iteration 5 — divergence narrowed to strand reverse_probe (flow_divergence trace)

Deployed a temporary flow_divergence trace to the live validator and captured
the strand None reasons in execute_single_strand:
- 13 reason=reverse_probe_none  <- the tfPartialPayment dry class (e.g. 8CD880D2;
  no quality_threshold, so it cannot be a threshold reject). The strand's rev()
  pass produces zero -> SingleStrandResult None -> strand deactivated -> total_out=0
  -> tecPATH_DRY. THIS is the real divergence to fix next.
- 2 reason=replay_probe_none.
- 27 reason=quality_threshold_reject with realized.value - limit.value ~1.7e-10
  relative (13th digit). These are limitQuality payments. The reject matches
  rippled StrandFlow.h:730 (q < limitQuality && !(adjustedRemOut && within 1e-7)).
  The 1e-7 tolerance is gated on adjustedRemOut (= out_from_avg_q produced
  out < remainingOut). If both rippled and Quaxar get adjustedRemOut=false here,
  both reject (rippled-correct limitQuality miss). Likely NOT a divergence, but
  verify by correlating a reject tx with its network result.

Verified matches rippled line-for-line: Quality Ord (reversed: higher value =
worse), the limitQuality reject condition (StrandFlow.h:730), limitOut /
limit_single_strand_out (adjustedRemOut = out < remainingOut, with 1e-9
within-distance snap to remainingOut), AmmContext multi_path handling.

NEXT (precise): instrument reverse_probe per-step rev() amounts for a
tfPartialPayment USDC->XRP strand (steps: Direct USDC, Book USDC/XRP,
XrpEndpoint). Find which step's rev() returns zero despite AMM+CLOB liquidity.
The book step rev() for the IOU->XRP direction is the prime suspect
(limitStepOut / the reverse book consumption producing zero out).

Node restored to parallel-sweep-f915266e; main has no diagnostic code.

## New campaign iter 1 — CONFIRMED divergence + localized to strand step-0 dry

CONFIRMED a real tx divergence by cross-checking node tecPATH_DRY candidates
against the live network: tx 01D0D930DDF194A379A3A95941949ABE1A9B6BA969A9CD62F7D046C025B03F71
(ledger 107359089) is a self-directed tfPartialPayment FRH->XRP (Account==Dest,
SendMax=FRH, Amount=1M XRP, Flags=131072, no explicit Paths). NETWORK:
tesSUCCESS delivering 564765 drops via a book/offer path (3 AccountRoot, 2
DirectoryNode, 2 RippleState). NODE: tecPATH_DRY. (3 of 4 sampled dry txns the
network ALSO rejects as tecPATH_DRY -> those are rippled-correct; only this one
diverges, so most node tecPATH_DRY are correct.)

Live flow_divergence tracing localized the dry: it is NOT the AMM/book step. It
is STEP 0 of the strand (reverse_probe step_rev_zero at index 0) producing zero
output, in two variants:
- step_kind=Direct with has_line=false: max_payment_flow returns zero because
  there is NO trust line between src and the SendMax issuer for that currency.
  In rippled DirectIPaymentStep::check returns terNO_LINE which REJECTS the
  strand at construction (so an alternative path/default is used), whereas
  Quaxar builds the strand and dries it during execution. SUSPECT: strand
  construction / validate_strand should reject a no-line direct step like
  rippled's check(), OR the default path should not route through a nonexistent
  line.
- step_kind=XrpEndpoint at index 0 (XRP source): the XRP endpoint produces zero.

NEXT: for the FRH->XRP divergence, determine the exact strand rippled builds vs
Quaxar. rippled's default path for a self-payment IOU->XRP with SendMax issuer:
[src -> (direct to issuer) -> book -> XRP]. If src has no line to the FRH
issuer, rippled's check() rejects that strand (terNO_LINE) -> payment uses the
book directly from src's own issued FRH? Verify validate_strand parity with
DirectIPaymentStep::check (terNO_LINE / terNO_AUTH) so Quaxar rejects the same
strands rippled rejects, changing which path executes.

Node restored to parallel-sweep-f915266e; main has no diagnostic code.
