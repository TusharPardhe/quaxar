# Bounding memory during sync and in steady state

Branch `perf/trusted-baseline-restart` (not merged). This follows the trusted
baseline restart work (`2026-10-11-trusted-baseline-restart.md`) and covers two
more strategies:

- A. Children-first release during sync, which bounds the cold-sync spike.
- B. Cache sizing from a measured miss-ratio curve, which bounds the caches
  once the node is `full`.

All measurements are from the testnet node (3.9.172.38, `node_size = medium`,
fjall store, `online_delete = 512`), taken on 2026-10-11.

## 1. Problem

| Situation | Before |
| --- | --- |
| Cold sync from an empty store | 13.3 GiB peak RSS; 8.3M inner and 21.4M leaf nodes resident |
| Steady state after `full` | RSS 0.77 → 1.91 GiB over the first 25 min and still rising |

## 2. A: children-first release during sync

### Research basis

Soft Updates (Ganger, McKusick, Soules, Patt, ACM TOCS 2000) orders metadata
writes so that whatever a pointer refers to reaches disk before the pointer
does. Once that holds, an in-memory copy of the referenced block can be thrown
away at any time and read back later. Merkle trees work the same way: a parent
names each child by hash, so a child that is durable can always be re-read and
re-verified through its parent.

### The invariant we already had

The acquisition engine only publishes a FullBelow marker for an inner node
after the NodeStore write batch that holds the node's whole subtree has been
accepted (`publish_persisted_full_below`). So "marked full below" already
means "this subtree is durable". Before this change the marked subtree stayed
fully decoded in memory until the whole ledger finished.

### Change

When a full-below marker is published, drop that node's loaded children
(`release_loaded_children`). The node keeps its child hashes and its
full-below generation, so:

- every traversal still skips it (`is_full_below`), with no extra reads;
- any later reader reloads a child through the TreeNodeCache or NodeStore by
  hash, exactly like a backed SHAMap descend.

Only nodes with `cowid == 0` (shared, immutable) are released.
`QUAXAR_DURABLE_RELEASE=0` turns the release off.

### Example

```text
             R                      R
           /   \                  /   \
          a     b       ==>      a     b*      b* = full below, durable
         / \   / \              / \    (hashes only, children dropped)
        1   2 3   4            1   2

  b's subtree was written, its marker published, so 3 and 4 are freed.
  Memory now follows the active frontier (a, 1, 2), not the whole tree.
```

### Results: cold sync on an empty store

Same host and network for both runs, one after the other.

| | Control `b9507931` | A `ad362fb5` |
| --- | --- | --- |
| Time to `full` | 1527 s | 1538 s |
| Peak RSS | 13.3 GiB | **6.5 GiB** |
| Peak jemalloc allocated | 13.4 GiB | 6.2 GiB |
| Peak inner / leaf nodes | 8.27M / 21.4M | 4.73M / 5.81M |
| Peak structural bytes | 6.1 GiB | 2.4 GiB |
| Subtree releases | 0 | 9,927,814 |

Correctness checks on the A run:

- `full`, with one transition.
- Hash and account_hash match the public node on 31 of 31 sampled ledgers.
- `pruned_verify_last_ok = true`.

The remaining peak (about 4.7M inner nodes) has two sources. One is nodes
whose writes are still in flight, so their markers are still staged. The other
is the TreeNodeCache's strong set of up to 2M entries during sync.

## 3. B: cache sizing from a miss-ratio curve

### Research basis

The core reference is SHARDS (Waldspurger, Park, Garthwaite, Ahmad, USENIX
FAST 2015):

- Hash each key and sample the keys whose hash falls below a threshold
  (rate `R`).
- For each sampled reference, compute its LRU reuse distance in the sampled
  stream and scale it by `1/R`.
- The histogram of scaled distances gives the miss ratio of an LRU cache at
  every size. It costs constant memory and has about 0.01 mean absolute error
  at small `R`.

Mattson et al. (IBM Systems Journal 1970) is the stack-distance foundation
SHARDS builds on.

The rest of the data is left to the OS page cache. That is the usual split
for a log-structured store, where the kernel already keeps hot SST blocks.

