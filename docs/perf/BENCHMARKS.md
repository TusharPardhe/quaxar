# RPC and WebSocket benchmark: xrpld 3.4.0 vs quaxar

Date: 2026-10-09. Quaxar revision: branch `perf/rpc-ws-latency` at
[`b9c7c66b`](https://github.com/TusharPardhe/quaxar/commit/b9c7c66b), the last
code change (later commits only add benchmark tooling and these docs).

The changes behind these numbers are described in
[IMPROVEMENTS.md](IMPROVEMENTS.md). Each one is compared there, line by line,
with the xrpld source at the exact commit benchmarked here.

## Result

Each of the 32 cells (8 methods × HTTP/WebSocket × 1/16 connections) is
compared against the **faster of two xrpld configurations** for that cell.
- **Throughput:** quaxar serves **1.5x to 234x the requests per second**.
- **p99 latency:** lower in every cell.
- **Median latency:** lower in every cell but one. In HTTP `fee` at 16
  connections, standalone xrpld's median is 0.26 ms against quaxar's 0.28 ms,
  while quaxar served 28x the requests.
- **Errors:** no response from either server contained an `error` member.

```
WebSocket, 16 connections, requests per second (best xrpld configuration vs quaxar)

ledger_data (256)  xrpld    192 ▏▌
                   quaxar 2,110 ▏██████                       11.0x
book_offers (200)  xrpld     95 ▏▎
                   quaxar 1,539 ▏████▌                        16.2x
account_tx (200)   xrpld     84 ▏▎
                   quaxar   567 ▏█▋                            6.8x
account_info       xrpld  9,626 ▏█████
                   quaxar 49,403 ▏█████████████████████████    5.1x
```

The bars are drawn to scale within each method only; the methods differ by
orders of magnitude.

## What exactly was compared

| | xrpld | quaxar |
|---|---|---|
| Build | Official release image `rippleci/xrpld:3.4.0` (`sha256:8105dae3856f…`). `xrpld --version` reports commit `4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17`, which is tag [`3.4.0`](https://github.com/XRPLF/rippled/tree/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17). | Release build of [`b9c7c66b`](https://github.com/TusharPardhe/quaxar/commit/b9c7c66b), cross-compiled to x86-64 ([`Dockerfile.cross`](../../infra/bench/Dockerfile.cross)) and packaged with [`Dockerfile.runtime`](../../infra/bench/Dockerfile.runtime). |
| Architecture | linux/amd64 under emulation (xrpld publishes no arm64 image) | linux/amd64 under the **same** emulation |
| Container limits | 4 CPUs, 4 GiB, data directory on tmpfs | identical |
| Node store | NuDB, `online_delete = 512` | fjall, `online_delete = 512` |
| Config | [`rippled.cfg`](../../infra/bench/rippled.cfg) (standalone), [`rippled-net.cfg`](../../infra/bench/rippled-net.cfg) (network) | [`quaxar.cfg`](../../infra/bench/quaxar.cfg) |

Every claim about xrpld's code in these documents links to that same commit
(`4a4fded2`), so the source read and the binary measured are identical.

**Host.** An Apple M1 Pro running an OrbStack Docker VM with 10 CPUs and
12 GiB. Each server runs alone and is stopped before the next one starts.
The load generator runs in its own container with 4 CPUs. Compose file:
[`infra/bench/docker-compose.yml`](../../infra/bench/docker-compose.yml).

### Two xrpld configurations; the faster one is counted

Standalone xrpld runs its whole job queue, which executes every RPC, on
**one thread** (from
[`Application.cpp` L330-L345](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/main/Application.cpp#L330-L345)):

```cpp
if (config->standalone() && !config->forceMultiThread)
    return 1;
...
auto count = static_cast<int>(std::thread::hardware_concurrency());
```

That would understate xrpld under concurrency. So every run also starts
xrpld as a one-validator network (`--start --valid --quorum 1`, with
[`--valid` removing the minimum-peer requirement](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/misc/NetworkOPs.cpp#L342)).
In that mode the queue is sized from `hardware_concurrency()`; `server_info`
reported 6 threads.

The ratio column always divides by the **faster** of the two xrpld
configurations. Quaxar runs in standalone mode, where its RPC pool is not
reduced.

### The ledger

Every server holds the same state, built by
[`scripts/bench_populate.py`](../../scripts/bench_populate.py) from
transactions signed offline by xrpld:

```
gateway ──trust line (USD)── holder
   │                            │
   └─ 200 OfferCreate           └─ 200 Tickets
      (one XRP/USD book,
       200 different prices = 200 quality directories)
```

### The client

[`xrpld/server/examples/rpc_load.rs`](../../xrpld/server/examples/rpc_load.rs)
is a closed-loop client. It keeps N connections open, either keep-alive
HTTP/1.1 or one WebSocket each, with one request outstanding per connection,
and records every latency.

The same binary and the same request bodies drive both servers, and every
response is checked for an `error` member.

```
loadgen ──N connections──► server
  send request ─► wait for full reply ─► record latency ─► send next
```

### The requests

| name | request |
|---|---|
| ping | `{"command":"ping"}` |
| fee | `{"command":"fee"}` |
| account_info | gateway, `ledger_index: validated` |
| tx | one OfferCreate, by hash |
| account_objects_200 | holder, `limit: 400` (200 tickets + 1 trust line) |
| book_offers_200 | the XRP/USD book, `limit: 200` |
| ledger_data_256 | `limit: 256`, `ledger_index: validated` |
| account_tx_200 | gateway, `limit: 200` |

Response sizes are comparable, and quaxar's are never smaller, so it is not
winning by sending less (HTTP response bytes):

| method | xrpld | quaxar |
|---|---|---|
| account_objects_200 | 61,441 | 61,641 |
| book_offers_200 | 116,827 | 116,977 |
| ledger_data_256 | 145,253 | 167,033 |
| account_tx_200 | 458,660 | 476,165 |
| tx | 2,259 | 2,346 |

## Full results

Cells: **p50 ms / p99 ms / requests per second**.

| method | transport | conns | xrpld standalone | xrpld network | quaxar | quaxar vs best xrpld |
|---|---|---|---|---|---|---|
| account_info | http | 1 | 0.30 / 42.04 / 1,034 | 1.14 / 50.40 / 104 | 0.16 / 0.23 / 6,249 | 6.0x |
| account_info | http | 16 | 0.30 / 43.34 / 2,489 | 1.27 / 50.08 / 1,080 | 0.27 / 0.87 / 51,575 | 20.7x |
| account_info | ws | 1 | 0.25 / 0.51 / 3,793 | 0.21 / 0.35 / 4,558 | 0.15 / 0.21 / 6,759 | 1.5x |
| account_info | ws | 16 | 2.16 / 3.36 / 7,260 | 1.64 / 2.73 / 9,626 | 0.28 / 0.95 / 49,403 | 5.1x |
| account_objects_200 | http | 1 | 6.22 / 7.18 / 159 | 5.83 / 6.14 / 170 | 0.82 / 1.08 / 1,139 | 6.7x |
| account_objects_200 | http | 16 | 96.74 / 117.61 / 164 | 17.22 / 55.89 / 646 | 4.39 / 7.29 / 3,458 | 5.4x |
| account_objects_200 | ws | 1 | 6.90 / 53.97 / 54 | 6.42 / 48.73 / 60 | 0.83 / 1.03 / 1,172 | 19.5x |
| account_objects_200 | ws | 16 | 91.83 / 138.54 / 163 | 16.29 / 89.05 / 594 | 4.40 / 6.31 / 3,455 | 5.8x |
| account_tx_200 | http | 1 | 47.08 / 93.50 / 20 | 45.18 / 90.87 / 20 | 6.68 / 16.62 / 139 | 7.0x |
| account_tx_200 | http | 16 | 754.72 / 806.36 / 21 | 191.24 / 255.29 / 84 | 30.79 / 37.91 / 504 | 6.0x |
| account_tx_200 | ws | 1 | 47.94 / 57.02 / 21 | 46.16 / 62.58 / 21 | 6.52 / 7.40 / 152 | 7.2x |
| account_tx_200 | ws | 16 | 749.10 / 786.25 / 21 | 193.35 / 213.12 / 84 | 26.87 / 45.20 / 567 | 6.8x |
| book_offers_200 | http | 1 | 18.24 / 23.81 / 54 | 17.54 / 24.23 / 56 | 2.06 / 3.71 / 435 | 7.8x |
| book_offers_200 | http | 16 | 287.53 / 311.64 / 55 | 167.26 / 209.33 / 97 | 10.00 / 14.49 / 1,516 | 15.6x |
| book_offers_200 | ws | 1 | 18.46 / 23.27 / 54 | 18.04 / 30.72 / 55 | 2.03 / 2.57 / 481 | 8.7x |
| book_offers_200 | ws | 16 | 284.61 / 300.52 / 56 | 171.84 / 209.38 / 95 | 10.01 / 12.52 / 1,539 | 16.2x |
| fee | http | 1 | 0.33 / 46.02 / 277 | 1.23 / 51.04 / 78 | 0.13 / 0.26 / 7,396 | 26.7x |
| fee | http | 16 | 0.26 / 44.43 / 1,835 | 1.30 / 47.84 / 1,550 | 0.28 / 0.79 / 51,663 | 28.2x |
| fee | ws | 1 | 0.18 / 0.39 / 5,221 | 0.15 / 0.34 / 6,155 | 0.09 / 0.16 / 11,346 | 1.8x |
| fee | ws | 16 | 1.78 / 2.58 / 8,891 | 1.40 / 2.72 / 11,120 | 0.25 / 0.74 / 56,801 | 5.1x |
| ledger_data_256 | http | 1 | 14.12 / 19.06 / 70 | 13.98 / 15.15 / 71 | 1.40 / 1.87 / 651 | 9.2x |
| ledger_data_256 | http | 16 | 227.13 / 234.64 / 70 | 77.63 / 96.19 / 224 | 7.44 / 10.54 / 2,045 | 9.1x |
| ledger_data_256 | ws | 1 | 14.85 / 16.65 / 67 | 14.37 / 15.30 / 69 | 1.36 / 1.95 / 709 | 10.3x |
| ledger_data_256 | ws | 16 | 227.09 / 238.45 / 70 | 87.69 / 116.62 / 192 | 7.32 / 11.59 / 2,110 | 11.0x |
| ping | http | 1 | 0.74 / 49.68 / 92 | 1.00 / 49.69 / 78 | 0.04 / 0.10 / 21,516 | 233.9x |
| ping | http | 16 | 1.01 / 52.67 / 950 | 1.14 / 44.54 / 2,712 | 0.21 / 0.50 / 73,395 | 27.1x |
| ping | ws | 1 | 0.13 / 0.31 / 7,456 | 0.12 / 0.26 / 7,956 | 0.04 / 0.09 / 24,225 | 3.0x |
| ping | ws | 16 | 1.36 / 2.91 / 11,253 | 1.21 / 1.88 / 13,122 | 0.23 / 0.57 / 66,077 | 5.0x |
| tx | http | 1 | 0.49 / 1.04 / 1,726 | 0.51 / 0.73 / 1,901 | 0.26 / 0.62 / 3,511 | 1.8x |
| tx | http | 16 | 5.00 / 6.11 / 3,158 | 2.14 / 5.34 / 6,528 | 0.75 / 2.04 / 18,834 | 2.9x |
| tx | ws | 1 | 0.46 / 0.70 / 2,080 | 0.48 / 0.61 / 2,074 | 0.22 / 0.31 / 4,380 | 2.1x |
| tx | ws | 16 | 4.83 / 6.48 / 3,236 | 2.19 / 4.29 / 6,955 | 0.74 / 1.92 / 18,807 | 2.7x |

Raw data:
[`infra/bench/results-2026-10-09.jsonl`](../../infra/bench/results-2026-10-09.jsonl).
To regenerate the table, run `scripts/bench_report.py infra/bench/results-2026-10-09.jsonl`.

### Reading the table

- **xrpld network vs standalone.** Under concurrency, the extra job-queue
  threads help xrpld a lot (`ledger_data` at 16 connections: 70 → 224 rps).
  With one connection they don't help, because requests arrive one at a time
  anyway.
- **The ~45-50 ms HTTP p99 on xrpld.** xrpld turns Nagle's algorithm off
  (`TCP_NODELAY`) only for loopback clients
  ([`PlainHTTPPeer.h` L77-L82](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/server/detail/PlainHTTPPeer.h#L77-L82)).
  Between containers the client is not loopback. Stalls of 40-50 ms are the
  classic Nagle and delayed-ACK interaction; that cause is inferred from the
  code and the latency pattern, not traced at packet level.
  Quaxar sets `TCP_NODELAY` on every connection. This is why HTTP `ping` at
  1 connection shows the largest ratio (234x). The WebSocket rows (3x-5x for
  `ping`) are not affected by it.
- **The ledger-walking methods** (`ledger_data`, `book_offers`,
  `account_objects`, `account_tx`) do the most work per request, and quaxar is
  5x to 20x faster on them. [IMPROVEMENTS.md](IMPROVEMENTS.md) explains each
  part of that gap against the xrpld code.

## Rendering micro-benchmarks

Every response that lists ledger objects or transactions renders them to
JSON. `cargo run --release -p protocol --example json_render_bench` renders a
corpus of real mainnet and testnet data,
[`json_corpus.txt.gz`](../../xrpl/protocol/tests/fixtures/json_corpus.txt.gz):
8,592 ledger entries across 22 types and 662 transactions with metadata.

Times are ns per record in a 4-CPU container:
- **tree:** build a JSON value tree, then serialize it. xrpld's `getJson`
  works this way ([IMPROVEMENTS.md §3](IMPROVEMENTS.md#3-json-rendering)).
- **direct:** quaxar's writer.

Each direct output is asserted byte-identical to the tree output.

| class | count | decode | tree | direct | speedup |
|---|---|---|---|---|---|
| all ledger entries | 8,592 | 1,321 | 2,713 | 1,244 | 2.2x |
| AccountRoot | 5,001 | 1,211 | 2,192 | 1,034 | 2.1x |
| RippleState | 1,682 | 1,284 | 3,762 | 1,928 | 2.0x |
| DirectoryNode | 1,203 | 1,233 | 2,292 | 898 | 2.6x |
| Offer | 182 | 1,488 | 3,142 | 1,194 | 2.6x |
| NFTokenPage | 59 | 5,265 | 7,177 | 1,757 | 4.1x |
| transactions | 662 | 2,629 | 2,632 | 1,169 | 2.3x |
| metadata | 662 | 4,941 | 9,808 | 4,229 | 2.3x |

## Limits of this benchmark

- **Emulated x86-64 on an arm64 host.** Both servers run under the same
  emulation, so the comparison is like for like. Absolute numbers on native
  x86 hardware will differ.
- **In-memory ledger.** Data directories are on tmpfs and the ledger is small.
  The benchmark therefore measures the RPC/WebSocket path, ledger reads and
  rendering. It does not measure disk I/O, cache misses or network sync.
- **One ledger shape and one request set.** Very different ledgers or request
  mixes need their own runs; the tooling below makes that a small change.
- **Quaxar standalone vs xrpld network.** xrpld's network configuration also
  runs consensus rounds (one validator, nearly empty ledgers). Its standalone
  numbers are shown alongside for that reason.

## Reproduce

```sh
docker build -t quaxar-dev:1.90   -f infra/bench/Dockerfile.dev   infra/bench
docker build -t quaxar-cross:1.90 -f infra/bench/Dockerfile.cross infra/bench

# Cross-compile quaxar for x86-64 and package it.
docker volume create quaxar-xtarget-new
docker run --rm -v "$PWD":/src -v quaxar-xtarget-new:/target -e CARGO_TARGET_DIR=/target \
  quaxar-cross:1.90 cargo build --release --target x86_64-unknown-linux-gnu -p quaxar-main
mkdir -p /tmp/qimg
docker run --rm -v quaxar-xtarget-new:/target -v /tmp/qimg:/out quaxar-cross:1.90 \
  sh -c 'cp /target/x86_64-unknown-linux-gnu/release/quaxar /out/ && x86_64-linux-gnu-strip /out/quaxar'
docker build --platform linux/amd64 -t quaxar-bench:new -f infra/bench/Dockerfile.runtime /tmp/qimg

docker volume create quaxar-target-new   # load-generator build cache
scripts/bench_matrix.sh                  # xrpld standalone, xrpld network, quaxar
scripts/bench_report.py                  # prints the results table
```
