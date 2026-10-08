# Residual Fork — Final Diagnosis (2026-10-06, iters 1–5)

## Node status (verified, end of session)
Fully functional and syncing in lockstep with testnet: `state=full`, our validated
seq within 1–3 of network, 20 peers, **0 wrong-ledger / 0 invariant errors**,
cadence ~1.7s/ledger with **no stalls**. Disk healthy (2.4G free, journal capped
at 300M). Residual forks are **rare, tiny (±1 drop / 2–4 SLEs), and self-healing**
(node adopts the validated chain every time).

## Two fork classes identified and their status

### Class A — over-accumulation / recovery stalls (ROOT CAUSE FOUND + FIXED)
The testnet server disk was **100% full (30G/30G)** during the earlier
investigation. A full disk stalled NuDB/ledger-persistence writes, caused 16–82s
`WrongLedger` recovery stalls, and amplified forks via open-ledger
over-accumulation. **Fixed non-destructively**: vacuumed journald (freed 559M),
removed ~30 stale diagnostic binaries, set a permanent `SystemMaxUse=300M`
journald cap. After the fix the 82s stalls disappeared and cadence became clean.
This was the dominant stability problem.

### Class B — CLOB offer-crossing ±1-drop rounding (ROOT CAUSE PINPOINTED)
Remaining rare forks are a **±1-drop rounding-direction mismatch** in offer
crossing on heavily-traded self-dealing market-maker books (rLgZXam / BEEC74 /
1860F4, book `21FBD3CA`).

**Exact captured reproduction (fork 21332266, binary widetrace-1ae45356):**
- Two XRP `AccountRoot`s differ by **exactly 1 drop**, with **identical**
  `PreviousTxnID`, `Sequence`, `OwnerCount`, `Flags`:
  - `BC3006…`: ours 262640361 vs validated 262640360 (+1)
  - `1860F4…`: ours 609392365 vs validated 609392366 (−1)
- Crossing (`CLOB_OFFER_CONSUMPTION`): resting offer `2E92317F` with
  `TakerGets=5999993 drops XRP`, `TakerPays=3.039980301943390 524C`
  (quality `oq=5625558974706031031`), a tfSell crossing wanting
  `remaining_out=4000000 drops XRP`, consuming `cons_out=4000000` XRP for
  `cons_in=2.026655899060810` 524C, `owner_funds=3541511999`.
- Fork 21332270 shows the identical pattern (offer `9813D76`, 1-drop delta,
  same PrevTxnID), confirming reproducibility.

**Where it is:** the consumed amounts (`cons_out`/`cons_in`) themselves match the
network; the ±1 drop appears in the **balance transfer** that maps those amounts
to account balances (`execute_offer_trade` / `ownerGives` / the IOU↔XRP
transfer-rate rounding), i.e. a ceil-vs-floor boundary.

**Verified byte-parity with rippled (ruled OUT as the cause):** `mul_round` /
`mulRoundImpl`, `get_rate`, `Quality::increment` (`--value_`),
`quality_satisfies_threshold` (`>=`), gateway `send_max` / `multiplyRound`,
`get_book_offers` directory-quality (`quality_from_key`), `checkSeqProxy`,
retry classification, canonical ordering + salt, `LEDGER_TOTAL_PASSES=3`,
`MAX_OFFERS_TO_CONSUME=1000`, avalanche params. Earlier root fork 21331433 showed
a 7.1-XRP delta on an rLgZXam tfSell self-dealing OfferCreate (same signature,
larger magnitude).

## Fixes shipped this investigation
- `fix(consensus): only trusted UNL validator proposals influence consensus`
  (7670589f) — correct rippled `PeerImp::checkPropose` parity (no-op on this
  all-trusted testnet, but correct).
- Disk-full remediation + permanent journald cap (environmental, major stability
  win).

## Exact next step to eliminate Class B
Build a deterministic regression in `book_step_fork_repro_tests.rs` using the
captured amounts (offer `TakerGets=5999993` drops / `TakerPays=3.039980301943390`
524C; tfSell `remaining_out=4000000`; `owner_funds=3541511999`) that runs the
full `execute_book_step` + `execute_offer_trade` transfer and asserts the
resulting XRP balance delta matches the network (currently off by 1 drop). Bisect
the single ceil/floor op in the transfer (prime suspects: the XRP-side
`mul_ratio_amount` round direction, or `ceil_out_strict` round_up on the
remaining-out clip). Then correct that one rounding to match rippled's
`BookStep`/`TOffer::consume`. Validate over a multi-hour testnet run that the
fork rate drops to 0 before shipping, because a wrong rounding change regresses
every crossing.

## Diagnostics deployed (logging-only; since stripped from the code)
`FORK_STATE_DIFF` (both-sides field delta incl. PrevTxnID/OwnerNode/Indexes +
THIS_LEDGER/INHERITED tags), `CLOB_OFFER_CONSUMPTION` (per-offer crossing math,
seq-tagged, widened to all crossings), `OFFER_CREATE_RESIDUAL`, `CLOB_STOP_*`,
`consensus_txset_*`, `PROPOSAL_TRUST_TALLY`, `apply_audit`,
`record_observer_built_for_fork_diff`. Keep the `7670589f` trust-gate fix.
