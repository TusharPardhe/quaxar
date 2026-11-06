# Consensus under load: profile, causes and fixes (2026-11-06)

## Incident

Mainnet ledger 108134664 (2026-11-05 16:03 UTC). The node closed it with its
own transaction set (`MovedOn`, 1,986 transactions; the network's had 1,835),
did not sign it, and switched back to the network 3 s later at 108134671.

Execution was not at fault: the network's ledger replays byte-exact on the
deployed build. The node had fallen behind the network.

| Time (UTC) | Network | Node |
|---|---|---|
| 16:01:45 | ledger 649, 2,442 transactions | applying it took 6.8 s |
| 16:02:36–42 | closes 657, 658 | after closing 656, the consensus thread spent 6.6 s before closing 657 (open-ledger rebuild 1.9 s, closed-ledger work 1.7 s, adding 1,600 queued transactions 2.5 s) |
| 16:02:52–03:23 | 3–7 ledgers ahead | saw only 2–5 proposers per round (the rest had moved on); 1,200–4,300 disputed transactions |
| 16:03:26 | 664 validated with 1,835 transactions | `MovedOn` with its own 1,986 |

Host during 16:00–16:10 (sar, 10-minute averages): iowait 19.8% (5.8% before),
swap in/out 128/144 pages/s (about 0 before), major page faults 121/s, free
memory down to 1.5 GB, quaxar peak 6.9 GB. Median transaction apply time was
1.0 ms, against 0.5 ms for ≥1,000-transaction ledgers over the rest of the day.

## Parity check against rippled

| Area | Result |
|---|---|
| `getPreferred(curr, minValidSeq)` | Same. Validations for ledgers not yet acquired stay in `acquiring_` and do not move the trie until `checkAcquired` succeeds, so a node that is behind stays on its own chain. |
| `checkLedger` on every `timerEntry` | Same. |
| `checkConsensus` → `MovedOn` | Same rule. |
| No validation after `MovedOn` | Same. |

**IMPORTANT:** this was a throughput problem, not a logic difference. Under
rippled's rules a node that cannot keep up closes its own ledger and recovers.

## Profile (perf, 99 Hz, frame pointers, 60 s, 2026-11-06 00:40 UTC, quiet: 16 ledgers/min)

CPU share of the quaxar process by thread:

| Thread | Share | Where |
|---|---|---|
| `pruned-store` | 42% | 97% in fjall point reads from the prune claim (`compute_claim_delta` → `visit_differences`); 85% of those in kernel `pread` copying |
| overlay | 18% | message routing (`HashRouter::emplace`) |
| `fjall:worker` | 16% | LZ4 compression during compaction |
| `networkops-strand` (consensus) | 10% | see below |

Consensus thread breakdown (share of its own time): applying the accepted
transaction set 50%, open-ledger rebuild 12%, ledger build and persist 10%,
**coordinator bookkeeping 22%** (`coordinator_report_peer_availability`,
`coordinator_heartbeat`, both republishing `publish_coordinator_mirror`).

### Finding 1: whole-table indexes re-read from the page cache

`strace` on `pruned-store` for 6 s: 161 reads of ≥4 MB, all of them one of
**4 index blocks of 19.5 MB**; about 1.2 GB/s read, 9 MB/s from disk. A
probe on a hard-linked snapshot with the live 1 GiB cache measured 0.14 MB
read and 541 µs per point read.

- The live store was created before #135 (partitioned, pinned indexes). fjall
  persists keyspace options at creation and `from_kvs` reuses them on reopen,
  so **#135 never applied to this store**. Every table has one ~20 MB index.
- fjall's block cache is a `quick_cache` with cores × 4 = 16 shards of 64 MB
  each. A 20 MB index fits in a shard, but the steady stream of 4 KB data
  blocks evicts it before its next use. A local `quick_cache` model with the
  same options did not reproduce the eviction, so the exact eviction pattern is
  not established; the re-reads are measured.

At 1.2 GB/s of `pread`, the prune thread competes with consensus for CPU and
memory bandwidth, and the page cache it fills competes with quaxar's own
memory. That matches the swapping at 16:10, but it is not proven to have
caused the slowdown.

### Finding 2: coordinator bookkeeping on every strand wake

The strand loop wakes every 50 ms and on every proposal and validation. On
each wake it reported active peers to the acquisition coordinator and sent a
heartbeat fact; each call re-snapshots peers and republishes the coordinator
mirror. rippled does the equivalent once per `ledgerGranularity` (1 s) in
`NetworkOPsImp::processHeartbeatTimer`.

## Options considered

| Option | Expected gain | Cost |
|---|---|---|
| A. Rebuild the store so #135's partitioned, pinned indexes apply | removes most of the prune thread's 42% and the 1.2 GB/s of page-cache reads | the node is out of service while the store is rebuilt |
| B. Coordinator report and heartbeat once per second, or on a peer-set change | about 22% of the consensus thread | small, parity with rippled's heartbeat cadence |
| C. Faster ledger acquisition when behind (6–28 s per ledger on Nov 5) | shorter recovery | medium; needs a busy-period profile first |
| D. Memory limits under load (queue, retries, concurrent acquisitions) | removes swap pressure | medium; A may remove most of it |

## Decisions

1. **B, implemented** (`CoordinatorPeerReportCadence` in
   `network_ops_strand.rs`). The peer set is still compared on every wake, so a
   connect or disconnect is reported at once; otherwise the report and the
   heartbeat go out once per second. The below-quorum gate still runs on every
   wake. Test: `coordinator_peer_report_runs_once_per_second_or_on_peer_change`.
2. **A, done by an offline store rebuild** (2026-11-06 02:27–02:35 UTC,
   8 minutes out of service). See "Store rebuild" below.
3. **C and D:** decide after a busy-period profile on the rebuilt store
   (`~/busyprof.sh` on the validator records 90 s when 3 of 5 ledgers carry
   ≥1,500 transactions).

## Store rebuild (A)

**Snapshot route abandoned.** The online `export-snapshot` walks the state
tree with point reads on the old store, so it hit the same whole-table index
reads: 4 export threads at about 1.1 GB/s each and 70% CPU, 100 MB of output
after 15 minutes. At that rate the export would have taken hours and loaded
the validator, so I stopped it.

**Offline copy instead** (`tools/fjall-rebuild`):

1. Stop quaxar.
2. Read every live key of the old store in key order and bulk-ingest it into
   a new store created with quaxar's keyspace options (partitioned, pinned
   index and filter blocks; 1 GiB tables). Tombstones are not copied.
