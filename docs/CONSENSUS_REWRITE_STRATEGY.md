# Consensus & JobQueue Rewrite Strategy

## Objective

Rebuild quaxar's consensus engine and JobQueue subsystem from scratch with full,
byte-for-byte behavioral parity against rippled's C++ implementation, using
idiomatic Rust ownership and concurrency paradigms. The rebuilt system must
converge reliably under concurrent transaction load with zero forced
divergence, zero stuck nodes, and zero drift from rippled's consensus
algorithm, timing parameters, and recovery mechanisms.

## Scope

### New crate: `xrpld/consensus`

The generic, adaptor-parameterized consensus algorithm, ported directly from
rippled's `Consensus.h`, `DisputedTx.h`, `LedgerTrie.h`, `Validations.h`, and
`ConsensusParms.h`.

Modules to implement:

- `algorithm/` — the `Consensus<Adaptor>` state machine: phases (`Open`,
  `Establish`, `Accepted`), `timerEntry`, `closeLedger`, `checkLedger`,
  `updateOurPositions`, `haveConsensus`, `checkConsensus`,
  `checkConsensusReached`, `shouldCloseLedger`, mode transitions
  (`Proposing`/`Observing`/`WrongLedger`/`SwitchedLedger`), and the
  `ConsensusAdaptor` trait boundary.
- `algorithm/params.rs` — `ConsensusParms`, verified field-for-field against
  `ConsensusParms.h` (including `ledgerIdleInterval`, `ledgerMinConsensus`,
  `ledgerMaxConsensus`, `ledgerMinClose`, `ledgerAbandonConsensusFactor`,
  `ledgerAbandonConsensus`, `avMinConsensusTime`, `avalancheCutoffs`,
  `avCtConsensusPct`, `avMinRounds`, `avStalledRounds`).
- `model/proposal.rs` — `ConsensusProposal`, `SEQ_JOIN`/`SEQ_LEAVE` sentinels,
  staleness checks.
- `model/disputed_tx.rs` — avalanche dispute voting (`DisputedTx::updateVote`,
  weight calculation, avalanche state transitions).
- `model/ledger_trie.rs` — `LedgerTrie`, branch support tracking, `getPreferred`.
- `rcl_support/` — the RCL (Ripple Consensus Ledger) adaptation layer:
  `RclConsensusAdapter` trait, `Validations`/`getPreferredLCL`, proposal/
  suppression hashing.

### App-level integration: `xrpld/app/src/consensus/`

- The concrete adaptor implementing `RclConsensusAdapter` against quaxar's
  ledger master, validations store, transaction queue, and overlay.
- `doAccept`/`endConsensus` equivalents, ported from `RCLConsensus.cpp`,
  including the exact `JtAccept`/`"AcceptLedger"` JobQueue dispatch pattern
  so ledger construction never blocks the consensus timer thread.
- `checkLastClosedLedger`/`NetworkOPs` fork-detection and recovery, including
  peer-count majority fallback when trusted-validation preference is
  inconclusive.

### JobQueue: `xrpld/app/src/job/`

- A genuine persistent worker-thread pool, matching rippled's `JobQueue`
  design: N worker threads blocked on a condition variable, continuously
  draining queued jobs, started at application boot and joined at shutdown.
- Job priority/type accounting matching `JobTypes.h`.

## Design principles for the rewrite

1. **Line-for-line parity first, idiomatic Rust second.** Every timing
   constant, phase transition, and recovery path must be traceable to a
   specific line in rippled's C++ source before being considered complete.
   Deviations are only acceptable where Rust's ownership model requires a
   structurally different (but behaviorally identical) approach.
2. **No polling loops standing in for real concurrency primitives.** Timer
   ticks, proposal draining, and job dispatch must use real OS threads,
   channels, and condition variables — not busy-loops or single-shot
   dispatch-and-hope patterns.
3. **Single-owner state.** Each piece of mutable consensus state (current
   phase, peer positions, disputes, pending round parameters) must have
   exactly one thread with write access at any time, enforced by the type
   system (ownership/borrowing), not by convention.
4. **No silent proposal drops.** Every code path that can reject an incoming
   peer proposal must log why, at a level that survives production log
   filtering, and must be verified against rippled's equivalent rejection
   path (staleness, sequence, ledger mismatch, bow-out).
5. **Explicit thread and task ownership diagram.** Before writing code,
   document which thread/task owns the consensus mutex, which owns the
   JobQueue, and how they hand off — this must be drawn out and reviewed
   before implementation begins, not discovered afterward.

## Build order

1. Rewrite `ConsensusParms` and verify every constant against
   `ConsensusParms.h` with a dedicated reference-vector test.
2. Rewrite `ConsensusProposal`, `DisputedTx`, `LedgerTrie` as pure,
   dependency-free data structures with unit tests ported from rippled's
   own test suite where available.
3. Rewrite the `Consensus<Adaptor>` state machine against a mock adaptor,
   with unit tests covering every phase transition and timeout path.
4. Rewrite the JobQueue with a real worker-thread pool; unit test job
   ordering, concurrency limits, and shutdown behavior in isolation.
5. Rewrite the RCL adaptation layer (`RclConsensusAdapter`, `Validations`,
   `getPreferredLCL`) against the now-tested algorithm core.
6. Rewrite the app-level adaptor (`do_accept`, `end_consensus`,
   `checkLastClosedLedger`) and wire it to the new JobQueue.
7. Rewrite the bootstrap/driver integration (proposal draining, timer
   dispatch, validation acceptance) using the documented thread-ownership
   diagram from the design principles above.
8. Only after unit coverage is green: deploy to the standalone 5-node
   Docker Compose network (no Kurtosis dependency) and run the full stress
   test suite (concurrent multi-account transaction bursts, sustained load,
   idle baseline) until multiple consecutive clean runs are achieved.
9. Cross-validate against a live 5-node rippled Kurtosis network under the
   identical stress pattern as the acceptance bar — quaxar must match
   rippled's convergence behavior exactly under identical conditions.

## Acceptance criteria

- Zero hash divergence across 3 consecutive full stress-test runs.
- Zero nodes stuck (validated-ledger age not advancing) for more than the
  idle interval under any tested load pattern.
- Consensus round times remain stable and comparable to rippled's under
  identical concurrent load, with no round-to-round timing collapse.
- Every consensus parameter matches `ConsensusParms.h` exactly, verified by
  an automated test, not manual inspection.
- Full existing test suite (transaction engine, RPC, overlay, ledger
  storage) continues to pass unmodified, confirming the rewrite did not
  regress unrelated subsystems.
