# Pruned store leak: skipped claims and untracked writes (2026-11-09)

## Symptom

On the mainnet validator (`online_delete = 512`), the live data in the fjall
store grew by about 1.4 GB a day while the retained window stayed at 512
ledgers. The Nov 6 store rebuild did not stop it, because it copied every
key.

## Measurements (read-only census of a hard-linked snapshot, 2026-11-09 05:00 UTC)

| | Entries |
|---|---|
| Total `nodes` | 74.5M: 44.6M inner, 23.9M state leaves, 6.0M transaction leaves, 87k headers |
| Current claimed state tree (full walk) | 28.7M (8.46M inner, 20.26M leaves), 0 missing |
| Dead-pending in the window (count-0 rows) | 1.40M |
| Owned (transaction-tree) records of window ledgers | 0.12M |
| **Expected** | **≈ 30.3M** |
| **Unreachable and untracked** | **≈ 44M (59%)** |

- Window bookkeeping: only 444 of the 513 window sequences had owned records,
  so 69 validated ledgers (13%) were never claimed, in 57 gaps of 1-4.
- No claim failures were logged, so every gap was a skip.
- Samples: 117 of 120 transaction leaves belonged to ledgers below the window.
  Unreachable state leaves were validated mainnet versions with no count row
  and no notebook record; 28 of 38 had existed for exactly one ledger.
- `pruned_unclaimed` was 0.

## Cause

1. `PrunedDriver::submit` kept only the newest validated ledger. When a claim
   ran past the next ledger, the waiting one was replaced, so its claim was
   skipped.
2. The next claim diffs the previous claimed state against the new one
   (N-1 against N+1). A node created in N and replaced by N+1 is in neither
   tree, so it never gets a count or a death record. N's transaction tree is
   never claimed as owned either.
3. The orphan sweep retires nodes tracked as unclaimed, but only
   `PrunedStore::store` recorded them. Ledger building and acquisition write
   through `DatabaseNodeImp::store_batch` straight to the backend, so nothing
   was tracked.

Ledgers this node built but the network did not validate (`MovedOn`, lost
rounds) leaked the same way.

rippled has no equivalent: it keeps no per-node references and rotates whole
databases on `online_delete`.

## Fix

- **Claim every validated ledger in order.** `PendingClaims` is a FIFO of up
  to 32 ledgers (about two minutes). Only a full queue drops the oldest.
- **Track fresh writes.** Once the store is anchored, the fjall backend
  records each node that `store` / `store_batch_result` inserts while it is
  absent (`contains_key` before the insert). At each claim, under the writer
  lock and before the claim, `PrunedStore::claim` notes these as unclaimed
  at the claim's sequence. The claim removes the ones it adopts (`new_state`,
  `owned`). The orphan sweep retires the rest after `online_delete` ledgers,
  and prune deletes them `online_delete` ledgers later.

**IMPORTANT, safety:** marking a live node unclaimed would get it deleted.

| Case | Why it is safe |
|---|---|
| Node already present when written (live, or dead-pending) | `contains_key` is true, so it is not recorded. Dead-pending re-stores stay covered by `restore_watch`. |
| Record reaches the index after the claim that adopted the node (write and claim race, for example through `restore_missing`) | `IndexWriter::recent_claimed` keeps the adopted sets of the last 8 claims, and those records are skipped |
| Writes before the anchor (initial sync) | Tracking is enabled only on an anchored store, and anchoring discards pending records: every node present at the anchor is implicitly live |
| `online_delete = 0` (full history) | No pruned store is built, and tracking is guarded on `online_delete > 0` |
| More than 4M records between two claims | Further records are dropped. A dropped record can only leave a node unpruned. |

Unclaimed tracking is in memory only, so nodes written before a restart and
never claimed still leak. Ledger headers are not claimed, so they are now
deleted about two windows after they are written, outside the window.

## Follow-up (2026-11-17): the unclaimed list survives restarts

Each restart leaked the ~100k-170k nodes on the in-memory unclaimed list
(about 50 MB). The list is now also kept in an `unclaimed` keyspace
(`hash -> first_seen_seq`, `u32` LE):

- **Memory stays the working copy.** Claims, the orphan sweep and prune read
  only the in-memory map. The keyspace is read once, in `IndexWriter::open`,
  before anything can claim, sweep or prune.
- **Changes land in the batch that causes them.** Every change goes through
  `pin_unclaimed` / `unpin_unclaimed`, which mark the hash dirty. The claim,
  orphan-sweep, prune and anchor batches write the dirty hashes whose wanted
  state differs from the last written state. A node pinned and adopted
  between two batches writes nothing, so a claim writes only the nodes it
  leaves unclaimed. A failed batch keeps the hashes dirty for the next one.
- **Cap.** Fresh-write tracking stops adding entries at 2M (mainnet runs at
  100k-170k). Pins of dead nodes that an in-flight ledger needs are never
  capped.

**IMPORTANT, safety:** a wrongly loaded row makes the orphan sweep retire a
live node. Counts cannot tell "claimed once" from "never claimed" (both have
no row), so a row is trusted only when nothing can have adopted its node
behind the keyspace's back:

| Case | Handling |
|---|---|
| A claim ran without maintaining the keyspace (rollback to an older binary, then forward) | Every claim batch writes `meta/unclaimed_seq = seq`. At open, rows load only if `unclaimed_seq == claimed_seq`; otherwise all are deleted. A discarded row can only leave a node unpruned. |
| Row for a node with an explicit live count, or an unreadable count row | Cannot be shown unclaimed: dropped at open (the open still succeeds) |
| Malformed row (key not 32 bytes, value not 4 bytes) | Dropped at open |
| Store not yet anchored | No row is trusted; the anchor clears the keyspace |

Peers cannot write the keyspace. They can only cause fresh writes of
hash-verified nodes, which the cap bounds. Write access to the store
directory is outside this guarantee, as for the other keyspaces; the
periodic verify still halts pruning on a missing required node.

Tests: `unclaimed_node_is_pruned_after_a_restart` and
`orphan_noted_before_a_restart_is_pruned` (real fjall close and reopen) fail
without the load; `saved_rows_are_discarded_when_a_claim_ran_without_them`
fails without the `unclaimed_seq` check. Also
`node_adopted_after_its_row_was_saved_survives_a_restart`,
`saved_row_of_a_node_with_a_live_count_is_dropped`,
`saved_row_with_a_corrupt_count_row_is_dropped_and_the_store_opens`,
`malformed_saved_rows_are_dropped`, `anchor_starts_the_saved_list_empty`,
`node_adopted_by_the_next_claim_writes_no_row`,
`failed_claim_keeps_unclaimed_changes_for_the_retry`.

## Tests

- `node_hidden_by_a_claim_gap_is_pruned`
- `live_node_recorded_after_its_claim_is_never_pruned`: fails without the
  recent-claims guard
- `rewriting_a_stored_node_is_not_tracked`
- `writes_before_the_anchor_are_not_tracked`
- `skipped_claims_prune_hidden_nodes_and_keep_live_trees`: real ledgers
  written through `store_batch`, every third claim skipped. The latest
  claimed trees stay complete and a never-claimed root is pruned.
- `pending_claims_keep_every_ledger_in_order`

## Not done

The ~44M nodes already leaked are not tracked and stay in the store. Removing
them needs an offline copy of only the window's reachable nodes, or a resync.
