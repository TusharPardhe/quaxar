# RPC / WebSocket performance: quaxar vs rippled (2026-10-09)

Branch `perf/rpc-ws-latency`. All numbers below were produced in containers
(OrbStack on an M1 Pro, 10 cores / 12 GiB VM) with the scripts in this repo.

## Result

On every measured method, transport and concurrency, this branch serves
**1.85x to 275x the throughput of rippled 3.4.1**, with lower p99 latency
everywhere and lower median latency everywhere except HTTP `fee` at 16
connections (0.30 vs 0.27 ms p50, while serving 31x the requests). Against `origin/main` it ranges from 0.97x (cheap `fee` calls at
16 connections, within the ~±10% run-to-run noise measured for small
requests) to 14x. The largest
gains are on the heavy, ledger-walking methods under concurrency
(`ledger_data`, `book_offers`, `account_objects`, `account_tx`: 16x to 25x
rippled at 16 connections).

## Method

- **Servers**, each run alone in standalone mode (`-a --start`) with the same
  limits (4 CPUs, 4 GiB, tmpfs data dir), `infra/bench/docker-compose.yml`:
  - `rippled`: prebuilt release image `rippleci/xrpld:3.4.1`
    (`sha256:4584c973052f…`, linux/amd64).
  - `quaxar-base`: `origin/main` (8c4280e7) plus the standalone skip-list fix
    (07b34452), without which a standalone quaxar node crashes on `submit`.
  - `quaxar-new`: this branch.
  - Both quaxar images are cross-compiled for linux/amd64
    (`infra/bench/Dockerfile.cross`) so all three run under the **same**
    amd64 emulation as the only available rippled images.
  - `quaxar-native`: this branch built natively (arm64), for reference only.
- **Ledger**: identical on every server (`scripts/bench_populate.py`): two
  funded accounts, a USD trust line, 200 offers in one order book (gateway),
  200 tickets (holder). Transactions are signed offline by rippled.
- **Client**: one closed-loop generator for all servers
  (`xrpld/server/examples/rpc_load.rs`, 4 CPUs): keep-alive HTTP/1.1 or one
  WebSocket per connection, one outstanding request per connection. Every
  response is checked for an `error` member (none occurred).
- **Run**: `scripts/bench_matrix.sh`; render with `scripts/bench_report.py`.
  Raw results: `infra/bench/results-2026-10-09.jsonl`.

## Results

Cells are **p50 ms / p99 ms / requests per second**. Ratios compare throughput.

