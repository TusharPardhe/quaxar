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

## New campaign iter 2 — cascade confirmed; validated chain is correct

Key proofs this iteration:
1. Confirmed divergent tx 01D0D930 (FRH->XRP self-payment) is a CASCADE: txns
   at index 0 and 83 in the same ledger (107359089) also touched the sender's
   FRH RippleState BEFORE the divergent tx at index 439. So the sender's FRH
   line state at tx 439 depends on earlier txns the node may apply differently.
2. The network path for 01D0D930: sender's FRH line DELETED (balance->0),
   consumed via a CLOB offer owned by rwZRg5AaB giving XRP. So the sender DID
   hold FRH at ledger start; the node dried because by tx 439 its view of the
   FRH line differs (cascade).
3. Ruled out (3 passing isolation tests committed):
   - AMM primitive (amm_offer_for_clob_quality) correct.
   - Reverse book step consumes AMM liquidity correct.
   - max_payment_flow returns positive redeem flow for an IOU-holding sender
     (both low and high orientations) correct.
4. The node's VALIDATED/stored ledgers all MATCH the network (checked
   107359348-350,362). The node never persists a wrong ledger; the oscillation
   is purely the transient losing-candidate build -> demote -> re-sync.
5. Current rate on stable binary: ~0.27 demotions/min, ~2 same-seq
   divergences/15min.

DIAGNOSTIC GAP: added a per-tx meta_divergence fingerprint
(sha512_half(metadata) + affected_nodes count) at the consensus-accept site
(application_root ~11219). BUT the trace captured only the WINNING (network-
matching) ledgers' meta, never the divergent seqs' meta in the same journal
window -> the losing candidate's per-tx metadata is built/discarded without
reaching that site in a capturable way, OR the divergent-seq meta scrolled past
the --since window (divergences ~2/15min, meta ~1500 lines/min).

NEXT: capture the LOSING candidate's per-tx metadata. Either (a) widen journal
capture to guarantee the divergent seq's meta is included and diff
affected_nodes vs network per tx to find the FIRST divergent tx, or (b) add the
meta fingerprint earlier in the consensus close (where the losing candidate's
txns are applied, before LCL switch), so the divergent build is always logged.
Then fix the first divergent tx's state-delta rounding.

Node restored to parallel-sweep-f915266e; main has no diagnostic code.

## New campaign iter 3 — FIRST divergent tx FOUND + offer-create parity fix

