# Trusted committed baseline for restart catch-up

Status: implemented on branch `perf/trusted-baseline-restart` (not merged).
Results: see section 8.

## 1. Problem

On a hot restart (existing fjall store) quaxar walks the whole testnet state
tree before it can reach `full`. Measured on testnet (2026-10-10/11, six
restarts, `~/hot-restart/probe-*.jsonl` on the server):

| metric | value |
|---|---|
| time to `full` | 470–616 s (median ≈ 500 s) |
| NodeStore reads before `full` | 20–28 M |
| NodeStore writes before `full` | 3.9–12 M |
| peak jemalloc resident | 13.5–14.2 GiB |
| tree nodes resident at peak | ≈ 30 M |

The work is redundant. The pruned store already guarantees that the last
claimed ledger's tree is complete on disk, but acquisition cannot use that
fact: "hash present in the store" does not imply "subtree complete" (nodes
from interrupted acquisitions and consensus builds are written top-down and
may be orphans with missing children), so every restart re-proves the tree
node by node and keeps it resident while it does.

## 2. Research basis

The SHAMap is an immutable, copy-on-write, hash-addressed radix tree with a
new root per ledger. Three established results apply directly.

1. **Shadow paging / copy-on-write trees.** Rodeh, *B-trees, Shadowing, and
   Clones*, ACM TOS 2008; the ZFS uberblock and btrfs superblock designs.
   A tree whose nodes are never modified in place is committed by durably
   recording its root after its nodes; on restart the system *mounts* the
   last committed root in O(1) instead of re-verifying the tree.
2. **Ordered durability.** Ganger et al., *Soft Updates*, ACM TOCS 2000.
   If a pointer is never persisted before its target, a persisted committed
   root implies every node reachable from it is persisted.
3. **Merkle anti-entropy.** Merkle, CRYPTO 1987; DeCandia et al., *Dynamo*,
   SOSP 2007. Two hash trees are reconciled top-down, skipping any subtree
   whose hashes match; cost is proportional to the difference times the depth,
   not to the size of the state.

Applied together: treat the pruned store's last claimed root as a committed
snapshot (1, 2) and reconcile the network's ledgers against it (3).

## 3. Why the claimed root is a committed, complete tree

From `xrpld/nodestore/src/pruned` and `xrpld/app/src/shamap/pruned_driver.rs`:

- `claim(L)` diffs L against the previous claimed root, **restores every node
  of L missing from the store** (`restore_missing`, design rule R3), applies
  counts and records `claimed_seq`/`claimed_state_root` in one batch, and
  persists with `SyncAll`. The anchor (first claim on a fresh store) is the
  first validated ledger of a completed sync.
- `prune(K)` deletes only nodes whose count is 0 and whose death sequence is
  ≤ K = validated − `online_delete`. Every node of the latest claimed tree has
  count ≥ 1.
- A node's ancestors change whenever it changes, so `died(ancestor) ≤
  died(descendant)`; a present live node therefore has a present subtree.

**Invariant B.** At process start, every node reachable from
`claimed_state_root` is in the store. Nodes of that tree die only when newer
ledgers are claimed and are deleted no earlier than `online_delete` (512)
ledgers after that.

## 4. Design

```
startup ── read claimed (seq, root) ── install TrustedBaseline
                                            │
 acquisition traversal of target ledger T   │
   at inner node T_P (position P), child b: │
     hash(T_P.b) == hash(B_P.b) ?  ─────────┘ B_P = baseline node at P
        yes → subtree identical & complete on disk → skip (no read)
        no  → descend as today (read / network), and B_{P.b} is loaded
                                              lazily only on this path
 retire when: node is Full, or validated > baseline_seq + 256
