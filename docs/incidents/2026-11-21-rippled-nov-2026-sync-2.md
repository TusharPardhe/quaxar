# rippled develop sync — November 2026, part 2

**Date:** 2026-11-21
**Branch:** `sync/rippled-nov-2026-2`
**Upstream range:** `XRPLF/rippled` `develop` `58b73715c7..82217fbd0a` (11 commits,
Nov 11 – Nov 20, after the November sync).

IMPORTANT: none of these changes alters ledger results. No amendment, transactor
result or wire format changed upstream in this range.

## Ported

| rippled | Title | Area | Notes |
|---|---|---|---|
| #8662 `8ab065668a` | Reset a ledger's save state when its database write fails | ledger persistence | See below. |
| #8660 `ec7e0a38a8` | Count key-only cache hits, misses and removals | caches | An expired key-only entry now counts as a cache removal and a map removal in `sweep_key_partition`, so the per-partition sweep log reports both, as rippled. quaxar's `KeyCache` keeps no hit/miss or `cacheCount` counters and reports its size from the map, which is what rippled now converges to; nothing else to change. |

### #8662 in detail

rippled now handles a failed relational write in `saveValidatedLedger` by
clearing the sequence from the complete range, then finishing the pending save,
then clearing the SAVED flag on the ledger hash (in that order, so a concurrent
enqueue cannot re-arm the flag and dedupe the retry away). It adds
`HashRouter::clearFlags`.

quaxar changes:

- `HashRouter::clear_flags` and `HashRouterEntry::clear_flags` (plus `!` on
  `HashRouterFlags`). Clearing never creates or refreshes an entry.
- The SAVED flag now lives in the application's shared `HashRouter`, as in
  rippled, instead of a private `HashSet` in `AppLedgerPersistenceRuntime`.
  It therefore expires with the router's hold time (300 s by default) like
  rippled's. The old set was never pruned and grew by one hash per validated
  ledger (about 21k a day).
- `LedgerPersistenceRuntime::save_validated_ledger` returns a
  `ValidatedLedgerSave`, so the two failures rippled tells apart are handled
  the way rippled handles them:

| Outcome | rippled | quaxar before | quaxar now |
|---|---|---|---|
| Accepted ledger has missing nodes | `failedSave` (clear range + reacquire) on every path; SAVED stays set | `failed_save` only for queued saves; SAVED cleared | `failed_save` on every path; SAVED stays set |
| Database write fails | clear range, finish work, clear SAVED, rethrow | queued: `failed_save` (clear + reacquire); sync: nothing; SAVED cleared before finish work | clear range, finish work, clear SAVED, return `false` |

**Deliberate difference:** rippled rethrows the database error. From a job
that ends the process, and on the synchronous path it unwinds into the caller.
quaxar returns `false` and logs instead, and keeps running with the same
state rippled leaves behind before the throw.

## Not ported

| rippled | Why |
|---|---|
| #8627 Batch preflight without allocations | C++ performance. The duplicate-sequence/ticket rule is unchanged: sequences and tickets share one key space per account, and exactly one of them is present on an inner transaction. quaxar's `validate_batch_preflight` already uses one set per account for both. |
| #8628 log levels | The re-levelled messages (CheckCash preflight/preclaim, `Transaction is obsolete`, `processLedgerRequest`, `Need validated ledger for preferred ledger analysis`, `Thread Tx`) have no equivalent log line in quaxar. |
| #8334 config directory search | Refactor with the same search order and fallback. quaxar does not search rippled's candidate directories: it uses `--conf`, with fixed fallbacks per command (`./quaxar.cfg` for the CLI's RPC port lookup, `/etc/quaxar/quaxar.cfg` for snapshot loading). Unchanged, pre-existing difference. |
| #8486 `json::Value` from `string_view`, #8487 JSON-RPC version constants | C++ refactors with no behaviour change. |
| #8737, #8721, #8738, #8740 | Docs, build and CI. |

## Testing

- New tests:
  - `hash_router::tests::clear_flags_clears_only_the_named_flags` and
    `clear_flags_reports_when_nothing_changes` (ported from rippled's new gtests).
  - `persistence::failed_write_releases_the_save_state_so_a_retry_runs` and
    `synchronous_save_with_missing_nodes_calls_failed_save`. Both fail with the
    previous save-path behaviour restored.
  - `application_root_shares_one_persistence_runtime_with_real_dedup_state_across_calls`
    now also checks that SAVED is in the shared router and that clearing it
    lets the next save run. It caught the startup runtime not using the shared
    router before storage attach.
  - `key_partition_sweep_counts_expired_entries_as_cache_and_map_removals`.
- Full workspace suite: see the PR. Fork-capture replays were not rerun: no
  transaction-processing code changed.