Breakthrough: continuous journal capture + per-tx meta fingerprint
(affected_nodes count) diffed against the network for divergent ledger 107359777
(same parent C1788B8A, same 51 txns, same close 844174131, network ledger_hash
2192C58F = node's demote target). FIRST divergent tx is at INDEX 35:
  tx 56761BFCA8B7064AACE7818DAC854ADFBBF840B650105ECF76941FCFB7074E68
  OfferCreate, Flags 655360 (tfSell|tfImmediateOrCancel),
  TakerGets 0.000003702929240260918 ETH, TakerPays 0.01 RLUSD.
  NETWORK: tesSUCCESS, 6 affected nodes (crosses the RLUSD/ETH counter-offer
    owned by rMsXVzCug, modifies 4 RippleState + Offer + AccountRoot).
  NODE: tecKILLED (150), 1 affected node (fee only) -> flow_cross crossed NOTHING.
Transactions 0-34 matched (tx_id + affected_nodes), so the book state at idx 35
equals the network's -> tx 35 is a DIRECT bug, not a cascade.

Two issues:
1. FIXED (committed a4ac0665): OfferCreate IOC/FOK result parity. rippled
   returns tecKILLED for an IOC offer that crossed nothing (#4115) and uses
   fixFillOrKill sell-mode (gets-side) completion; Quaxar always returned
   tesSUCCESS for IOC and only checked takerPays for FOK. Now tracks `crossed`
   and matches rippled. (This aligns the RESULT code but tx35 still needs #2.)
2. ROOT (next): flow_cross crosses NOTHING for tx35's TINY tfSell offer
   (0.0000037 ETH / 0.01 RLUSD) where rippled crosses the counter-offer. This
   is a tiny/sub-precision offer-crossing bug in flow_cross/book_step for the
   sell direction (near IOU precision limit; likely fixReducedOffers / small
   increased-quality offer handling). Reproduce: cross tx35's offer against the
   rMsXVzCug RLUSD/ETH offer (TakerGets 1907 RLUSD / TakerPays 0.706 ETH) and
   assert positive cross; compare the rounded step amounts to rippled BookStep.

NEXT: build a focused flow_cross reproduction for tx35's crossing, find why the
tiny tfSell offer crosses zero, fix the rounding to match rippled, verify the
affected-node count becomes 6 and result tesSUCCESS.

## Iter 4 — tiny-amount hypothesis DISPROVEN; localized to BookTip traversal

Decoded tx 35 fully: OfferCreate tfSell|tfImmediateOrCancel, TakerGets
0.000003702929240260918 ETH / TakerPays 0.01 RLUSD, node tecKILLED(150) / 1
affected vs network tesSUCCESS / 6 affected (crossed the rMsXVzCug ETH/RLUSD
offer).

Built a reproduction (xrpld/app/tests/integration/offer_crossing.rs
`tiny_sell_ioc_offer_crosses_resting_liquidity`): a sub-unit tfSell+IOC offer
(gives 1 drop XRP, wants 0.0001 USD) against deep resting liquidity CROSSES
correctly (tesSUCCESS, committed as a passing regression guard). => The
tiny/sub-precision rounding hypothesis is DISPROVEN; CLOB crossing of tiny
offers is correct. (Early red runs were test-direction errors: both offers on
the same book side -> genuine no-cross -> tecKILLED, which also independently
validated the committed IOC no-cross tecKILLED parity fix a4ac0665.)

Localized the tx-35 "crossed nothing" to BookTip traversal
(xrpld/ledger/src/domain/book_tip.rs). For tx 35 the node's flow_cross loop got
`next_offer == None`, i.e. `view.succ(book_base, Some(book_end))` returned None
=> no directory page found in tx35's book range. Candidate causes to verify
next:
  1. get_book_base/book_end orientation for the ETH(issuer rvYAfWj5)/RLUSD
     (issuer rMxCKbED) pair vs the counter-offer's BookDirectory
     4327BA72...057C3 (does succ's (book_base, book_end) range actually contain
     that directory?).
  2. SEPARATE confirmed concern: BookTip::step ERASES current_entry on every
     advance (view.erase at top of step) -- destructive read traversal; correct
     only because flow_cross treats step as consume. Revisit when moving
     OfferCreate onto the full flow() engine.

NEXT (final iter): build a byte-exact tx-35 replay fixture (parent ledger +
both book directions + account roots + trust lines for ETH/RLUSD), replay via
the offer_crossing fixture harness, confirm the node reproduces tecKILLED, then
fix get_book_base/succ (or move OfferCreate crossing to the flow() engine used
by payments) so the counter-offer is found and crossed -> tesSUCCESS / 6
affected. Verify affected-node parity, run full suites, deploy, measure
demotions/min = 0 over >=15min.

## Iter 5 — CRITICAL correction: real OfferCreate path + crossing setup verified rippled-faithful; divergence is inside the flow engine

CORRECTION: the real consensus OfferCreate apply path is
`xrpld/app/src/state/offer_create.rs::do_offer_create` (dispatched from
transactor_dispatcher.rs:3174), NOT `xrpld/tx/src/utility/offer_create.rs`
(system_invoke_apply) which is NOT wired into app apply. The iter-3 FOK/IOC fix
(a4ac0665) was applied to the non-dispatched utility module — harmless and a
correct improvement there, but it is NOT the path that produced tx35's
tecKILLED. The real app path ALREADY has the correct rippled IOC logic
(`if is_ioc { if !crossed { tecKILLED } }`, offer_create.rs:487-492) and
derives `crossed` from the flow result via `offer_was_crossed`.

Verified the real OfferCreate crossing setup is rippled-faithful end to end:
  - Book base: get_book_base hashes [in.ccy,out.ccy,in.acct,out.acct] (==
    rippled getBookBase); low 64 bits zeroed; next_quality_key == getQualityNext
    (+1 at bit 64). succ(book_base, book_end) range covers all qualities.
  - Threshold: Quaxar `Quality::from_amounts(Amounts::new(send_max,
    taker_pays))` == rippled OfferCreate.cpp:429 `Quality{takerAmount.out,
    sendMax}` (Quality.h:132 => Amounts(in=sendMax, out=TakerPays)). MATCHES.
  - Crossing engine: `strand_flow::execute_strands` (the SAME full flow engine
    payments use), with OfferCrossing::Sell, limit_quality=threshold, sendMax,
    IOU/IOU XRP-bridge path added. Matches rippled flowCross(flow()).

=> tx35 crossed nothing because `execute_strands` returned a DRY result for
tx35's exact ETH(rvYAfWj5)/RLUSD(rMxCKbED) liquidity at these high-precision
amounts (TakerGets 0.000003702929240260918 ETH / TakerPays 0.01 RLUSD). The
divergence is INSIDE the flow-engine strand evaluation (BookStep quality
rounding / limitQuality reject / AMM-vs-CLOB selection) for this specific
book+amounts — NOT in OfferCreate result handling, book keying, or threshold
construction (all verified equal to rippled this iteration). Synthetic
round-number tiny tfSell+IOC crosses correctly (committed guard
`tiny_sell_ioc_offer_crosses_resting_liquidity`), so the edge is specific to
tx35's awkward-precision quality vs the counter-offer quality
4327BA72...057C3.

EXACT NEXT STEP (unavoidably requires real state; cannot be guessed): capture
tx35's parent-ledger state via JSON-RPC — the 6 affected objects (taker
AccountRoot; taker ETH + taker RLUSD + rMsXVzCug ETH + the RLUSD gateway
RippleStates; the rMsXVzCug Offer) plus BOTH book directions and any ETH/RLUSD
AMM — as an offer_create_* fixture, replay through the offer_crossing fixture
harness, confirm the node reproduces tecKILLED, then instrument
execute_strands/BookStep limitQuality to find where the counter-offer at quality
4327BA72...057C3 is rejected against threshold Quality{out=RLUSD 0.01,
in=sendMax ETH}, and fix the rounding to match rippled StrandFlow limitQuality
(the StrandFlow.h:730 reject is faithful, so the divergence is in the quality
VALUE computed for the offer or the threshold at this precision). Then verify
affected-node parity == 6/tesSUCCESS, run full suites, deploy, measure
demotions/min = 0 over >=15min.

## Iter (round 2) — PRIMARY ROOT CAUSE FIXED: IOU/IOU offer-cross XRP-bridge path

Found and fixed the primary oscillation root cause. In app/state/offer_create.rs
the IOU/IOU crossing path adds an XRP intermediate (rippled OfferCreate.cpp:447
`path.emplaceBack(nullopt, xrpCurrency(), nullopt)`). Quaxar built it via
STPathElement::inferred(.., force_asset=FALSE); for an XRP asset that sets NO
type bit -> TYPE_NONE, which valid_path_element rejects as temBAD_PATH(-291).
Result: to_strands returned ZERO strands, the cross was treated as dry, and
EVERY IOU/IOU OfferCreate needing to cross returned tecKILLED instead of
crossing -> divergent tx-tree/account hash -> consensusViewChange demotion ->
oscillation.

Fix (commit 567ee50d): force_asset=TRUE so the XRP intermediate carries
TYPE_CURRENCY. Reproduced exactly as mainnet seq 107359777 tx35
(tfSell|IOC ETH/RLUSD): tecKILLED -> tesSUCCESS
(test tx35_exact_tiny_sell_ioc_crosses_deep_offer). Diagnosed via tracing that
to_strands returned Ter(-291)/0 strands before the fix.

Deployed to mainnet (binary quaxar.xrp-bridge-567ee50d; rollback
parallel-sweep-f915266e retained). Post-deploy measurement, classifying each
consensusViewChange as SAME-SEQ-DIVERGENT (real oscillation: local_closed seq ==
preferred seq, wrong hash) vs node-behind-lag (benign: local < preferred,
catching up):
  - Full 95 min since deploy: 47 demotions = 6 same-seq-divergent + 41 lag.
  - The 6 divergent were 2 at warmup (18:14, empty DB) + a 4-cluster 18:35-39.
  - Last 20 min: 0 same-seq-divergent, 9 lag.
  - Last 10 min: 1 same-seq-divergent, 5 lag.
So the IOU/IOU offer-cross divergence is largely eliminated, but a RESIDUAL
same-seq-divergent source remains (~1 per 10 min, down from the pre-fix rate).
node-behind-lag demotions are a separate benign performance issue (node falls
~4-14 ledgers behind and resyncs; NOT a wrong-hash build).

NEXT: redeploy the meta_divergence diagnostic (diag branch only) to capture the
NEXT same-seq-divergent seq's first divergent tx (as done for tx35), identify
its tx type, fix that transactor to match rippled, redeploy, confirm
same-seq-divergent == 0 over >=20 min. Then address node-behind-lag separately
(throughput) if Full<->Syncing transitions must also reach 0.

## Iter (round 2 cont.) — residual divergence IDENTIFIED: multi-hop self-payment arbitrage flow

With the offer-cross fix deployed, used the meta_divergence per-tx trace
(diag/meta3, now deleted; main is diagnostic-free) to capture 3 same-seq
divergent builds (seq 107366760, 107366764, 107366839) and diffed each tx's
(index, tx_id, affected_node_count) against the live network.

CONFIRMED FIRST DIVERGENCE (seq 107366839, idx 104): a Payment
6277260729FE..., Account == Destination == rogue5HnPRSszD9CWGSUz8UGHMVwSSKF6
(a circular self-payment / arbitrage), tfPartialPayment (Flags 131072):
  Amount  = CSC  1051.839906607096  (issuer rCSCManTZ...)
  SendMax = BITX 0.0007503332193915335 (issuer rBitcoiN...)
  Paths   = [ SOLO(rsoLo2S1...) -> SOLO issuer -> CSC(rCSCManT...) -> CSC issuer ]
  i.e. ripple BITX -> SOLO -> CSC, plus the default direct BITX -> CSC path.
  NETWORK: tesSUCCESS, 7 AffectedNodes (CSC + BITX + SOLO RippleStates across
    the rippling hops + AccountRoot).
  NODE: tesSUCCESS but only 5 AffectedNodes -> it rippled through FEWER hops
    (skipped the SOLO intermediate trust lines), producing different metadata
    -> different tx hash -> different ledger hash -> consensusViewChange.

This is a MULTI-PATH FLOW SELECTION divergence in ripple_calculate /
execute_strands: given the default path + an explicit SOLO-bridge path, the node
selects/combines a different set of strands than rippled's flow(), touching 2
fewer trust lines. It is a DIFFERENT root cause from the offer-cross XRP-bridge
fix. The other divergent seqs (760/764) also center on self-payment path
payments (tecPATH_PARTIAL/tecPATH_DRY), confirming multi-hop rippling arbitrage
as the residual class. Rate is low (~1 per few minutes) vs the pre-fix rate.

Separately, node-behind-lag demotions (local_closed seq < preferred seq;
benign catch-up, NOT wrong-hash) remain and are a throughput concern, not a
divergence.

NEXT: reproduce the idx-104 BITX->SOLO->CSC self-payment with tfPartialPayment
against a CSC/BITX/SOLO multi-currency fixture, compare node strand
selection/ordering to rippled flow() (default-path + explicit-path strand set
and the order they are consumed), fix ripple_calculate/execute_strands to match,
verify affected-node parity (7 nodes), then confirm same-seq-divergent == 0 over
a sustained window.

## Iter (round 3) — residual divergence CLASS confirmed: multi-hop self-payment arbitrage through offer BOOKS

Deployed-fix steady-state measurement (binary xrp-bridge-567ee50d): across a
~20 min capture, same_seq_divergent demotions = 3 (seqs 107367440/454/458),
node_behind_lag = 7. Divergent builds are much rarer than pre-fix but NOT zero.

Characterized the residual class precisely by fetching each divergent ledger
from the network: EVERY divergent seq contains multiple **self-payment
arbitrage path-payments** (Account == Destination, tfPartialPayment, explicit
Paths with type-48 currency = BOOK steps) run by market-maker bots doing
circular crosses, e.g. CORE->BITX, CSC<->PLX, RED<->PLX, EURO, SOLO/BITX/CSC.
Their AffectedNodes vary (5, 6, 7, 9, 11) and the node computes a DIFFERENT
affected-node count than the network for some -> divergent tx tree -> hash
mismatch -> consensusViewChange.

Reproduction finding (sub-agent, test
xrpld/app/tests/integration/payment_flow_divergence.rs
`mainnet_107366839_self_payment_must_modify_both_solo_hops`): the idx-104 case
(BITX->SOLO->CSC) does NOT reproduce from trust lines alone -> it returns
tecPATH_DRY. The path's type-48 elements are OFFER-BOOK steps, so the delivery
crosses the BITX/SOLO and SOLO/CSC order books; the SOLO trust-line changes are
side effects of the offer owners' balances moving. Adding bridge book liquidity
makes it deliver. => The divergence is in BOOK-STEP crossing WITHIN a payment
path (offer_crossing=No), not pure rippling. This is a DIFFERENT code path from
the OfferCreate XRP-bridge fix (offer_crossing=Sell/Yes) already landed.

So the remaining root cause: payment-path book crossing (execute_strands /
execute_book_step with offer_crossing=No) consumes a different set/amount of
offers than rippled for these multi-book arbitrage paths. The OfferCreate fix
does not cover it because payment paths are explicit (no synthetic XRP bridge).

NEXT (hardest, do first): build a COMPLETE replay fixture for one divergent
arbitrage payment INCLUDING the offer books it crosses (capture, from parent
ledger, the book_offers for each hop's book: book_offers taker_gets/taker_pays
for BITX/SOLO, SOLO/CSC, etc., plus the offer-owner trust lines and transfer
rates), replay via the fixture harness, confirm node affected-node count != net,
then diff the node's crossed offers vs the network's AffectedNodes to find which
offer/book the node mis-crosses, and fix execute_book_step for the
offer_crossing=No payment path to match rippled. The clustered recurring seqs
(same MM bots every few ledgers) make capture reliable: pick a current divergent
seq, pull its self-payment txns + referenced books from the network, build the
fixture.

## Iter (round 4) — SECOND root cause FIXED: CanonicalTXSet ordering (TransactionIndex shift)

Using the meta_divergence trace + per-tx meta_hash diff (match network tx by
computing tx id = sha512_half(0x54584E00 ++ tx_blob) from `ledger expand binary`,
then compare sha512_half(meta) to the node's logged meta_hash), root-caused a
divergence on mainnet seq 107368090: a CONTIGUOUS tail (node idx 72..96) all
diverged while idx 0..71 matched. The node assigned TransactionIndex 72 where
the network assigned 74 -- a constant +2 shift -- with ALL balances/state
identical. Cause: account r3ASEe1LLn had 4 consecutive sequences (595..598);
the network applied them contiguously (TI 70,71,72,73) but the node applied
595,596 early (idx 70,71) and DEFERRED 597,598 to the end (idx 95,96).

Root: the consensus-accept build loop consumed `result.txns.all_items()` in raw
SHAMap (tx-hash) order, NOT CanonicalTXSet order. Out-of-sequence same-account
application forces a spurious retry that re-applies in a later pass at a later
TransactionIndex; since TransactionIndex is serialized into every tx's metadata,
that shifts the metadata of every subsequent tx and diverges the ledger hash.
rippled RCLConsensus::buildLCL applies a CanonicalTXSet (retriableTxs) in
canonical order (salted account, seq, tx id).

FIX (commit 5886b03a): order the decoded consensus txns through
CanonicalTXSet(salt = tx-set id) before the build loop. Deployed as
quaxar.canonorder-5886b03a.

MEASURED (diag binary = fix + trace): over ~26 min steady proposing,
same_seq_divergent dropped to 1 (was ~3/20min + the frequent TI-shift class),
node_behind_lag 9. The single remaining divergence (seq 107369025 idx 82) is
NOT a contiguous tail (only idx 82 differs) => the TI-shift class is ELIMINATED.

REMAINING (one class left): seq 107369025 idx 82 is an OfferCreate
Flags=786432 (tfSell|tfFillOrKill) that crosses and DELETES 2 offers (13
affected nodes). Single-tx content divergence in OfferCreate multi-offer book
crossing (consumption amount / which offers consumed / rounding). Distinct from
the XRP-bridge path fix and the canonical-order fix. This is the last known
divergence class; rate ~1 per 26 min.

NEXT: capture seq 107369025 idx 82's node AffectedNodes content (the diag trace
logs `nodes=`) and diff against the network's 13 AffectedNodes to find the exact
differing balance/offer, then fix the tfSell|FOK multi-offer crossing
consumption in execute_book_step to match rippled.

## Iter (round 5) — remaining class ROOT-CAUSED: AMM output rounding in tfSell|FOK crossing

With the two prior fixes deployed (XRP-bridge + CanonicalTXSet), the residual
divergence (~1 per 26 min, non-contiguous single-tx) was captured and
root-caused via meta_divergence + per-tx meta_hash diff.

DIVERGENT TX: mainnet 107369025 idx 82, OfferCreate
CE565E16A5DFF7B9D996206C48CBD0F552CA4754774EE884437193C450A7DE75:
  Account rLPV1SBMbiTMno43PxhBEMWK9pz1hXYk53, Flags 786432 (tfSell|tfFillOrKill),
  TakerGets 1560000 drops XRP (sell XRP), TakerPays 2.32361370726417 RLUSD.
It crosses: (1) a CLOB offer owner rU8QEjAbNbXDnBR7kx9TLPXsAEng3axsz4 giving
0.790418 RLUSD for 530658 drops; (2) an AMM pool
rhWTXC2m2gGGA9WozUaoMm6kLAVPb1tcS3 (confirmed AMMID) XRP/RLUSD, trading_fee=197,
parent reserves XRP=1526460107335 drops, RLUSD=2278196.924178077; and it
self-cross CANCELS the creator's own resting offer (seq 105265383), which both
node and network delete.

DIVERGENCE: node delivers 2.323622646 RLUSD to the taker; network delivers
2.323653246 (taker RLUSD line 17.13776773080886 -> network 19.46142097680886 vs
node 19.46139037215994). Node is short by EXACTLY 0.000030600 RLUSD, entirely in
the AMM leg (CLOB leg 0.790418 matches). => AMM synthetic-offer output rounding
in the sell crossing is slightly low vs rippled.

Reproduced the SYMPTOM deterministically (test
offer_crossing.rs::mainnet_107369025_sell_fok_multi_source_delivery_matches_network,
currently #[ignore]) but with a CLOB stand-in for the AMM, so it is not yet a
faithful AMM model.

CODE AREA: xrpld/ledger/src/domain/ripple_calc/book_step.rs AMM path --
get_amm_offer -> amm_offer_for_clob_quality (fixAMMv1_1 branch ->
amm_offer_starting_with_gets / amm_offer_starting_with_pays /
generate_fibonacci_amm_offer) -> amm_swap_asset_in/out. The output
number_to_amount(..., RoundingMode::Downward) and the staged
Upward/Downward guards are the suspects for the -0.0000306 shift.

NEXT (final): build a FAITHFUL AMM reproduction -- create the XRP/RLUSD AMM pool
(reserves above, fee 197) via amm_utils in xrpld/ledger/tests/domain/, place the
rU8Q CLOB offer and the creator self-offer, enable fixAMMv1_1 + fixFillOrKill +
fixReducedOffersV2, apply the tfSell|FOK OfferCreate, and assert the AMM leg
delivers 1.533235246 RLUSD (network) not the node's 1.533204646. Then compare
the node's generated AMM offer (amm_offer_for_clob_quality output) to rippled's
AMMLiquidity/BookStep for this pool+quality and fix the rounding to match.

STATUS: 2 root causes fixed+deployed (frequent oscillation eliminated); this AMM
sell-crossing rounding is the last known divergence class. Node healthy on
quaxar.canonorder-5886b03a.

## Iter (round 6, final) — AMM offer GENERATION verified faithful; divergence narrowed to limit/application

Compared the node's AMM offer-at-quality generation to rippled line-by-line:
  - node amm_offer_starting_with_pays == rippled getAMMOfferStartWithTakerPays
    (AMMHelpers.h): identical quadratic (a=f, b=pool.in*(1+f),
    c=pool.in^2 - pool.in*pool.out*q), identical nTakerPaysConstraint, identical
    `toAmount(pool.in, nTakerPays, Downward)` then swapAssetIn, identical
    reduceOffer fallback.
  - node amm_offer_starting_with_gets == rippled getAMMOfferStartWithTakerGets.
  - node amm_swap_asset_in/out staged Upward/Downward guards match AMMHelpers.
This case is SINGLE-path (XRP<->RLUSD, one side native, no IOU/IOU XRP bridge),
so it uses amm_offer_for_clob_quality (not the multi-path Fibonacci path), which
is faithful. => the AMM synthetic-offer GENERATION is not the divergence.

Therefore the -0.0000306 RLUSD is in how BookStep APPLIES/limits the generated
AMM offer during the sell crossing: get_amm_offer -> amm_offer.limit(raw_in_limit
= mul_ratio(remaining_in, QUALITY_ONE, tr_in, false), remaining_out) -> the
clamped re-swap, and the step_in/step_out accounting (mul_ratio tr_in rounding),
plus how the AMM contribution is summed with the CLOB contribution
(insert_sorted/sum_sorted smallest-to-largest IOU re-summation). The exact
differing step requires a faithful multi-liquidity replay (real AMM pool
reserves 1526460107335 drops / 2278196.924178077 RLUSD, fee 197 + the rU8Q CLOB
offer + the self-cross offer + fixAMMv1_1) to diff the node's clamped AMM output
(1.533204646) vs rippled's (1.533235246).

MEASURED (2-fix binary quaxar.canonorder-5886b03a, 30 min steady proposing):
same_seq_divergent = 2 (seqs 107369025, 107369368 -- BOTH contain tfSell|FOK
OfferCreates crossing AMM liquidity), node_behind_lag = 14. The remaining
divergence is this ONE class (AMM sell-crossing output application).

CAMPAIGN RESULT: 3 root causes fixed+deployed over the campaign
(1) IOU/IOU OfferCreate XRP-bridge path temBAD_PATH (commit 567ee50d),
(2) CanonicalTXSet ordering / TransactionIndex shift (commit 5886b03a),
(and earlier the acquisition/sweep PRs). Frequent oscillation ELIMINATED;
residual is a single rare AMM sell-crossing rounding class (~2 per 30 min),
narrowed to the BookStep AMM offer limit/application (generation proven faithful
to rippled this iteration). Node healthy and proposing on the 2-fix binary.

## Iter (round 7) — AMM BookStep code diffed line-by-line vs rippled; no formula divergence found

Checked the full AMM sell-crossing path against rippled:
  - node amm_offer_starting_with_pays/gets == rippled getAMMOfferStartWithTakerPays/Gets (AMMHelpers.h): identical quadratic, constraint, toAmount(..,Downward), swapAssetIn/Out, reduceOffer fallback.
  - node SyntheticAmmOffer::limit == rippled AMMOffer::limitOut/limitIn (AMMOffer.cpp): multiPath -> ceil_out_strict/ceil_in_strict(fixReducedOffersV2); single-path -> swapAssetOut/swapAssetIn(limit). Identical.
  - node amm_target_quality == rippled BookStepCrossing::qualityThreshold (BookStep.cpp:478): returns None (uncapped AMM) iff fixAMMv1_1 && !multiPath && threshold>lobQuality; else lobQuality. Identical. For 107369025 idx82 the taker quality (0.000001489496 RLUSD/drop) is slightly WORSE than the CLOB tip (0.000001489505), so the AMM is correctly capped to the CLOB tip (not uncapped).
  - node forEachOffer order == rippled: tryAMM(clobTipQuality) once, then CLOB offers (BookStep.cpp:887).
  - Verified arithmetically: swapAssetIn(1029342 drops) = 1.5332352648 => rounds to the NETWORK value 1.533235246. The node consumed ~1029321 drops (1.533204646), ~21 drops short.

CONCLUSION: no divergent AMM FORMULA or control-flow vs rippled was found; every
checked function matches. The residual -0.0000306 RLUSD (~21 drops of AMM input)
is a sub-ULP Number/STAmount precision artifact that only manifests with the
byte-exact mainnet pool mantissas. A faithful AMM_CREATE reproduction
(mainnet_107369025_amm_sell_fok_faithful, ignored) cannot be made byte-exact
because AMM_CREATE-seeded reserves/LP rounding differ from the pool's exact
RippleState at crossing time (amm_info 2278196.924178077 vs the RLUSD RippleState
2278196.910635268), so the test is a diagnostic, not a pass/fail guard.

To fix with certainty, the next step must capture the AMM pool's EXACT RippleState
balances (both the AMM/XRP AccountRoot drops and the AMM/RLUSD RippleState value)
at parent ledger 107369024, inject them directly as ledger entries (not via
AMM_CREATE), replay the crossing, and bisect the single Number rounding guard in
amm_swap_asset_in / number_to_amount / mul_ratio that accounts for the 21-drop
input (or ~0.0000306 output) delta -- comparing each intermediate Number against
a rippled instrumented run of the same pool.

## New campaign iter 1 — divergence confirmed sub-ULP in LATER hops of multi-step arbitrage flows

Busy-period bursts (10 same-seq-divergent in 15 min) are dominated by rogue5-style
market-maker self-payment arbitrage (Account==Destination, tfPartialPayment,
multi-currency Paths crossing chains of IOU books + AMMs). Example captured:
seq 107376752 idx 5 (hash C935028F...): self-payment XAH -> ... -> PLX, 9
affected nodes.

Byte-level finding (meta_divergence node-content trace, matched by tx id =
sha512_half(0x54584E00 ++ tx_blob) vs sha512_half(meta)):
  - For idx 5, the FIRST 5 affected-node final balances match the network
    EXACTLY (-122952.8345465588, -3632.257444769506, 55870015833.29757,
    -65859.94438178065, 3741456.286725798).
  - The divergence is in a LATER hop (nodes DAFDE597 / E110DF4B2500 /
    EF88142AFEEF, beyond the 6000-char trace truncation) -- a sub-ULP balance
    difference accumulated through the multi-step strand.
This confirms the residual is accumulated sub-ULP rounding in the LATER steps of
long multi-book/AMM strands, not an early-step or ordering error. Divergent txns
are single/non-contiguous (idx 5 and 114 independently), i.e. content rounding,
not TransactionIndex shift (that class remains fixed by CanonicalTXSet).

Verified earlier (still holds): AMM offer generation (getAMMOfferStartWith*),
AMMOffer::limitIn/Out, BookStepCrossing::qualityThreshold, forEachOffer order,
and swapAssetIn/Out all match rippled line-by-line. The residual sub-ULP delta
only manifests with byte-exact multi-book/multi-AMM live state, which is NOT
reconstructable from fetchable RPC data (every crossed offer book + AMM pool at
the exact ledger; AMM_CREATE cannot seed byte-exact reserves/LP rounding).

HARD BLOCKER for a verified fix: cannot build a byte-exact reproduction of these
live multi-liquidity arbitrage flows, so cannot isolate the single Number
rounding guard that differs, and cannot verify a candidate fix without risking
regression of the extensive passing AMM/flow parity suite. A verified fix
requires either (a) a mainnet-state snapshot replay harness that loads the exact
parent-ledger SHAMap (all offer dirs + AMM pools) and replays the tx, diffing
each strand step's Number against an instrumented rippled run of the same
snapshot, or (b) rippled-side instrumentation to dump per-step Number values for
these exact txns. Node remains healthy on the 2-fix binary
quaxar.canonorder-5886b03a; validated chain always correct (divergences are
transient losing candidates).

## New campaign iter 2 — KEY MECHANISM: content divergence (tecPATH_DRY) CASCADES into TransactionIndex shift

Captured a busy-period divergent ledger (seq 107377610) with the full-content
meta trace. The divergent set was a CONTIGUOUS tail (node idx 48-55), initially
looking like the ordering bug. But it is a ROTATION: node idx 48->55 map to net
TI 49->55 (+1), and node idx 55 -> net TI 48. I.e. one tx (A58C45C93D) the
network placed FIRST (TI 48) the node placed LAST (idx 55).

ROOT: A58C45C93D is a tfPartialPayment SELF-payment (rfgtQeBdx), deliver
44426293 drops XRP, SendMax 29.1495305002868 USDC, NO explicit Paths. Network:
tesSUCCESS, 4 nodes, crossing the USDC/XRP AMM rGHt6LT5 (confirmed AMMID; parent
reserves XRP 38244294956 / USDC 58282.14276260956, fee 54). NODE: tecPATH_DRY
(Ter 128), 1 node -- it found NO default-path AMM liquidity. Because tecPATH_DRY
is a retry-class result, the node deferred the tx to a later pass and re-applied
it at idx 55, shifting the TransactionIndex of every tx in between -> contiguous-
tail metadata/hash divergence.

=> IMPORTANT: the sub-ULP/edge AMM divergence and the TransactionIndex-shift
bursts are the SAME underlying bug. When the node's AMM crossing returns dry (or
a different result) at a specific pool state, the retry re-orders the tx and
cascades the hash divergence across the rest of the ledger. Fixing the AMM
dry/edge case fixes BOTH.

Reproduction status: a default-path self-payment (deliver XRP, SendMax USDC, no
Paths) against the USDC/XRP AMM at the amm_info reserves DELIVERS correctly
(tesSUCCESS ~19.1M drops) in the node -- committed as guard
amm_default_path_self_payment_delivers_xrp_not_path_dry. So the live tecPATH_DRY
is NOT a plain default-path-AMM failure; it only occurs at the pool state AFTER
earlier same-ledger txns depleted/modified the AMM (idx 48 is preceded by 47
txns, several of which are AMM-crossing arbitrage). The dry condition is
state-dependent on intra-ledger AMM consumption.

NEXT: reproduce by applying the SEQUENCE of same-ledger AMM-touching txns before
idx 48 (or capture the AMM pool's exact RippleState/AccountRoot at the point idx
48 executes via the node's own store), to get the depleted pool state at which
the node's get_amm_offer/amm_offer_for_clob_quality returns None (dry) while
rippled returns a deliverable offer, then fix that None-returning edge in
book_step.rs AMM generation.