| method | transport | conns | rippled | quaxar-base | quaxar-new | quaxar-native | new vs rippled | new vs base |
|---|---|---|---|---|---|---|---|---|
| account_info | http | 1 | 0.29 / 43.02 / 789 | 0.20 / 0.31 / 4,898 | 0.16 / 0.27 / 5,987 | 0.13 / 0.28 / 7,341 | 7.6x | 1.2x |
| account_info | http | 16 | 0.30 / 43.48 / 2,437 | 0.34 / 1.19 / 40,059 | 0.29 / 0.78 / 50,393 | 0.22 / 0.63 / 64,674 | 20.7x | 1.3x |
| account_info | ws | 1 | 0.26 / 0.54 / 3,638 | 0.18 / 0.30 / 5,494 | 0.14 / 0.24 / 6,736 | 0.10 / 0.25 / 10,008 | 1.9x | 1.2x |
| account_info | ws | 16 | 2.17 / 3.77 / 7,182 | 0.32 / 0.96 / 42,548 | 0.31 / 0.90 / 45,503 | 0.23 / 0.65 / 61,070 | 6.3x | 1.1x |
| account_objects_200 | http | 1 | 5.27 / 8.78 / 184 | 3.00 / 5.01 / 307 | 1.00 / 3.21 / 877 | 0.62 / 3.35 / 1,359 | 4.8x | 2.9x |
| account_objects_200 | http | 16 | 80.11 / 85.91 / 199 | 8.77 / 61.13 / 1,057 | 4.53 / 6.46 / 3,390 | 3.55 / 5.17 / 4,309 | 17.0x | 3.2x |
| account_objects_200 | ws | 1 | 6.36 / 70.81 / 44 | 2.85 / 5.74 / 338 | 0.86 / 1.16 / 1,127 | 0.60 / 0.80 / 1,608 | 25.6x | 3.3x |
| account_objects_200 | ws | 16 | 75.14 / 124.51 / 195 | 6.91 / 53.85 / 1,356 | 4.84 / 7.43 / 3,167 | 3.45 / 4.85 / 4,453 | 16.2x | 2.3x |
| account_tx_200 | http | 1 | 38.79 / 51.17 / 25 | 18.04 / 28.69 / 53 | 7.06 / 12.25 / 134 | 4.66 / 5.59 / 201 | 5.4x | 2.5x |
| account_tx_200 | http | 16 | 624.36 / 647.15 / 25 | 86.23 / 163.25 / 179 | 28.09 / 35.24 / 546 | 19.86 / 26.31 / 768 | 21.8x | 3.1x |
| account_tx_200 | ws | 1 | 39.40 / 46.10 / 25 | 17.50 / 18.78 / 57 | 6.98 / 8.36 / 142 | 4.55 / 5.89 / 216 | 5.7x | 2.5x |
| account_tx_200 | ws | 16 | 619.91 / 641.84 / 26 | 79.53 / 148.12 / 196 | 30.16 / 37.89 / 520 | 20.43 / 33.41 / 739 | 20.0x | 2.7x |
| book_offers_200 | http | 1 | 15.97 / 16.75 / 62 | 8.06 / 22.35 / 118 | 2.32 / 2.63 / 420 | 1.44 / 1.88 / 650 | 6.8x | 3.6x |
| book_offers_200 | http | 16 | 253.14 / 261.24 / 63 | 108.14 / 297.26 / 130 | 11.19 / 14.61 / 1,398 | 7.50 / 10.10 / 2,043 | 22.2x | 10.8x |
| book_offers_200 | ws | 1 | 16.34 / 17.49 / 61 | 8.02 / 14.25 / 123 | 2.31 / 2.70 / 430 | 1.55 / 2.73 / 620 | 7.0x | 3.5x |
| book_offers_200 | ws | 16 | 252.00 / 268.23 / 63 | 108.15 / 271.82 / 131 | 10.70 / 14.78 / 1,454 | 7.10 / 9.46 / 2,164 | 23.1x | 11.1x |
| fee | http | 1 | 0.46 / 46.64 / 212 | 0.15 / 0.26 / 6,355 | 0.14 / 0.23 / 7,046 | 0.10 / 0.21 / 9,317 | 33.2x | 1.1x |
| fee | http | 16 | 0.27 / 45.98 / 1,555 | 0.29 / 0.77 / 49,851 | 0.30 / 0.84 / 48,349 | 0.22 / 0.71 / 64,071 | 31.1x | 1.0x |
| fee | ws | 1 | 0.18 / 0.65 / 4,859 | 0.13 / 0.23 / 7,531 | 0.10 / 0.19 / 10,006 | 0.04 / 0.14 / 17,746 | 2.1x | 1.3x |
| fee | ws | 16 | 1.76 / 4.01 / 8,559 | 0.28 / 0.81 / 50,399 | 0.29 / 0.84 / 49,105 | 0.21 / 0.60 / 67,886 | 5.7x | 1.0x |
| ledger_data_256 | http | 1 | 12.57 / 13.57 / 79 | 9.44 / 10.96 / 105 | 1.45 / 2.06 / 628 | 1.07 / 3.05 / 788 | 7.9x | 6.0x |
| ledger_data_256 | http | 16 | 198.51 / 204.08 / 80 | 101.80 / 258.19 / 140 | 7.74 / 10.71 / 1,976 | 5.68 / 8.29 / 2,623 | 24.7x | 14.1x |
| ledger_data_256 | ws | 1 | 13.19 / 20.90 / 75 | 9.44 / 16.01 / 105 | 1.51 / 2.16 / 643 | 1.07 / 1.42 / 914 | 8.6x | 6.1x |
| ledger_data_256 | ws | 16 | 201.01 / 251.82 / 78 | 101.69 / 270.41 / 139 | 7.86 / 11.61 / 1,965 | 5.71 / 7.97 / 2,664 | 25.2x | 14.1x |
| ping | http | 1 | 0.94 / 48.55 / 76 | 0.07 / 0.17 / 12,837 | 0.04 / 0.12 / 20,872 | 0.02 / 0.11 / 32,950 | 274.6x | 1.6x |
| ping | http | 16 | 1.16 / 51.76 / 962 | 0.30 / 0.83 / 48,052 | 0.23 / 0.56 / 66,746 | 0.16 / 0.73 / 87,228 | 69.4x | 1.4x |
| ping | ws | 1 | 0.14 / 0.30 / 7,123 | 0.07 / 0.18 / 12,948 | 0.04 / 0.12 / 22,641 | 0.02 / 0.13 / 32,837 | 3.2x | 1.7x |
| ping | ws | 16 | 1.24 / 2.37 / 12,566 | 0.27 / 0.78 / 53,708 | 0.23 / 0.59 / 66,239 | 0.15 / 0.58 / 89,113 | 5.3x | 1.2x |
| tx | http | 1 | 0.45 / 0.78 / 1,780 | 0.32 / 0.54 / 2,929 | 0.26 / 0.44 / 3,711 | 0.18 / 0.32 / 5,171 | 2.1x | 1.3x |
| tx | http | 16 | 4.34 / 5.66 / 3,571 | 0.89 / 3.57 / 15,064 | 0.76 / 2.07 / 18,528 | 0.52 / 1.48 / 26,640 | 5.2x | 1.2x |
| tx | ws | 1 | 0.48 / 0.79 / 1,996 | 0.31 / 0.52 / 3,107 | 0.24 / 0.40 / 3,946 | 0.18 / 0.35 / 5,316 | 2.0x | 1.3x |
| tx | ws | 16 | 4.17 / 5.14 / 3,759 | 0.91 / 3.92 / 14,835 | 0.83 / 2.20 / 17,001 | 0.58 / 1.69 / 23,934 | 4.5x | 1.1x |

