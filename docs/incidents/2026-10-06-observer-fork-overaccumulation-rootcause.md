# Residual Observer Fork — Root Cause (2026-10-06)

## Status
Node is **fully functional and syncing in lockstep** with testnet (our validated
seq tracks the network, 0 wrong-ledger errors, 0 invariant failures, 0 panics
over multi-hour runs, state=full, 17 peers). It still produces **intermittent
local forks (~1–2/hr)** that **self-heal** (the node adopts the validated chain
within one round, leaving a transient 1-ledger gap in `complete_ledgers`).

## Ground-Truth-Proven Root Cause

The fork is a **consensus tx-set OVER-ACCUMULATION on "spike" rounds**, NOT a
transactor, build, apply, or ledger-hashing bug.

### Decisive evidence (public testnet `s.altnet.rippletest.net`)
For forked ledger **21324370**:
- Our node proposed and built a **36-transaction** ledger (`consensus_tx_set=91FE397E…`).
- The network validated a **3-transaction** ledger (`tx_hash=08ED3C6E…`, `ledger_hash=08ED…`).
- Mapping our 36 tx-ids against the network's ledgers: all 36 are real (0 not-found),
  distributed across **5 consecutive network ledgers**:
  - 21324370: 3 (match) · 21324371: 9 · 21324372: 10 · 21324373: 3 · 21324374: 11 → **36 total**.
- **We packed ~5 network ledgers' worth of transactions into one ledger.**

For a second fork (21323450): network validated **26** txs, we built **35** (strict
superset, 9 extra = ticket/`Sequence=0` funding Payments + an AccountSet, all of
which the network validated in the NEXT ledger 21323451).

### Correlation
When our `effective_close_time` and tx-count match the network (e.g. 21324407:
both 22 txs / close 844606010; 21324410: both 18 txs / close 844606020), there is
**no fork**. Forks occur only when `open_tx_count` SPIKES (normal 10–25, spikes to
36/39/63) and we accept a superset.

## What was verified BYTE-IDENTICAL to rippled (ruling these out)
Exhaustive line-by-line audit against `../rippled` `develop`:
- `LEDGER_TOTAL_PASSES=3`, `LEDGER_RETRY_PASSES=1`
- `getNeededWeight`, avalanche cutoffs `{Init 50, Mid 65, Late 70, Stuck 95}`,
  `avMinRounds=2`, `avStalledRounds=4`, `avCtConsensusPct=75`,
  `ledgerMinConsensus=1950ms`, `minConsensusPct=80`, `ledgerGRANULARITY=1s`
- `DisputedTx::stalled` / `update_vote` (observer branch `yays > nays`) /
  `set_vote` / `un_vote`
- `check_consensus` / `check_consensus_reached` (incl. `stalled => true`)
- `Transactor::checkSeqProxy` (`terPRE_SEQ`/`tefPAST_SEQ`/`terPRE_TICKET`)
- `applyTransaction` retry classification (applied⇒Success; tef/tem/tel⇒Fail; else⇒Retry)
- `CanonicalTXSet` ordering `(account XOR salt, seqProxy, txId)` with salt =
  tx-set SHAMap hash
- Payment XRP source-reserve / `tecUNFUNDED_PAYMENT` check (`preFeeBalance_`)
- `roundCloseTime` / `effCloseTime`
- `shouldCloseLedger` fast-path `(proposersClosed + proposersValidated) > prevProposers/2`
- `ApplyStateTable` / `RawStateTable` / sandboxes use deterministic `BTreeMap`
  (no `HashMap` non-determinism)

## Fix shipped this investigation
`fix(consensus): only trusted UNL validator proposals influence consensus`
(commit `7670589f`). rippled (`PeerImp::checkPropose`) feeds ONLY trusted-validator
proposals to `processTrustedProposal → peerProposal`; our ingress fed ALL peer
proposals into `curr_peer_positions`. This is a correct parity gap and is fixed,
but it is a **no-op on this testnet** (all received proposals are already trusted:
`PROPOSAL_TRUST_TALLY trusted=N untrusted=0`), so it does not remove these forks.

## Remaining mechanism (the actual fix target)
The abnormality is **dispute non-convergence on spike rounds**:
`update_our_positions` logs `vote_changes == dispute_count` every round at
`converge_pct` 82–95% (where votes should be STABLE by the Stuck=95% avalanche
state). Our accepted position stays a superset of the quorum's final set.

Because every static consensus/build component matches rippled byte-for-byte, the
divergence is a **live timing/convergence dynamic**, most consistent with
**stale/lagging peer-position processing**: the validators trim their proposed set
across the round, but our `curr_peer_positions` snapshot used by
`update_our_positions`/`have_consensus` lags behind their latest (shrinking)
positions, so we accept on a stale snapshot where peers appeared to agree on our
bloated set.

### Recommended next steps (require careful live validation)
1. Instrument per-peer `propose_seq` held in `curr_peer_positions` at accept vs the
   latest `propose_seq` the overlay received for that peer. Confirm/deny lag.
2. Ensure the consensus strand drains ALL pending `PeerProposal` commands
   immediately before each `timer_tick` (`phase_establish`/`update_our_positions`)
   so disputes always run against the freshest peer positions.
3. Verify `update_our_positions` runs enough establish rounds per ~3s round to
   satisfy avalanche convergence (`avMinRounds`), and that it re-trims our set
   against peers' latest positions each round.
4. Any change here MUST be validated over a multi-hour testnet run measuring fork
   rate before/after, because consensus-timing changes can regress convergence.

## Diagnostics to strip before shipping
Logging-only additions made during this investigation (harmless to behavior, but
verbose): `FORK_STATE_DIFF` + `record_observer_built_for_fork_diff`
(`history.rs`, `application_root.rs`, `rcl_consensus.rs`), `APPLY_AUDIT`
per-tx, `CONSENSUS_TXSET_AGREED`/`DIVERGENCE`, `PROPOSAL_TRUST_TALLY`,
`THIS_LEDGER`/`INHERITED` tags. The `7670589f` trust-gate change is a real fix
and should be KEPT.