3. Ingestion writes uncompressed 64 MB tables at canonical level 1, so major-
   compact `nodes` into the last level with 1 GiB tables, which applies the
   last-level LZ4 policy (28 GB → 16.5 GB).
4. Move the old store aside, put the new one in place, start quaxar.

All state, including the pruned-store bookkeeping (`notebook`, `counts`,
`meta`), carries over, so nothing is re-downloaded. SQLite is unchanged and
consistent with it. A dry run on a hard-linked snapshot took 6 + 5 minutes at
idle priority before the real run.

| | Old store | Rebuilt store |
|---|---|---|
| `nodes` | L5 3.9 GB (76% tombstones) + L6 12.4 GB | L6 16.5 GB, 59.5M entries, 0 tombstones |
| Index per table | one ~20 MB block (`index_region None`) | partitioned: 361 KB pinned top level (`index_region Some`) on all 16 tables |
| Probe on a snapshot, 1 GiB cache | 0.14 MB read per point read | 0.01 MB read per point read |
| `pruned-store` thread, live | 80% CPU, 1,232 MB/s read | **4.5% CPU, 2.4 MB/s read** |
| Copy time | | 191 s ingest + 160 s compaction |
| Back to `proposing` | | 40 s after start |

**IMPORTANT:** fjall persists keyspace options when a keyspace is created, so
a changed option in `keyspace_options` reaches an existing store only through
a rebuild like this one. The old store is kept at
`/srv/quaxar/fjall.old-20261106` until the rebuilt one has run cleanly.

## How to reproduce the measurements

- Thread CPU: two `/proc/<pid>/task/*/stat` samples 30 s apart.
- Profile: `perf record -F 99 -g --call-graph fp -p <pid> -- sleep 60`.
- Large reads: `strace -e trace=pread64 -p <pruned-store tid>` for a few
  seconds, grouped by fd and offset.
- Store layout: `fjall-inspect` on a hard-linked snapshot (`index_region`
  `None` means a whole-table index).