```

- **Position-aligned comparison.** Equal hash ⇒ identical content-addressed
  subtree. Comparing at the same position needs only the baseline nodes on
  paths where T diverges, which is the Dynamo cost bound. Baseline inner
  nodes are loaded synchronously from the store and memoized by position;
  the memo is bounded and released at retirement.
- **Safety margin.** Retirement at `baseline_seq + 256` keeps all use inside
  half of the 512-ledger prune window, so no baseline node can be deleted
  while trusted (invariant B). Startup briefly reports `tracking` from the
  stub ledger, so only `full` (not `tracking`) retires on catch-up.
- **Scope.** State tree only; transaction trees are per-ledger and share
  nothing with the baseline. Consensus-neutral: only which local reads are
  skipped changes, never ledger contents or hashes.
- **Effect on RAM.** Skipped subtrees are never loaded, so the ≈30 M-node
  resident tree during catch-up is replaced by the diverging paths only.
- **Kill switch.** `QUAXAR_TRUSTED_RESTART=0` disables installation.

### Code changes

| file | change |
|---|---|
| `xrpl/shamap/src/owners/sync.rs` | `MissingNodeResidentLookup::is_known_complete_child` (default `false`), consulted next to the FullBelow check |
| `xrpld/app/src/ledger/inbound_ledgers/trusted_baseline.rs` | new: baseline, memoized position walk, retire, stats |
| `xrpld/app/src/ledger/inbound_ledgers/coordinator_engine.rs` | `AppResident` consults the active baseline for the state tree |
| `xrpld/app/src/bootstrap/bootstrap.rs` | `install_trusted_restart_baseline` before the coordinator is installed |
| `xrpld/app/src/shamap/shamap_store_component.rs` | `pruned_driver()` accessor |
| `xrpld/app/src/network/network_ops_strand.rs` | retire at `full` or when too old |

## 5. What this does not change

- Cold sync (empty store) still materializes the tree. Bounded-RAM cold sync
  needs children-first persistence (Soft Updates applied to acquisition
  writes) and is the next step.
- Steady-state RAM is unchanged (resident trim from #89 still applies).
- Fresh-store behaviour: no claimed root, so no baseline.

## 6. Failure modes considered

| case | handling |
|---|---|
| claimed root missing from store | not installed (checked at startup) |
| orphan nodes from interrupted acquisitions | never trusted: only nodes reachable from the claimed root are compared |
| long outage (> 256 ledgers behind) | baseline retires; normal traversal resumes |
| baseline node unreadable | treated as divergent; normal path |
| target on a fork | still correct: equal hash ⇒ equal subtree |

## 7. Measurement plan

Hot restarts on the same testnet store, probe every 5 s
(`server_info`, `fetch_info`, `get_counts`), compared with the six baseline
restarts in section 1: time to `full`, NodeStore reads and writes until
`full`, peak jemalloc resident, peak tracked tree nodes, baseline skips/loads
(`trusted_baseline_retired` log event).

## 8. Results

Testnet, same fjall store, `medium` node on a 4-vCPU / 31.5 GiB EC2 host.
Probe every 5 s; raw data in `~/hot-restart/` on the server.

| run | outage | time to `full` | NodeStore reads | NodeStore writes | peak jemalloc resident | peak tracked tree nodes |
|---|---|---|---|---|---|---|
| control, main `d223b8f9` | ~10 s | **504 s** | 26.2 M (to full) | 4.3 M | **13.9 GiB** | ≈ 30 M |
| v1 `488909b0`, retire at Full | ~14 s | 15 s | 0.73 M (at 30 s) | – | 1.39 GiB | 0.86 M |
| v1 `488909b0`, retire at Full | 5 min | 10 s | 8.8 M (backfill, 150 s) | 1.4 M | 5.9 GiB | 9.7 M |
| v2 `b9507931`, retire by age | 5 min | **15 s** | 36 k (240 s) | 0.21 M | **0.67 GiB** | 73 k |
| v2 `b9507931`, retire by age | ~10 s | **10 s** | 33 k (180 s) | 0.06 M | **0.62 GiB** | 45 k |

Baseline counters (v1, 5-minute outage, first 9 s): 410,790 subtrees skipped
for 3,596 baseline node loads, no load misses.

Takeaways:

- Restart time drops from ~8.5 min to 10–15 s (≈ 35–50×), and NodeStore
  reads by ~3 orders of magnitude, matching the Dynamo bound: cost follows
  the change since the baseline, not the state size.
- Peak resident memory during catch-up drops from 13.9 GiB to < 0.7 GiB.
- v1 showed that history backfill of the restart gap also needs the baseline:
  retiring at Full let backfill rebuild 9.7 M resident nodes. v2 keeps it
  until it ages out, which is safe by invariant B.

Age retirement (v2, restart at 08:12): retired at 08:26 (`too_old`, validated
> baseline + 256) after 2,027,392 subtree skips for 12,119 baseline node loads
and no load misses; the node stayed `full` and RSS was 1.32 GiB afterwards.

Correctness checks after v2 deploys:

- `full`, `state_accounting.full.transitions == 1` after restart.
- Validated `ledger_hash` and `account_hash` match the public testnet node on
  31 of 31 sampled ledgers (every second ledger of the last 60).
- `pruned_verify_last_ok = true`, `pruned_unclaimed = 0`.

Pre-existing gap observed (not caused by this change): `ledger` by sequence
or hash for a ledger in `complete_ledgers` that is no longer in the 64-entry
ledger cache returns `lgrNotFound`, because `LedgerMaster::get_ledger_by_hash`
only consults the cache and the closed ledger (rippled falls back to loading
from the node store). Tracked separately.

## 9. Next steps

1. Children-first persistence in acquisition (Soft Updates) so a cold sync
   on an empty store also stays bounded in RAM.
2. Node-store fallback for `get_ledger_by_hash` / `by_seq` (section 8 gap).
3. Re-check steady-state RSS over 24 h with the baseline-trimmed tree.
