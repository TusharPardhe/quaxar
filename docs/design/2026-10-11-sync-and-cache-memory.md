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
miss ratio by at least a fixed tolerance (0.03: a
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
(`trim_resident_ledger_tree`).

## 4. Next steps

1. Soak the knee fix for several hours to confirm the caches hold at the floor
   and RSS stays flat.
2. Rerun A's cold sync with B's sampling enabled, to size the TreeNodeCache's
   strong set during sync. This is the remaining part of A's peak.
3. If RSS still creeps with the caches pinned, measure which retained ledgers
   pin the resident tree and compare with rippled's `LedgerHistory` retention.

## 5. Snapshot bootstrap (sidesteps the sync peak entirely)

The cold-sync peak (section on A) is a one-time cost of downloading state from
peers. A node can instead be bootstrapped from a snapshot of an existing full
node, which skips the network state download and therefore the in-memory
frontier that drives the peak. This is the assumeutxo model (Bitcoin Core,
`doc/design/assumeutxo.md`): trust the snapshot producer until local
verification confirms it. OPODIS 2022 ("Robust and Fast Blockchain State
Synchronization") documents the same snapshot-based catch-up.

Quaxar already ships `export-snapshot` (online) and `load-snapshot` (offline).
Measured on testnet (medium node, 4 vCPU, 30.6M nodes, 4.6–5.2 GB file):

| Step | Network cold sync | Snapshot bootstrap |
| --- | --- | --- |
| export (node stays online) | — | 58 s |
| load into fresh store (offline) | — | 2m 9s (parallel verify) |
| start to `full` | ~1527 s | ~15–20 s |
| peak RSS | 13.3 GiB | under 0.7 GiB |

Correctness on the bootstrapped node: `full` with one transition (no
oscillation), hash and account_hash parity on every sampled ledger,
`pruned_verify_last_ok = true`.

Operational note: the snapshot file is written by the node's service user
(`rippled`), not the invoking shell, so the output directory must be writable
by that user.

### Parallelizing the load verification

`load-snapshot` runs in two phases: a sequential bulk write of all nodes, then
a verification that walks the entire reachable SHAMap graph (one random
NodeStore read plus a SHA recomputation per node). On a fresh store the verify
walk is the dominant cost. A SHAMap is a radix tree, so the root's 16 top
subtrees are disjoint and can be verified independently. Fanning them out with
rayon (fork-join parallel tree traversal) gives, on 4 cores, measured on the
same snapshot:

| Phase | Serial (`RAYON_NUM_THREADS=1`) | Parallel (4 cores) |
| --- | --- | --- |
| load | 50.1 s | 50.2 s |
| verify | 210.5 s | 78.4 s (2.7x) |
| total | 260.6 s | 128.6 s (2.0x) |

The verify speedup is 2.7x rather than 4x because part of the walk is bound by
random LSM reads, which do not scale linearly with threads. The load phase is
sequential bulk writes (fjall's strong suit) and is unchanged. Verification
semantics are identical; the 19 snapshot tests pass on the parallel path.

## 6. Snapshot export/load profiling (what is and is not the bottleneck)

Full flow, measured on testnet (4 vCPU with SHA-NI + AVX2, 31M nodes, 4.9 GB
file). The CPU has hardware SHA-256, so SHA-256 is already fast; the levers are
I/O structure and the storage write path, not the hash or compression
algorithm.

### Export (~43 s)

| Phase | Time | Bound by |
| --- | --- | --- |
| node iteration (LSM scan + encode + compress + hash) | 34.3 s | serial LSM scan of 31M nodes |
| pass-2 copy (temp file to final file + whole-file SHA-256) | 8.5 s | 4.9 GB reread/rewrite |

A parallel per-chunk compress+hash stage (pigz model, pigz(1): independent
blocks compressed in parallel) was implemented and measured: **no change**
(42 s serial vs 43 s with 4 threads). Compression is not the bottleneck; the
export is bound by the single-cursor LSM scan. The change was reverted to keep
the code simple. Real export levers, not yet done:
- Partitioned parallel range scan: split the 256-bit key space into N ranges,
  scan each concurrently (`Keyspace::range`), emit per-range chunk streams
  (chunk order does not matter: the loader stores all nodes and verifies from
  the roots). Targets the 34 s scan.
- Eliminate the pass-2 copy by moving the chunk table to the end of the file
  (so chunk data streams straight to the final file after a fixed header).
  Removes the 8.5 s recopy but changes the file layout (needs a version bump).

### Load (~2 m 25 s = ~46 s load + ~76 s verify)

| Phase | Serial | Parallel (4 cores) | Bound by |
| --- | --- | --- | --- |
| load (verify chunk hash, decompress, decode, store) | 50.1 s | 46.5 s | serial fjall batch commits |
| verify (walk the SHAMap graph) | 210.5 s | 75.6 s (2.8x) | random reads + SHA recompute |

Only the verify parallelism (section 5) is a real win. A parallel per-chunk
decode stage was implemented and measured: **negligible** (50 s to 46 s),
because the load phase is bound by the single-writer fjall commit, not by
decompression. Reverted with the export change.

The real load lever is the storage write path. fjall 3.1 exposes a bulk
ingestion API (`Keyspace::start_ingestion` to `write` to `finish`) that builds
SSTables directly, bypassing the memtable and per-batch journal commit — the
standard fast bulk-load technique (cf. RocksDB `IngestExternalFile`). Its
precondition is that keys are written in strictly ascending order. The export
iterates the fjall `nodes` keyspace, which yields keys in sorted order, and
that order is preserved through the chunk stream.

This is now implemented. `Backend::bulk_ingest_sorted` (default: fall back to
`store_batch_result`) is overridden by the fjall backend to drive one
ingestion across the whole load; the loader feeds it a pull-based, in-order
chunk stream. The fjall path enforces ascending order and returns a clean
error (never panics) on a violation, since snapshot input is untrusted.

Measured on the same hardware/snapshot, load phase only (verify unchanged at
~77 s):

| Load write path | Load phase |
| --- | --- |
| per-chunk batch commit (memtable + journal) | 50.1 s |
| bulk ingestion (direct SSTable build) | 37.2 s |

A 26% cut on the load phase. Correctness on the ingestion-loaded store: `full`
with one transition, 13/13 ledger parity, `pruned_verify_last_ok = true`. The
on-disk bytes and SHAMap verification are unchanged.

Remaining: the export is still bound by the serial LSM scan (34 s) and the
pass-2 copy (8.5 s); those levers (partitioned parallel scan, chunk-table at
file end) are described above and not yet done.

## 7. Snapshot format v2: leaves only, rebuilt on load

A SHAMap inner node is fully determined by its 16 child hashes, so v2 stores
only the leaves of the anchor ledger's state and transaction maps, sorted by
key. The loader rebuilds every inner node bottom-up with one open node per
depth (O(depth) memory), writes each node as soon as it is final, and verifies
by matching the rebuilt roots with the header's `account_hash` / `tx_hash`.
That replaces the separate read-back walk of all 30M nodes. This is the
Ethereum `snap` approach ("retrieve and verify all the account and storage
data without downloading intermediate Merkle trie nodes. The final state trie
is reassembled locally") and bottom-up bulk loading of a B+-tree from sorted
input. The builder reproduces rippled's canonical shape (no path compression)
and is tested against quaxar's own SHAMap insert path: same root, same bytes
for every node. v1 files are rejected by version.

Testnet, same ledger size (22.1M state leaves, 30.6M nodes rebuilt), 4 vCPU:

| | v1 best (parallel verify + ingestion) | v2 |
| --- | --- | --- |
| file size | 4.98 GB | 2.68 GB |
| export (node online) | 43 s | 112 s |
| load into a fresh store | 114 s | 99 s |
| load peak RSS | ~4.6 GB | 0.38 GB |
| start to `full` | ~15 s | ~20 s |

What made the v2 load fast, from a perf profile of the first cut (142 s):
- Nodes are hashed directly (SHA-512-half over the stored prefix bytes)
  instead of building `SHAMapTreeNode` objects, and each chunk's leaves are
  hashed in parallel.
- On a store whose node table was empty, there is nothing to reconcile, so
  the 30M-hash retained set is not collected (this removed ~27 s and most of
  the memory); non-empty stores keep the reconcile path.

A writer-thread pipeline was tried and reverted: it hung in fjall's
`local_backpressure` (4+ sealed memtables, all fjall workers idle, no flush
queued). Writes run inline from the rebuild loop.

Export is slower than v1 (112 s vs 43 s) because it walks the tree with one
random store lookup per node rather than scanning the store sequentially; it
is parallelized over the state map's depth-2 subtrees. Export runs in the
background on a node that is already serving, so it is off the bootstrap
critical path; download size and load time are on it, and both improved.

Correctness on a v2-bootstrapped node: `full` with one transition, ledger and
account hash parity on every sampled ledger, `pruned_verify_last_ok = true`.

## 8. NodeObject cache removed

rippled with `online_delete` uses `DatabaseRotatingImp`, which has no
NodeObject cache. Measured on testnet, quaxar's cache had a 2 to 8 % hit
ratio at about 1 GB. It is now removed as standard behavior (`c1e5ff88`).
Store reads are served by fjall and the OS page cache.

A sampler artifact was also fixed (`e71b58c3`): `LedgerHistory::cached_ledgers`
called `fetch` on every entry, which refreshed each one, so history never
aged out (879 ledgers, 1.3M inner nodes). It now reads `strong_values()`.

## 9. Testnet fork at 21485530: exception classes

Transaction 32 (`1E2B5609…`) was a Payment of 20 USD, SendMax 90 XRP,
tfPartialPayment. The XRP to USD book held a crafted offer asking 10^17 drops
(the native maximum) for 9.999999999999995 USD, from an owner holding
9.999999999999994 USD. The owner is underfunded, so OfferStream reduces the
offer (`ceil_out_strict`), and the reduced input exceeds the native range.

| | rippled | quaxar before `e8899f93` |
| --- | --- | --- |
| Throw | `std::runtime_error` "Native currency amount out of range" | panic, same message |
| OfferStream catch | `std::overflow_error` only; runtime_error passes | caught every panic |
| Result | RippleCalc `catch (std::exception&)`: tecINTERNAL, fee only | BookStep returned tefEXCEPTION; with MPTokensV2 it would remove the offer |

`b3f3be3d` added the RippleCalc and flowCross catches, but that was not
enough: the panic never reached them. The regression test
`fp_crafted_offer_overflowing_native_range_is_tec_internal_like_rippled`
uses the exact parent-ledger state. It failed with tefEXCEPTION before the
fix and passes after, with and without MPTokensV2.

Fix (`e8899f93`): typed payloads in `basics::contract`. `OverflowError`
mirrors `std::overflow_error`. Untyped panics count as `runtime_error`, and
`ContractViolation` counts as `logic_error`. Every rippled
`std::overflow_error` site we found now throws `OverflowError`: Number,
IOUAmount, muldiv/mulRound/divRound, IOU range, and root2. Catch sites match
on the C++ class:

| rippled site | catches | quaxar |
| --- | --- | --- |
| OfferStream::step | overflow_error (MPTokensV2: remove, else rethrow) | `small_offer_exception_disposition` |
| BookStep::forEachOffer | overflow_error (MPTokensV2: remove offer) | `catch_crossing_overflow!` around crossing arithmetic |
| AMMLiquidity::getOffer | overflow_error, std::exception: no offer | `get_amm_offer` |
| RippleCalc, OfferCreate::flowCross | std::exception: tecINTERNAL | `b3f3be3d` |
| AMMDeposit/Withdraw applyGuts | runtime_error (fixCleanup3_4_0: tecAMM_FAILED) | `amm_math_panic_ter` (logic_error stays tefEXCEPTION) |
| equalDepositTokens, equalWithdrawTokens | std::exception: tecINTERNAL | LP-token branches |
| VaultDeposit/Withdraw/Clawback | overflow_error: tecPATH_DRY | `catch_vault_overflow`, plus the missing `debitIsNonZeroDust` |

Not ported:
- `validFwd` FlowException catches: rippled has no production callers.
- The AMM synthetic-offer consumption inside forEachOffer's try block.
- XChainBridge attestation parsing `catch (...)`.
