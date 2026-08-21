# Mainnet LCL operating-mode oscillation investigation

## Status and safety

- Baseline: `be7ef87793bfd9c3353530a3db282105bbc8db8d`.
- Working branch: `fix/lcl-oscillation-be7ef877`.
- Protected source branch: `pr/preferred-lcl-mode-promotion` (unchanged).
- Backup: `backup/pr-preferred-lcl-mode-promotion-5399200-20260821-105531`.
- No deployment, service restart, database operation, or configuration change was performed.

## State transitions

```text
peers < minimum --------------------------------------------> Disconnected
Disconnected + enough peers -- request Connected ----------> Connected or Syncing*
Connected + validated age < 60s ----------------------------> Syncing
Syncing + validated age >= 60s -----------------------------> Connected
Connected/Syncing + endConsensus + no LCL change
  + !needNetworkLedger -------------------------------------> Tracking
Connected/Tracking + endConsensus + no LCL change
  + fresh current-open parent ------------------------------> Full
Tracking/Full + actionable preferred-LCL divergence --------> Connected or Syncing*
Full + accepted round with zero peer positions --------------> Connected (direct)
any mode above Connected + UNL/amendment blocked ------------> Connected

* `setMode(Connected)` is normalized to Syncing while validated-ledger age is
  under 60 seconds. `setMode(Syncing)` is normalized to Connected when it is
  60 seconds or older.
```

The observed `Connected -> Syncing -> Tracking -> Full` rise is therefore the
normal normalization/promotion chain. Repetition requires a later demotion,
and the incident path supplies it through unresolved preferred-LCL divergence.

## Root cause

The inbound completion owner persisted a completed canonical ledger, published
it to the resolver/validation adaptor, ran `checkAccept`, and acknowledged the
ready item. It did not install that ledger as the closed LCL when the consensus
runner was already `WrongLedger` in `Open` phase.

The decisive condition was:

- `network_ops_strand.rs::should_reconcile_preferred_lcl` allowed installation
  only in `ConsensusPhase::Accepted`;
- the completion path logged `reconcile_deferred_after_completion` for every
  non-Accepted completion;
- `Consensus::check_ledger` could then recompute a newer preferred hash on the
  next timer tick;
- `ApplicationRoot::check_accept_ledger` intentionally does not mutate the
  closed LCL, rebuild the open ledger, or restart consensus.

Thus an acquired target could become validated/cache-visible without receiving
the `switchLastClosedLedger` postconditions. If the network head advanced faster
than recovery, the runner repeatedly replaced its target and the local closed
slot remained on the divergent chain. Preferred divergence demoted Full or
Tracking, while end-consensus freshness later promoted the node again, producing
the mode oscillation.

## Fix

Immediately after the bounded completion handoff, the NetworkOps strand now:

1. checks the existing runner mode and existing `prev_ledger_id` target;
2. takes the existing reentrant LCL transition gate;
3. accepts only a durable, complete, exact-hash, current, compatible candidate;
4. runs the existing `switch_last_closed_ledger` path before another timer tick.

That path clears `needNetworkLedger`, processes TxQ, rebuilds the open ledger,
installs the closed LCL, runs post-switch `checkAccept`, broadcasts the switched
status, and starts exactly one round. No second preferred-target state or second
validation loop was added.

The transition trace records the exact target/sequence, completion count,
transition cause, strand owner, and LCL-gate wait time.

## Lock, strand, and queue audit

- Consensus runner: exclusively owned by `networkops-strand`; there is no
  consensus-state mutex. Proposal, tx-set, command, and completion drains are
  bounded per turn.
- LCL transition: `parking_lot::ReentrantMutex<()>`; the recovery install now
  uses the same outer gate as Accepted-phase reconciliation and records wait
  microseconds. Reentrancy is required because ApplicationRoot postconditions
  defend their public entry points with the same gate.
- Inbound registry: `Mutex<RegistryInner>`. Completion recording inserts the
  ledger before enqueuing `(hash, acquisition_id)`. Polling clones ready ledgers
  under the mutex, then releases it before persistence, validation, and switching.
  Acknowledgement reacquires it briefly and completed entries remain resident
  until ordinary sweep.
- Completion receiver: wakeup only. The registry's bounded ready queue is the
  authoritative handoff, so a full/disconnected receiver cannot lose the result.
- Validation state: its mutex is released before `check_accept`, explicitly
  avoiding recursive validation lookup deadlock.
- Open/closed transition: the LCL gate precedes the close gate where both are
  needed; no reverse close-gate -> LCL-gate path was found in the audited flow.

Conclusion: the deterministic reproduction proves a policy/handoff gap, not
lock contention. Production contention is not fully exonerated because no
matching runtime report existed under `/tmp` and the server was unreachable.
The new trace provides the required gate wait measurement on the repaired path.

The separate strand-panic hypothesis is not proven by incident logs. Source
inspection shows that the strand thread is spawned without `catch_unwind`, and
`stop()` discards the `JoinHandle::join` error. A panic could therefore terminate
the owner without a dedicated fatal lifecycle event. This remains follow-up
instrumentation; it is not needed to reproduce or fix the target-chase defect.

## Regression

`completed_wrong_ledger_target_is_installed_before_advancing_preference` starts
with a divergent local LCL, holds the first target at the durability fence,
advances network preference to a newer hash, releases the first completion, and
proves that the first exact runner target becomes both the closed LCL and the
open ledger parent. It also proves `needNetworkLedger` clears and only one new
round starts.

## Verification

- `cargo fmt --check`: passed.
- Focused regression: passed (`1 passed`, `500 filtered out`).
- `cargo check -p app`: passed with existing warnings.
- `git diff --check`: passed.
- `cargo test -p app --lib`: compiled and ran; `478 passed`, `23 failed`.
  Failures are baseline failures in bootstrap routing, acquisition/read-broker,
  replay, NetworkOps batch scheduling, and older provisional fixtures. The new
  regression is not among them.
- Requested `cargo build --release -p xrpld`: cannot run because no package is
  named `xrpld` in this workspace.
- Correct workspace build `cargo build --release -p xrpld-main`: passed and
  produced `target/release/quaxar`.

## Deployment blocker and plan

Read-only discovery against `ubuntu@16.60.183.163` timed out on TCP port 22.
Consequently the actual service unit, active executable, old binary hash, and
an exact rollback command are unknown. They must not be guessed.

After SSH access is restored:

1. run the mandated process/unit/executable discovery;
2. resolve the active PID's executable with `/proc/<pid>/exe` and confirm its
   systemd `ExecStart`, config argument, and runtime directory;
3. record old/new SHA-256 hashes;
4. copy the active executable to a timestamped sibling rollback file;
5. provide the literal rollback command for that discovered path and unit;
6. only after explicit approval, replace that executable and restart that unit;
7. verify RPC/process/peer/ledger/LCL health and stable mode for at least 30
   minutes, without touching `xrpld.cfg` or any runtime/database state.

Deployment is blocked until the 23 baseline test failures are dispositioned and
the discovery step succeeds.
