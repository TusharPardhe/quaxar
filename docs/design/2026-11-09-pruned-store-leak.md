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
