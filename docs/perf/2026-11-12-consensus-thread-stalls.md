# Consensus-thread stalls: resident-tree trim off the strand, timing logs

**Date:** 2026-11-12
**Trigger:** 3 missed ledgers at 02:46 UTC (108273245–108273247) on mainnet.

## What was measured

From the validator's journal, Nov 11 22:00 → Nov 12 13:52 UTC, thread 79 (the
NetworkOps strand, which also runs consensus): 384,700 gaps between
consecutive log lines.

| Quiet period on thread 79 | Count | Ending in `resident_tree_trimmed` |
|---|---|---|
| > 2.5 s | 142 | 78 |
| > 3 s | 68 | 44 |
| > 4 s | 11 | 1 (02:46, 7.7 s) |

| | p50 | p99 | max |
|---|---|---|---|
| Quiet period before a trim (n = 224) | 2.06 s | — | 7.7 s |
| Every other quiet period | 0.0002 s | 2.04 s | 10.9 s |
| Dirty-node persistence (`elapsed_us`, n = 14,317) | 73 ms | 416 ms | 2.2 s |

The long non-trim stalls ended in `update_our_positions: dispute status` (37 of
> 2.5 s) and `Consensus dirty-node persistence completed` (22 of > 2.5 s).

## What is assumed (not proven)

- About 2 s of a quiet period is the open phase idling; the trim adds the rest
  (≈ 0.5–1.5 s in a third of trims, 5.7 s once). The trim had no start log.
- The cost is mostly freeing nodes (the drop), not walking to them. The new
  `detach_ms` vs the dropper's `drop_ms` will show the split.

## Change

```
thread 79 (strand)                         quaxar-graph-drop (nice 10)
detach_durable_map_graphs(gen, 2, &mut v)  drop(v)   ← millions of nodes freed
  walk + move child links into v           malloc_trim → jemalloc arena purge
submit(v) ─────── bounded queue (64) ────► log background_drop_done drop_ms purge_us
log resident_tree_trimmed detach_ms
```

IMPORTANT: detaching gives exactly the same tree state as before (the same
child links are cleared); only the thread that frees the nodes changes. A full
queue drops inline on the caller, as before, and logs `background_drop_inline`.

The acquired-ledger graph release (`release_processed_durable_graphs`) uses the
same path, without the purge (it runs often during catch-up).

## Timing logs

| Event | Threshold | Fields |
|---|---|---|
| `strand_step_slow` | ≥ 250 ms | `step` ∈ peer_proposal, got_tx_set, timer_tick, execute_accept, start_round, resident_tree_trim, acquired_graph_release |
| `strand_turn_slow` | ≥ 1 s of work (the idle wait is excluded) | `sections_ms`: sections ≥ 50 ms, e.g. `end_consensus=1500` |
| `update_positions_slow` | ≥ 100 ms | `disputes` |
| `resident_tree_trimmed` | every trim | `detach_ms`, `subtrees`, `handoff` |
| `background_drop_done` | every drop | `drop_ms`, `purge_us`, `rss_freed_kb` |

## How to read a day of data

```
grep -E 'strand_(step|turn)_slow|update_positions_slow|background_drop_done' journal
```

- `background_drop_done drop_ms` large and `detach_ms` small → this change
  removed the trim stall.
- `detach_ms` large → the walk itself is slow; trim less often or walk
  incrementally.
- `strand_turn_slow sections_ms=end_consensus=…` with `execute_accept` slow →
  look at ledger build / persistence (dirty-node p99 is 416 ms).

## Update: the stalls line up with paging (Nov 12 evening)

`sar -B` (10-minute windows) on the validator:

| Event | Window | majflt/s | Window before | majflt/s |
|---|---|---|---|---|
| Nov 11 00:44 | 00:40–00:50 | 955.6 | 00:30–00:40 | 0.03 |
| Nov 12 02:46 | 02:40–02:50 | 506.9 | 02:30–02:40 | 8.2 |
| Nov 12 13:52 | 13:50–14:00 | 862.5 | 13:40–13:50 | 9.6 |

IMPORTANT: on an 8 GB host the node (5.2–5.8 GB RSS) leaves little page
cache, so the kernel evicts pages the node still uses. At 13:52 a long
`journalctl` read during a health check added to it. Two changes follow:

- the node writes `[debug_logfile]` itself (no journald read-back on the host);
- `strand_turn_slow host=majflt=… mem_some_ms=… io_some_ms=…` proves or rules
  out paging for each stall.
