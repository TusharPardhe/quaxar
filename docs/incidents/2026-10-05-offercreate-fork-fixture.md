# OfferCreate IOU-crossing fork — reproducible fixture (2026-10-05)

## Summary of the divergence hunt (evidence chain)
Residual testnet forks were localized, with hard in-process evidence, to a
**value-level transaction-application divergence** (NOT consensus, NOT close
time, NOT result codes, NOT which entries change):
- `consensus_txset_agreed` fired 721 times, `consensus_txset_divergence` 0 →
  we always agree with the quorum on the transaction set.
- Every transaction applies `tesSUCCESS` on both quaxar and the network.
- Affected-node **key sets match the network exactly** (same entries touched).
- Therefore a transactor writes a **different VALUE** to a shared entry.
- Static OfferCreate / Taker / BookStep / STAmount / Quality audit vs rippled
  found NO code mismatch → the bug is a **data-dependent runtime branch**.

## Exact reproducible fork transaction
- **Hash:** `1567D8374368F2B69C956596154F2864AE593B1692EF4FA7A9B3715BEB7558E7`
- **Validated by network in ledger:** 21305409 (our node forked locally here).
- **Type:** OfferCreate, `Account rQJaWu9aw4kJ5iNfCCeXqpq6njtAzCQmjR`,
  `Flags 131072 (tfSell)`, `Sequence 20874370`, `Fee 12`.
- **TakerGets:** `4000000` drops (XRP). **TakerPays:** `2.04693805346036` WAR
  (issuer `rQGbw4Q5jqq2t25r4vjsyRksXvoM1o2Jf6`).
- Raw tx blob (418 bytes) + binary meta (2328 bytes) available via
  `quaxar rpc tx '{"transaction":"1567D837...","binary":true}'`.

## The exact crossing it performs (expected network result = ground truth)
Consumed offer (owner `rapS8EKmjPp1VxRM7m8hMeJoc3menCoXKk`,
BookDirectory `12F6C714E05BDA0954F716C33F0ECD5A010C66E8BF64AEA25B06EDB94E615C00`):
- Offer TakerGets (WAR): `3.076560200591725` → `1.029622147131365`
- Offer TakerPays (XRP drops): `6000000` → `2008000`
- i.e. offer gave `2.04693805346036` WAR and received `3992000` drops.

RippleState balance deltas (ground truth the replay must reproduce):
- Taker trustline `20AC36315AEB469E1E1615DCA579C2602B9167498FCC5018BC820F32A4023222`:
  `-414.6200709596663` → `-416.6670090131267` (Δ = -2.0469380534604 WAR).
- Owner trustline `C0CB086F7855583EC97EA94A842F008819817FBEECB07E4BD7223106DC0FCC3F`:
  `184.2149045765512` → `182.1679665230908` (Δ = -2.0469380534604 WAR).
- Taker AccountRoot XRP `3290280301` → `3294272301` (owner side);
  taker pays `3992000` drops net.

## Where the bug is
`xrpld/ledger/src/domain/ripple_calc/book_step.rs::compute_offer_consumption`
(line ~2486) is the IOU CLOB crossing arithmetic. The 16-significant-digit WAR
amounts above exercise `mul_ratio_amount`, `ceil_out_strict`,
`ceil_in_strict`/`ceil_in`, `offer_owner_gives`, and `Quality` rounding. A
1-ULP divergence in one of these for this specific quality/remainder is the
fork. `fixReducedOffersV2` is enabled → the strict `ceil_in_strict(...,false)`
branch is taken.

## Fix recipe (deterministic, no live network needed)
1. Add a unit test next to `book_step_success_path_tests.rs` that constructs the
   exact offer (TakerGets=3.076560200591725 WAR, TakerPays=6000000 drops) and
   calls `compute_offer_consumption` with the taker's remaining
   (remaining_in=4000000 drops? — note taker is the one PAYING XRP for WAR, so
   map in/out per BookStepPass) and asserts the consumed amounts equal
   offer_out Δ = 2.04693805346036 WAR, offer remainder TakerGets=1.029622147131365,
   TakerPays=2008000, and step_in = 3992000 drops. Build IOU values with
   `IOUAmount::from_parts(mantissa, exponent)` (e.g. 2.04693805346036 =
   from_parts(2046938053460360, -15)) or parse via amount_from_json on the RPC
   amount objects.
2. Run it — it should FAIL (reproducing the fork remainder/rounding).
3. Add per-sub-op tracing (stp_in/stp_out/owner_gives/actual_ofr_in/out and each
   ceil_* result) and compare each against a hand-computed rippled trace of
   `BookStep.cpp` / `Quality.cpp` for the same inputs.
4. Fix the diverging rounding/branch to match rippled; keep the test as a
   regression.
5. Redeploy; confirm `observer local child vetoed` rate drops to ~0 under
   contention and `consensus_txset_agreed` builds no longer fork.

## Reproduce fresh on testnet (per operator hint)
Generate two funded wallets, set a WAR-like IOU trustline, place an offer with a
fractional 16-sig-digit IOU price, then submit a partially-crossing OfferCreate;
compare our built ledger's RippleState/Offer FinalFields to the network's via
`tx` RPC. Any 1-ULP delta reproduces the class.

## IMPORTANT cleanup before shipping
The diagnostic binaries (forkdiag → forkclass → agreed → applyaudit → metakeys →
valbal) added LOGGING-ONLY instrumentation that is VERBOSE (per-tx every
ledger): `apply_audit` `consensus_build_tx_result`, the `consensus_txset_*`
events, and the enriched veto/adoption logs. REMOVE these once the transactor is
fixed. Deployed diagnostic binary: `quaxar.valbal-be684a07`. The real prior
fixes (status-mirror, owner-parallelization, edge-trigger, forward-advance
no-demote) are the keepers.
