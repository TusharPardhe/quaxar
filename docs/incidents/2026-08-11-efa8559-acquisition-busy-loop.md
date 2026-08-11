# efa8559 acquisition busy-loop rollback and remediation

## Status

- Production deployment of `efa8559` was rolled back to binary commit `b0ee131` on 2026-08-11.
- The source branch remains at `efa8559`; it is **not** cleared for redeployment.
- The current worktree contains the local, uncommitted remediation and its regression test. Do not push or deploy it until all validation gates below pass.

## Production evidence

The AWS node built and started `efa8559` successfully, but its post-start `fetch_info` snapshot showed a severe actor scheduling loop:

- approximately `279,573` worker jobs / scan runs in about five seconds for a single acquisition;
- approximately `282,624` scan yields;
- after another short interval, lifecycle counters reported more than six million `data_jobs_submitted`/`data_jobs_started` while only about 73 packet steps had occurred.

This disproves the no-busy-spin invariant. The node was restored to `/usr/local/bin/quaxar.pre-efa8559-20260811T102115Z`, which is the `b0ee131` binary. The active AWS binary must remain `b0ee131` until a separately authorized, evidence-backed redeployment.

## Root cause

`MissingNodeContinuation::advance` stops CPU traversal when its missing-node admission budget (`remaining`) reaches zero. It correctly retains its scan stack and pending read/network edges so that a later broker result or peer reply can resume the exact traversal.

However, `MissingNodeContinuation::has_runnable_frontier` treated any nonempty retained scan stack as immediately runnable. When `remaining == 0`, an actor turn returned `Ready` without doing work; the actor then observed the plan as runnable and self-enqueued another worker turn. This repeated indefinitely while the plan was waiting for already-admitted reads or peer nodes.

## Remediation

The runnable-frontier predicate now treats a retained scan stack as CPU-runnable only when `remaining > 0`. Deferred parent resumes and newly unannounced read/network work remain runnable regardless of the budget, because they create real work. Pending reads and pending peer edges remain passive waiting state.

A deterministic SHAMap regression test, `exhausted_missing_budget_waits_for_admitted_read_without_spinning`, constructs a budget-one continuation with multiple missing children. It proves that after the sole read is admitted:

1. the retained stack is not advertised as runnable;
2. a subsequent `Ready` advance does not manufacture another actor turn;
3. the continuation waits for the admitted result.

## Required validation before redeployment

- `cargo fmt --all`, `cargo fmt --check`, and `git diff --check`.
- Focused SHAMap continuation tests, including the new liveness test.
- App actor/mailbox and NodeReadBroker regression tests.
- `cargo check --workspace --all-targets` and the relevant package/full nextest suite.
- Independent source audit of scheduling, timeout wakeups, broker completion wakes, and budget-exhaustion transitions.
- A controlled staging/production smoke test showing bounded `worker_jobs` and `state_scan_runs` during a waiting acquisition; no millions-of-jobs burst with a static packet count.

## Deployment safety

The previously built `efa8559` binary backup is retained on AWS at:

```text
/usr/local/bin/quaxar.pre-efa8559-20260811T102115Z
```

The AWS checkout may contain `efa8559` source for diagnosis, but the active service binary is intentionally `b0ee131`.