# RPC and WebSocket improvements, compared with xrpld

This document explains each change behind the numbers in
[BENCHMARKS.md](BENCHMARKS.md). For each one it shows:

1. **xrpld:** what xrpld does, linked to its source at tag `3.4.0`
   ([`4a4fded2`](https://github.com/XRPLF/rippled/tree/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17)),
   the exact commit of the benchmarked binary. Every xrpld statement below
   was read from that source.
2. **quaxar before / after:** what quaxar did before the change and what it
   does now, with the quaxar commit and files.
3. **Example and diagram:** a small example of the effect.
4. **Measured:** the effect, and how correctness was checked.

`X` below abbreviates
`https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17`.

## Summary

| # | Area | xrpld 3.4.0 | quaxar now | Verdict |
|---|---|---|---|---|
| 1 | Request path | Every request, including `ping`, posted as a JobQueue coroutine. One parse; WS reply copied twice. | One parse, one serialization into the send buffer; `ping`/`random` answered inline. | quaxar ahead |
| 2 | `TCP_NODELAY` | Loopback clients only | Every connection | quaxar ahead (non-loopback) |
| 3 | JSON rendering | `getJson` builds a `std::map` tree per object | Writes JSON text directly from the object | quaxar ahead |
| 4 | Decoding ledger objects | Template by pointer; template application O(n²) | Template shared; template application O(n) | quaxar ahead |
| 5 | Base58 / hex | `b58_fast` with a 128-bit division; `boost::algorithm::hex` | Same `b58_fast` algorithm, division by precomputed reciprocal; table hex | parity, slight edge |
| 6 | SHAMap child locks | One lock bit per child (`PackedSpinlock`) | Same, plus bounded spin then yield | parity (was behind) |
| 7 | RPC worker count | Job queue sized from `hardware_concurrency()` (1 thread standalone) | Lanes sized to CPUs; heavy and light calls separated | parity (was behind), plus lanes |
| 8 | `ledger_data` paging | One forward iterator per page | One forward walk per page | parity (was behind) |
| 9 | `book_offers` directories | `succ` + `read` per quality directory | One forward walk over the directories | quaxar ahead |
| 10 | `account_tx` | Decodes every row while holding the tx-DB lock | Reads rows under the lock, decodes after releasing it | quaxar ahead |
| 11 | WS slow clients | Queue limit 100, close "client is too slow" | Same | parity (was unbounded) |
| 12 | Standalone close | Updates the skip list on every close | Same | parity (was a crash) |

---

## 1. Request path

**xrpld.**
- **HTTP:** every request is posted to the job queue as a coroutine
  ([`ServerHandler.cpp` L320-L323](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/rpc/detail/ServerHandler.cpp#L320-L323)).
  It is parsed into a `json::Value`
  ([L614-L615](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/rpc/detail/ServerHandler.cpp#L614-L615)),
  and the reply is turned into a string
  ([L994](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/rpc/detail/ServerHandler.cpp#L994)).
- **WebSocket:** the message is parsed on the I/O thread
  ([L340](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/rpc/detail/ServerHandler.cpp#L340))
  and posted as a coroutine job
  ([L358-L367](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/rpc/detail/ServerHandler.cpp#L358-L367)).
  The reply is rendered with `to_string(jr)` and then copied into a separate
  `multi_buffer` before sending.

**quaxar before.** Requests were converted twice: bytes to a
`serde_json::Value`, then to the protocol `JsonValue`. Replies were converted
back the same way, then serialized. Every request, including `ping`, was
handed to the blocking thread pool.

**quaxar after**
([`fa88840d`](https://github.com/TusharPardhe/quaxar/commit/fa88840d),
[`transport/json.rs`](../../xrpld/server/src/transport/json.rs),
[`transport/router.rs`](../../xrpld/server/src/transport/router.rs)).
Requests are parsed once, with SIMD (sonic-rs), straight into the protocol
type. Results are serialized once, directly into the bytes that are sent.
`ping` and `random` are answered on the I/O thread with no handoff.

```
xrpld WS:       frame ─parse─► json::Value ─post job─► handler ─to_string─► std::string ─copy─► send buffer
quaxar before:  frame ─parse─► tree A ─copy─► tree B ─pool─► handler ─► tree B ─copy─► tree A ─write─► send buffer
quaxar after:   frame ─parse─► tree ─(pool, or inline for ping)─► handler ─► tree ─write─► send buffer
```

**Example.** For `{"command":"ping"}`, xrpld schedules a job and wakes a
worker thread to produce `{"result":{"status":"success"}}`. Quaxar writes
the reply on the thread that read the request.

**Measured.**
- Loopback `ping` went from 52 µs to 36 µs, and 32-connection throughput from
  65k to 130k rps (macOS loopback harness `xrpld/server/examples/rpc_latency.rs`).
- In the benchmark, WS `ping` is 3x-5x xrpld.
- Correctness: tests assert the new response bytes equal the old path's for
  HTTP and WS envelopes, and that parsing matches serde_json on a corpus of
  valid and invalid documents. One known difference: `-0` loses its sign,
  and it still arrives as a non-integer string.

## 2. TCP_NODELAY

**xrpld** turns Nagle's algorithm off only for loopback peers
([`PlainHTTPPeer.h` L77-L82](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/server/detail/PlainHTTPPeer.h#L77-L82)):

```cpp
// Set TCP_NODELAY on loopback interfaces,
// otherwise Nagle's algorithm makes Env
// tests run slower on Linux systems.
if (remoteEndpoint.address().is_loopback())
    socket_.set_option(boost::asio::ip::tcp::no_delay{true});
```

**quaxar after**
([`fa88840d`](https://github.com/TusharPardhe/quaxar/commit/fa88840d),
[`runtime/runtime.rs`](../../xrpld/server/src/runtime/runtime.rs)).
`TCP_NODELAY` is set on every accepted connection: plain, mixed peer/RPC, and
TLS.

```
Nagle on:   reply bytes ──wait for ACK of earlier small write (up to ~40 ms)──► wire
Nagle off:  reply bytes ──────────────────────────────────────────────────────► wire
```

**Example.** A wallet backend calling `account_info` over HTTP from another
host is a non-loopback peer.

**Measured.**
- xrpld's HTTP rows for the cheap methods (`ping`, `fee`, `account_info`)
  have p99 of 42-52 ms with medians under 1.3 ms. Quaxar's p99 for the same
  rows is 0.1-0.9 ms.
- The cause is inferred from the code above and the latency pattern; it was
  not traced at packet level.

## 3. JSON rendering

**xrpld.**
- A JSON object is a `std::map`
  ([`json_value.h` L162](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/json/json_value.h#L162)):
  `using ObjectValues = std::map<CZString, Value>;`
- `STObject::getJson` inserts every field into that map
  ([`STObject.cpp` L845-L855](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/protocol/STObject.cpp#L845-L855)).
  `STLedgerEntry::getJson` adds `index`
  ([`STLedgerEntry.cpp` L117-L121](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/protocol/STLedgerEntry.cpp#L117-L121)).
- Each entry of a `ledger_data` page
  ([`LedgerData.cpp` L124](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/rpc/handlers/ledger/LedgerData.cpp#L124))
  and each `book_offers` offer
  ([`NetworkOPs.cpp` L4935](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/misc/NetworkOPs.cpp#L4935))
  becomes such a tree, which is then serialized.

**quaxar before.** The same approach, with a Rust `BTreeMap` tree.

**quaxar after**
([`902cdc03`](https://github.com/TusharPardhe/quaxar/commit/902cdc03),
[`9c7ca826`](https://github.com/TusharPardhe/quaxar/commit/9c7ca826),
[`json_writer.rs`](../../xrpl/protocol/src/serialization/json_writer.rs)).
The JSON text is written directly from the decoded object; no tree is built.

JSON keys must come out sorted. Each object type's template is given its
sorted field order once, at startup
([`so_template.rs`](../../xrpl/protocol/src/serialization/so_template.rs)
`json_order`). Writing is then a walk through that list, with no map and no
sorting.

The rendered text is carried inside the result as `JsonValue::Raw`
([`stbase.rs`](../../xrpl/protocol/src/serialization/stbase.rs)), so it is
never rebuilt into a tree.

```
xrpld / quaxar before:
  Offer ─► map{ "Account": node, "BookDirectory": node, … }   one allocation per field
        ─► serialize ─► free the map

quaxar after:
  Offer ─► walk precomputed order: Account(slot 9) → BookDirectory(5) → BookNode(2) → Flags(0) …
        ─► {"Account":"r…","BookDirectory":"…","BookNode":"0","Flags":0,…}   written once
```

**Example.** A `book_offers` reply with 200 offers renders 200 objects. Xrpld
builds and frees 200 map trees with about 13 nodes each; quaxar appends 200
pieces of text to one buffer.

**Measured.**
- On 8,592 real ledger entries: 2.7 µs → 1.2 µs per entry (2.2x).
- Metadata: 9.8 µs → 4.2 µs (2.3x). Full table in
  [BENCHMARKS.md](BENCHMARKS.md#rendering-micro-benchmarks).

**Correctness.**
- The output is asserted byte-identical to the tree path on the whole corpus
  of 8,592 entries in 22 types, plus 662 transactions with metadata
  ([`json_writer_corpus.rs`](../../xrpl/protocol/tests/serialization/json_writer_corpus.rs)).
  The same test runs on 25,000 corrupted records.
- Endpoint-level byte identity is checked for `book_offers`,
  `account_objects`, `account_info`, `ledger_data`, `tx` and `ledger`
  ([`raw_rendering_parity.rs`](../../xrpld/rpc-integration-tests/tests/handlers/raw_rendering_parity.rs)),
  and for the transactions-stream event.

## 4. Decoding ledger objects

**xrpld.**
- An object points at its template rather than copying it
  ([`STObject.h` L72](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/protocol/STObject.h#L72)):
  `SOTemplate const* type_{};`.
- Applying the template searches the remaining fields once per template
  element, then erases the match
  ([`STObject.cpp` L158-L205](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/protocol/STObject.cpp#L158-L205),
  `find_if` at L173, `erase` at L182). That is O(n²) in the number of fields.

**quaxar before.**
- Every decoded object received its own copy of the template, including a
  per-field index table sized to all field types.
- Template application cloned all fields, then searched and removed them one
  by one.

**quaxar after**
([`c18a1bc2`](https://github.com/TusharPardhe/quaxar/commit/c18a1bc2),
[`so_template.rs`](../../xrpl/protocol/src/serialization/so_template.rs),
[`st_object.rs`](../../xrpl/protocol/src/serialization/st_object.rs)).
- Template storage is shared (`Arc`), so a copy is a reference-count bump.
- Each field is placed directly into its template slot, in O(n), with the
  same first-occurrence, discardable-field and required-default rules.

```
xrpld:          for each of 13 template fields: scan the fields, erase the match   ~ n²/2 comparisons
quaxar after:   for each field: slot = index[field]; put it there                  n steps
```

**Example.** An AccountRoot with 13 fields needs up to 91 comparisons with
the search approach (13 + 12 + … + 1). Direct placement needs 13 slot lookups.

**Measured.**
- `book_offers` with 200 offers: 4.5 ms → 2.3 ms in the endpoint harness
  (`examples/endpoint_render.rs`). That commit also replaced per-step SHAMap
  depth-mask computation with a table, and the two changes were measured
  together.
- Correctness: the O(n) placement is checked against the previous algorithm
  on 5,000 shuffled, partial, duplicated and foreign field sets
  (`apply_template_equivalence_tests` in `st_object.rs`).

## 5. Base58 addresses and hex hashes

**xrpld.**
- Base58 uses `b58_fast`, which converts through base 58^10
  ([`tokens.cpp` L109-L121](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/protocol/tokens.cpp#L109-L121),
  "10x-15x faster"; implementation from
  [L349](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/protocol/tokens.cpp#L349)).
- Its long division divides an `unsigned __int128`
  ([`b58_utils.h` L129-L131](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/protocol/detail/b58_utils.h#L129-L131)):
  `num / denom128`. Compilers lower that to the runtime routine
  `__udivti3`.
- Hex uses `boost::algorithm::hex`
  ([`strHex.h` L22](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/basics/strHex.h#L22)).

**quaxar before.**
- Base58 used the generic `bs58` crate, which produces one digit per pass.
- Hex called `format!("{:02X}")` per byte, which is 32 heap allocations per
  hash.

**quaxar after**
([`4e756eb3`](https://github.com/TusharPardhe/quaxar/commit/4e756eb3),
[`b58_fast.rs`](../../xrpl/protocol/src/base/b58_fast.rs);
[`812577d7`](https://github.com/TusharPardhe/quaxar/commit/812577d7),
[`str_hex.rs`](../../xrpl/basics/src/string/str_hex.rs)).
- Base58 is a port of xrpld's `b58_fast`.
- The division by 58^10 multiplies by a precomputed reciprocal instead of
  calling `__udivti3` (Möller & Granlund, IEEE Trans. Computers 60(2), 2011).
  Disassembly confirmed the first port did call `__udivti3`.
- Hex uses a 256-entry pair table.

```
generic base58:   big number ÷ 58, one digit per pass        × 34 passes
b58_fast:         big number ÷ 58^10, ten digits per pass    ×  4 passes
quaxar:           b58_fast, each ÷ 58^10 as multiply + shift
```

**Measured.**
- Encoding a 25-byte address payload: 1,058 ns → 157 ns.
- One hash to hex: 1,625 ns → 51 ns.
- This is parity with xrpld's algorithms, plus the division change.
- Correctness: identical to `bs58` on 78,000 random inputs of every length up
  to 38 bytes and on the genesis, `ACCOUNT_ZERO` and `ACCOUNT_ONE` addresses.
  The reciprocal division was checked against hardware division a million
  times.

## 6. SHAMap child locks

**xrpld** locks one bit per child slot of an inner node
([`PackedSpinlock`, `spinlock.h` L77-L130](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/basics/spinlock.h#L77-L130)),
in `getChild`
([`SHAMapInnerNode.cpp` L341-L352](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/shamap/SHAMapInnerNode.cpp#L341-L352))
and `canonicalizeChild`
([L365-L381](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/shamap/SHAMapInnerNode.cpp#L365-L381)).

**quaxar before.** Every child read took the whole 16-bit lock of the node.
Concurrent ledger reads therefore queued on the root.

**quaxar after**
([`cd4e981f`](https://github.com/TusharPardhe/quaxar/commit/cd4e981f),
[`tree_node.rs`](../../xrpl/shamap/src/nodes/tree_node.rs)).
- Child reads and canonicalization take one bit, as in xrpld. Layout-changing
  operations still take the whole word.
- Waiters spin on loads with bounded backoff and then yield the CPU.

```
before (quaxar):            after (quaxar, same as xrpld):
   root [1 lock]               root [16 lock bits]
   ▲   ▲   ▲   ▲               ▲ b3   ▲ b7   ▲ b3
  T1  T2  T3  T4 (wait)       T1     T2     T3 (waits only for T1)
```

**Measured.**
- Stack samples under 16 concurrent `ledger_data` calls showed every worker
  inside the root lock. After the change, 16-connection `ledger_data` went
  from 153 to 324 rps (native, 4-CPU container).
- Correctness: a concurrency test runs 8 threads × 20,000 mixed lock
  operations on one node and checks that all bits are released.

## 7. RPC worker count

**xrpld.** RPC work runs on the job queue
([`Application.cpp` L330-L345](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/main/Application.cpp#L330-L345)),
which has 1 thread in standalone mode and is sized from
`hardware_concurrency()` otherwise.

**quaxar before.** Semaphores allowed 64 general and 16 `ledger_data`
handlers to run at once on the blocking pool, regardless of CPU count.

**quaxar after**
([`bfadfac7`](https://github.com/TusharPardhe/quaxar/commit/bfadfac7),
[`00f736b7`](https://github.com/TusharPardhe/quaxar/commit/00f736b7),
[`router.rs`](../../xrpld/server/src/transport/router.rs)).
- Methods that walk many entries (`ledger_data`, `book_offers`,
  `account_objects`, `account_tx`, `account_lines`, `ledger`, …) share a lane
  of one handler per CPU.
- Point lookups get a lane of two per CPU, so they are not queued behind
  heavy pages.
- `QUAXAR_RPC_HANDLER_THREADS` overrides the size.

```
before:  64 heavy handlers on 4 CPUs  →  threads fight for the same tree nodes
after:   heavy lane: 4 at a time      →  each runs to completion quickly
         light lane: 8 at a time      →  account_info is not stuck behind ledger_data
```

**Measured.** At 16 connections, `ledger_data` went from 324 to 656 rps and
its p99 from 118 ms to 30 ms. `account_info` stayed at about 43k-61k rps.

## 8. `ledger_data` paging

**xrpld** walks a page with one forward iterator:
`for (auto i = lpLedger->sles.upperBound(key); i != e; ++i)`
([`LedgerData.cpp` L102-L103](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/rpc/handlers/ledger/LedgerData.cpp#L102-L103)).
That iterator is the SHAMap's own iterator
([`Ledger.cpp` L438-L441](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/ledger/Ledger.cpp#L438-L441)).

**quaxar before.** For every entry, it called `succ` (a search from the
root) and then `read` (a second search from the root).

**quaxar after**
([`b9c7c66b`](https://github.com/TusharPardhe/quaxar/commit/b9c7c66b),
`Ledger::state_entries_after` in [`ledger/src/lib.rs`](../../xrpld/ledger/src/lib.rs)).
- The page is collected in one forward walk of the state map, which is the
  same idea as xrpld's iterator.
- The old per-entry loop remains the fallback for the open ledger or a
  missing node.

```
before (quaxar), per entry ×256:   root→…→leaf (find next key)  +  root→…→leaf (read it)
after  (quaxar, like xrpld):       root→…→first leaf → next leaf → next leaf …
```

**Example.** A 256-entry page used to be 512 descents from the root; it is
now one descent and 255 steps along the leaves.

**Measured.** 16-connection `ledger_data` went from 706 to 2,306 rps, and
1-connection from 560 to 736 (native).

## 9. `book_offers` directories

**xrpld**, in `getBookPage`, finds each quality directory with `view.succ`
and then reads it
([`NetworkOPs.cpp` L4836-L4839](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/misc/NetworkOPs.cpp#L4836-L4839)):

```cpp
auto const ledgerIndex = view.succ(uTipIndex, uBookEnd);
...
sleOfferDir = view.read(keylet::page(*ledgerIndex));
```

**quaxar before.** The same: `succ` plus `read` per directory.

**quaxar after**
([`b9c7c66b`](https://github.com/TusharPardhe/quaxar/commit/b9c7c66b),
[`app_server_info.rs`](../../xrpld/rpc/src/state/app_server_info.rs)).
A book's quality directories are consecutive keys, so they are read in
forward walks of up to 64 directories each. The walk stops at the book's end
key or at any non-directory entry, which are the same stop rules as
`succ(tip, book_end)`.

```
xrpld / quaxar before, per price level:   succ(tip) → read(directory) → read offers
quaxar after:                             walk(tip, 64 directories) → read offers
```

**Example.** The benchmark book has 200 prices, so 200 directories. That was
200 successor searches plus 200 directory reads; it is now about 4 walks.

**Measured.** 16-connection `book_offers` went from 664 to 2,171 rps, and
1-connection from 455 to 697 (native). Against xrpld: 7.8x-16.2x.

## 10. `account_tx`

**xrpld**
([`SQLiteDatabase.cpp` L449-L476](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/rdb/backend/detail/SQLiteDatabase.cpp#L449-L476)):
1. Checks out the transaction database session with
   `auto db = checkoutTransaction();`. That session holds a
   `std::recursive_mutex`
   ([`DatabaseCon.h` L28-L39](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/rdb/DatabaseCon.h#L28-L39),
   [L177-L183](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/rdb/DatabaseCon.h#L177-L183)).
2. While holding it, runs the page query. For every row the query calls
   `onTransaction`
   ([`Node.cpp` L1281](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/rdb/backend/detail/Node.cpp#L1281)),
   which runs `convertBlobsToTxResult` and so decodes the transaction and
   its metadata.

**quaxar before.** The same: decoding happened while the shared connection
was locked.

**quaxar after**
([`b9c7c66b`](https://github.com/TusharPardhe/quaxar/commit/b9c7c66b),
[`app_server_info.rs`](../../xrpld/rpc/src/state/app_server_info.rs)).
Raw rows are collected under the lock; the lock is released before any
decoding.

```
xrpld:   [lock ── query ── decode 200 txs + metadata ── unlock]     others wait for all of it
quaxar:  [lock ── query ── unlock] ── decode 200 txs + metadata     others wait for the query only
```

**Measured.** 16-connection `account_tx` went from 479 to 892 rps (native).
Against xrpld: 6.0x-7.2x.

## 11. WebSocket slow clients

**xrpld** closes a client whose send queue exceeds `send_queue_limit`
([`BaseWSPeer.h` L218-L221](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/server/detail/BaseWSPeer.h#L218-L221),
"Policy error: client is too slow."). The default limit is 100
([`Port.cpp` L285](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/libxrpl/server/Port.cpp#L285)).

**quaxar before.** The send queue was unbounded. A subscriber that fell
behind the broadcast ring silently skipped events.

**quaxar after**
([`fa88840d`](https://github.com/TusharPardhe/quaxar/commit/fa88840d),
[`session.rs`](../../xrpld/server/src/transport/session.rs)).
- The queue holds 100 messages, and slow clients are closed with the same
  policy close.
- Queued frames are flushed in batches of up to 64.
- Each stream's broadcast ring holds 1024 events, which absorbs the burst of
  events at ledger close.

**Measured.** In a fan-out test (200 subscribers × 500 events), delivery went
from 11.8% to 100%. This is parity with xrpld's behaviour, and correctness
rather than speed.

## 12. Standalone ledger close

**xrpld** updates the skip list on every close
([`BuildLedger.cpp` L64](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/ledger/detail/BuildLedger.cpp#L64)).
`RCLValidatedLedger` checks that the skip list's last sequence is `seq - 1`
with `XRPL_ASSERT`
([`RCLValidations.cpp` L42](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/src/xrpld/app/consensus/RCLValidations.cpp#L42)).
That macro is a plain `assert`
([`instrumentation.h` L17-L23](https://github.com/XRPLF/rippled/blob/4a4fded2eba11427c48ce3f24d9c1aea5e7a9d17/include/xrpl/beast/utility/instrumentation.h#L17-L23)),
which release builds compile out.

**quaxar before.** Standalone `ledger_accept` did not update the skip list,
and the check was an unconditional assertion. As a result, `submit` after an
empty close crashed the node.

**quaxar after**
([`07b34452`](https://github.com/TusharPardhe/quaxar/commit/07b34452)).
The skip list is updated on every standalone close, as in xrpld. The check
is a debug assertion plus a warning.

A regression test,
[`standalone_close.rs`](../../xrpld/rpc-integration-tests/tests/transactions/standalone_close.rs),
covers this case. Without the fix, neither standalone server could have been
benchmarked.

---

## How the improvements combine

`book_offers` touches nearly every item above:

```
request ─► [1] parse once ─► [7] heavy lane ─► [9] walk directories ─► [6] per-child locks
        ─► [4] decode offers (O(n)) ─► [5] fast addresses ─► [3] write JSON directly
        ─► [1] serialize once ─► [2] no Nagle delay ─► client
```

WebSocket at 16 connections: xrpld (best configuration) 95 rps, quaxar
1,539 rps.

## Test status

- All tests ran in Docker (`infra/bench/Dockerfile.dev`): 16,665 workspace
  tests pass.
- Every remaining failure either already fails on `origin/main` or is a
  timing-sensitive test that passes when run on its own.
- The CI clippy correctness gate is clean.
