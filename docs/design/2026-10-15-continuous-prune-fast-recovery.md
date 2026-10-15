# Continuous pruning with fast fork recovery

Status: implemented on branch `perf/trusted-baseline-restart` (`abd8e22c`),
deployed to the second testnet node. Replaces the batch pruning of `02a3a515`.

## 1. Problem

The pruned fjall store deletes dead nodes as the `online_delete` window
slides. Every delete must clear the FullBelowCache, because a "subtree
complete" marker may cover a node that was just deleted. With deletes on
every validated ledger (~4 s), any acquisition whose tree walk takes longer
than one ledger loses its progress before it finishes. During the
2026-10-14/15 fork at 21540882 that produced 6–26 minute stalls and RSS
spikes to 3–3.7 GB: every restarted walk reloaded millions of tree nodes.

`02a3a515` fixed it the rippled way, deleting in batches every
`online_delete` ledgers. The cost is that the stored window swings between N
and 2N ledgers. This design keeps an exact, continuously sliding window of N
and still lets recovery converge.

## 2. Idea

```
         FullBelowCache (notes)                 trusted baseline
         ───────────────────────                ─────────────────
 truth   "subtree was complete when noted"      "subtree belongs to the newest
                                                 claimed (validated) ledger"
 cleared on every prune                         rolled forward on every claim
 → useless across deletes                       → valid across deletes
```

The newest claimed ledger's tree is complete on disk: the claim restores any
missing node and syncs the batch before it is published. None of its nodes can
be deleted for `online_delete` ledgers, because they die no earlier than the
next claim and are deleted N ledgers after they die. So an acquisition can
skip any subtree whose hash equals the baseline's subtree at the same
position, however often the FullBelowCache is cleared.

```
fork recovery: download network ledger 101 while we hold 100

        [R101]                       baseline = claimed ledger 100
       /   |   \
    [A]   [B]   [C']   A, B equal baseline hashes  → skipped, no reads
    skip  skip   |      C' differs                 → walked, small
               [c1']                               → fetched from peers
```

## 3. Changes

| file | change |
|---|---|
| `nodestore/src/pruned/store.rs` | batch gate removed: prune on every ledger again; `fetch_unwatched`, `online_delete()` |
| `nodestore/src/backends/restore_watch.rs` | new: watch states Watching → Restored / Deleting → Rescued; covers reads and writes |
| `nodestore/src/backends/fjall_backend.rs` | `fetch` / `store` / `store_batch` go through the watch |
| `nodestore/src/backends/backend.rs` | `begin_delete` / `finish_delete` on the backend trait |
| `nodestore/src/pruned/index.rs` | prune marks candidates Deleting, deletes, writes back rescued nodes and pins them as unclaimed |
| `app/src/shamap/pruned_driver.rs` | `set_on_claimed(seq, root)`, fired after the synced claim and before pruning |
| `app/.../trusted_baseline.rs` | `advance()` rolls the baseline; `max_age = online_delete / 2` |
| `app/src/bootstrap/bootstrap.rs` | install at start and register the rolling callback |

## 4. Edge cases

| # | case | handling |
|---|---|---|
| 1 | Acquisition reads a dead-pending node and attaches it; the node is then deleted, leaving a hole under a ledger that is not yet claimed | Fixed. A node-store read of a watched node marks it Restored; prune keeps it and pins it as unclaimed. Test `dead_node_read_by_node_store_is_kept_by_prune` fails without the hook. |
| 2 | A read or write lands while prune's delete batch is committing | Prune moves candidates to Deleting first. A racing read or write stores the bytes in Rescued, and prune writes them back after the commit. The read holds the watch lock across the store read, so no interleaving is lost. Test `read_racing_the_delete_batch_is_rescued`. |
| 3 | A rescued or read-pinned node is never claimed | It is pinned as unclaimed. The orphan sweep retires it after one window and it is deleted N ledgers later, so retention stays bounded. Covered by the second half of the read-pin test. |
| 4 | Baseline lookups pinning nodes | Baseline reads use `fetch_unwatched` (kv path), never the watched fetch. |
| 5 | Baseline trusted after its nodes become deletable | `max_age = online_delete / 2`. The baseline normally rolls every ledger. If claims stall (worker behind, store halted), the age check retires it and walks fall back to the FullBelowCache plus reads. |
| 6 | Callback fires before the tree is on disk | It fires only after `claim()` returns, after the batch is written with `SyncAll` and `restore_missing` has run. Test `on_claimed_publishes_a_tree_that_is_complete_on_disk` walks the tree from the store inside the callback while pruning runs every ledger. |
| 7 | In-flight acquisition holds the previous baseline | `advance` retires it: `has_complete_child` returns false and the memo is dropped. Worst case is a few extra reads, never a wrong skip. Test `advance_replaces_and_retires_previous_baseline`. |
| 8 | Anchor ledger (first claim on a fresh store) | The anchor is not a diff claim and does not fire `on_claimed`. The baseline appears from the first real claim (or the restart install). |
| 9 | Target on a fork | Equal hash at the same position means an identical subtree, so it is still correct; only the differing part is walked. |
| 10 | Transaction trees | The baseline is state-tree only. Tx trees are per-ledger and small; the FullBelowCache still serves them. |
| 11 | Pruning halted after a verify miss | Halted prune deletes nothing, so the baseline stays valid. Claims continue, so it keeps rolling. |
| 12 | Kill switch | `QUAXAR_TRUSTED_RESTART=0` disables the startup baseline. The rolling callback is registered inside the same function, so it is disabled too. |

## 5. Trade-offs

- Not rippled's model: rippled rotates whole databases and has neither
  reference counts nor a baseline. This is quaxar-specific and exists because
  quaxar's store can delete one node at a time.
- Consensus-neutral: only which local reads are skipped changes, never ledger
  contents.
- Memory: the baseline memo holds only baseline inner nodes on diverging
  paths and is released on every roll.
