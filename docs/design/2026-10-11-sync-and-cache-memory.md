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

### The knee regression and its fix

The first B build sized the caches well for the first half hour but drifted
back to full size over the following hours: at 8 h, RSS had reached 3.2 GiB,
the NodeObject cache was back to 1.3 GB and the TreeNodeCache target to its
configured 2,097,152.

Root cause was the knee rule, not the resident tree. The original rule picked
the smallest size whose miss ratio was within `tolerance` of the best
(largest-size) miss ratio. A miss-ratio curve is convex and non-increasing. In
the first half hour the sampled curve was flat (little reuse seen yet), so the
knee sat at the minimum. As samples accumulated, real reuse at large sizes
appeared and the curve sloped down all the way to the top:

```text
node-object @ 8 h: 4k:1.00 64k:0.995 256k:0.934 1M:0.804 2M:0.778 4M:0.757
tree       @ 8 h: 4k:0.527 64k:0.511 256k:0.475 1M:0.413 2M:0.386 4M:0.345
```

On such a curve "within tolerance of the best" degenerates to the maximum
size, because the best is only reached near the maximum. So both caches grew
back.

The fix replaces the rule with a diminishing-returns criterion. Sizes double
at each step, so the target is the top of the last doubling that still cuts the
miss ratio by at least `tolerance` (`QUAXAR_MRC_TOLERANCE`, default 0.03: a
doubling of cache memory must buy at least 3 points of hit ratio). A flat head
before a steep region is skipped; a shallow tail past the last paying doubling
is not bought. `knee_of_curve` has unit tests for the measured production
curves plus flat, step, and always-paying shapes.

With the fix, at 36 min uptime and 13k samples the curves had flattened past
128k and both caches were held at their floors:

- TreeNodeCache target 65,536 (knee 4,096, clamped to the floor).
- NodeObject cache 64 MiB (knee 4,096, clamped to the floor).
- RSS 1.30 GiB, with one `full` transition and 21 of 21 ledgers matching.

The soak over several hours is pending to confirm the caches stay at the floor
rather than drifting up again.

### Note on the resident ledger tree

An earlier draft attributed the long-run growth to the resident ledger tree
(inner nodes rising from 502k to 752k). That rise is real but secondary: the
dominant cause of the 8 h growth was the knee regression above. With the caches
held at their floors, the resident tree (about 150 MB of structural plus item
bytes at 320k nodes) is the next thing to watch, trimmed every 64 ledgers
(`trim_resident_ledger_tree`, interval `QUAXAR_RESIDENT_TRIM_INTERVAL`).

## 4. Next steps

1. Soak the knee fix for several hours to confirm the caches hold at the floor
   and RSS stays flat.
2. Rerun A's cold sync with B's sampling enabled, to size the TreeNodeCache's
   strong set during sync. This is the remaining part of A's peak.
3. If RSS still creeps with the caches pinned, measure which retained ledgers
   pin the resident tree and compare with rippled's `LedgerHistory` retention.