Cells: p50 ms / p99 ms / requests per second. Ratios compare throughput. Responses with an error member: 0.

Response sizes are equivalent across servers (e.g. `book_offers` 116.8 KB
rippled vs 117.0 KB quaxar; `ledger_data` differs because rippled returns
fewer bytes per entry for the same 256 entries).

rippled's ~45 ms p99 on single-connection HTTP `ping`/`fee`/`account_info`
is reproducible (Nagle/delayed-ACK on its HTTP responses); quaxar sets
`TCP_NODELAY`.

## What changed (commits on this branch)

| Change | Effect (measured) |
|---|---|
| Single-pass request parse, direct response serialization, inline `ping`/`random`, `TCP_NODELAY`, bounded WS egress + batched flushes (fa88840d) | loopback `ping` 52 → 36 µs; WS fan-out no longer drops 88% of events |
| Table-driven hex, rippled `b58_fast` + reciprocal division (812577d7, 4e756eb3) | hash→hex 1,625 → 51 ns; address encode 1,058 → 157 ns |
| Shared `SOTemplate`, O(n) `apply_template`, const SHAMap depth masks (c18a1bc2) | SLE decode ~2.3 → 1.3 µs; `book_offers` 4.5 → 2.3 ms |
| Direct JSON writer + `JsonValue::Raw`, raw rendering in RPC dispatch and the transactions stream (902cdc03, 9c7ca826) | per ledger object 2.2x, metadata 2.3x faster to render (below) |
| Per-branch SHAMap inner-node locks, spin backoff (cd4e981f) | 16-conn `ledger_data` 153 → 324 rps |
| RPC handler lanes sized to CPUs (bfadfac7, 00f736b7) | 16-conn `ledger_data` 324 → 656 rps; point lookups unthrottled |
| One forward walk per page (`ledger_data`, `book_offers` directories), tx DB lock released before decoding (`account_tx`) (b9c7c66b) | 16-conn `ledger_data` 706 → 2,306, `book_offers` 664 → 2,171, `account_tx` 479 → 892 rps (native) |
| Standalone `ledger_accept` updates the skip list (07b34452) | standalone `submit` no longer crashes the node |