### Example

```text
miss ratio
 1.0 |*
     | *
 0.5 |  *------------------------    <- flat: more cache buys nothing
     |
     +--+--+--+--+--+--+--+--+---- cache size (entries)
       4k 32k 64k ... 2M
             ^ knee: smallest size within 2 points of the best miss ratio
```

TreeNodeCache curve measured on the node once `full`, from 8,078 samples:

```text
4k:0.544 8k:0.528 16k:0.518 32k:0.509 64k:0.496 128k:0.493 ... 8M:0.493
```

NodeObject cache curve, from 10,160 samples:

```text
4k:1.000 ... 64k:0.997 256k:0.991 ... 8M:0.991
```

Once the node is `full`, both caches see almost no reuse. Each closed ledger
touches mostly new nodes. The configured sizes (2,097,152 decoded nodes and
1.5 GB of NodeObjects) buy at most 5 points of tree-node hit ratio and under 1
point of NodeObject hit ratio.

### Change

- `basics::sync::shards`: the SHARDS estimator, plus a process-wide sampler.
  Unsampled keys are rejected without a lock, and sampling is off unless the
  node is `full`.
- `SHAMapFamily::cache_lookup` and `NodeObjectCache::get_or_load` record
  their lookups (rate 1/16).
- At each application sweep (`size_caches_from_mrc`):
  - TreeNodeCache target = the knee, clamped to [65,536, configured].
  - NodeObject cache bytes = knee × the measured mean entry size, clamped to
    [64 MiB, configured]. It is rebuilt only on a change of more than 25%,
    because a rebuild drops the contents. The rebuild is fenced like
    `invalidate_all`.
  - All counts are halved every 10 sweeps, so the curve follows the current
    workload.
- `get_counts` reports `treenode_mrc`, `node_object_mrc`, their sample counts
  and knees, and the live `treenode_cache_target_size`.
- `QUAXAR_MRC_CACHE=0` keeps the configured sizes.

### Results: steady state after a hot restart, at equal uptime

| Uptime | Control `b9507931` | A only (sampling, no resize) | A + B `6eb54aa3` |
| --- | --- | --- | --- |
| 5 min RSS | 768 MB | 654 MB | 642 MB |
| 15 min RSS | 1129 MB | 1042 MB | 899 MB |
| 25 min RSS | 1911 MB | 1739 MB | **1078 MB** |
| 25 min NodeObject cache | 646 MB | 443 MB | 63 MB |
| NodeObject hit ratio | | 0.07 | 0.11–0.15 |

What B decided:

- TreeNodeCache target went from 2,097,152 to 65,536 entries.
- NodeObject cache went from 1.5 GB to 64 MiB.

The NodeObject hit ratio did not drop. It rose, because the smaller LRU keeps
only the recent working set and its idle expiry no longer competes with a large
cold tail. NodeStore reads over the same uptime were 202k with B against 549k
without it.

Correctness checks on the B run:

- `full`, with one transition.
- 31 of 31 ledgers match on hash and account_hash.
- `pruned_verify_last_ok = true`.

### What B does not fix

RSS still rises after 25 min with B: 1.08 GiB at 25 min, 1.30 GiB at 36 min.
Neither cache is responsible. The NodeObject cache stays at 63 MB and the
TreeNodeCache at 9–38k entries. The growth is the resident ledger tree:
inner nodes went from 502k to 752k and leaves from 130k to 236k. Those nodes
are reachable from the retained ledgers, which are the 64-entry ledger cache
plus the open and closed ledgers. They are trimmed every 64 ledgers
(`trim_resident_ledger_tree`; the interval is set with
`QUAXAR_RESIDENT_TRIM_INTERVAL`). That is the next target.

## 4. Next steps

1. Resident-tree creep: measure which retained ledgers pin the 750k inner
   nodes, then compare with rippled's `LedgerHistory` retention (64 ledgers
   for 180 s at medium) and its node sharing.
2. Rerun A's cold sync with B's sampling enabled, to size the TreeNodeCache's
   strong set during sync. This is the remaining part of A's peak.
3. 24-hour soak of A + B to confirm hit ratios and RSS.