## Rendering micro-benchmarks

`cargo run --release -p protocol --example json_render_bench` over the parity
corpus (`xrpl/protocol/tests/fixtures/json_corpus.txt.gz`: 8,592 mainnet and
testnet ledger entries across 22 types, 662 transactions with metadata),
ns per record, 4-CPU container. "tree" is the previous `json()` +
serialize path (already including the hex/base58 work); "new" is the direct
writer. Every direct rendering is asserted byte-identical to the tree.

| class | count | JSON bytes | decode | tree | new | speedup |
|---|---|---|---|---|---|---|
| all ledger entries | 8,592 | 463 | 1,321 | 2,713 | 1,244 | 2.18x |
| AccountRoot | 5,001 | 333 | 1,211 | 2,192 | 1,034 | 2.12x |
| RippleState | 1,682 | 603 | 1,284 | 3,762 | 1,928 | 1.95x |
| DirectoryNode | 1,203 | 623 | 1,233 | 2,292 | 898 | 2.55x |
| Offer | 182 | 552 | 1,488 | 3,142 | 1,194 | 2.63x |
| NFTokenPage | 59 | 2,825 | 5,265 | 7,177 | 1,757 | 4.08x |
| transactions | 662 | 599 | 2,629 | 2,632 | 1,169 | 2.25x |
| metadata | 662 | 2,001 | 4,941 | 9,808 | 4,229 | 2.32x |

Before any of this work the same Offer took 9.4 µs to build its tree
(`xrpld/server/examples/render_cost.rs`), so object rendering is ~8x faster
end to end.

## Correctness evidence

- Byte-identical JSON on the real-ledger corpus and on 25k mutated records
  (`xrpl/protocol/tests/serialization/json_writer_corpus.rs`).
- Endpoint byte-identity, raw vs tree rendering, for `book_offers`,
  `account_objects`, `account_info`, `ledger_data`, `tx` (v1/v2), `ledger`
  (`xrpld/rpc-integration-tests/tests/handlers/raw_rendering_parity.rs`) and
  the transactions stream event.
- Workspace tests in the dev container: 16,665 passed; every failure is
  either on `origin/main`'s failing list or a timing-flaky test that passes
  in isolation (`job_queue_counts_waiting…`, `workers_pause_resume…`,
  `callback_budget…`, `load_manager_can_raise…`, `overlay_peer_round_trip…`).

## Caveats

- amd64 under emulation on an arm64 host. The emulation applies equally to
  rippled and both quaxar builds; absolute numbers on native x86 hardware
  will differ. `quaxar-native` shows native arm64 quaxar for reference.
- Standalone mode, in-memory ledger: measures the RPC/WS path, ledger reads
  and rendering, not disk I/O or network sync.
- One ledger shape (one 200-offer book, 200 tickets); results for very
  different ledgers or request mixes need their own runs.

## Reproduce

```
docker build -t quaxar-dev:1.90 -f infra/bench/Dockerfile.dev infra/bench
docker build -t quaxar-cross:1.90 -f infra/bench/Dockerfile.cross infra/bench
# cross-compile quaxar for each revision into a dir with `quaxar`, then:
docker build --platform linux/amd64 -t quaxar-bench:new -f infra/bench/Dockerfile.runtime <dir>
scripts/bench_matrix.sh            # rippled quaxar-base quaxar-new [quaxar-native]
scripts/bench_report.py
```
