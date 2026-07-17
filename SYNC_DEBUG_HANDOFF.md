# Quaxar Sync Debugging — Full Handoff Prompt

## Objective

Debug and fix why quaxar (Rust XRPL node) cannot sustain ledger state acquisition beyond the initial ~100MB burst. The node gets the ledger header and root state nodes, but follow-up requests for deeper state tree nodes get no response from testnet peers.

## Proven Facts

1. **Peers respond to `ltype=ltCLOSED` (no hash) header requests** — within 59ms, reliably
2. **Initial burst works** — 100MB of data written to NuDB in first 10 seconds (header + root nodes from `sendLedgerBase`)
3. **Follow-up `liAS_NODE` requests (with specific nodeIDs + ledger_hash) get ZERO responses** from all 3 testnet peers
4. **The peer DOES keep recent ledgers** — rippled's `LedgerMaster` retains last `ledger_history` (256) ledgers, so our hash should be findable for ~15 minutes
5. **Data IS delivered to the worker** — 16K+ packets routed via fallback, `ever_received_peer_data=true`
6. **We send 4531-byte requests** (128 nodeIDs) that peers receive but don't respond to
7. **Memory is fixed** — stable at ~10GB (jemalloc + skip guard)

## The Specific Problem

After receiving the initial `sendLedgerBase` response (header + state root + tx root), the worker calls `trigger_with_family(Reply)` which sends `liAS_NODE` requests containing up to 128 specific nodeIDs. These requests include `ledger_hash` (the hash we committed to from the `ltCLOSED` response). The peer's `processLedgerRequest` calls `getLedger(m)` which does:

```cpp
// rippled PeerImp::getLedger (line 3173 of PeerImp.cpp)
if (m->has_ledgerhash()) {
    ledger = app_.getLedgerMaster().getLedgerByHash(ledgerHash);
    if (!ledger) {
        // if has_querytype → relay, else → silently drop
        return;
    }
}
```

**The peer cannot find our hash via `getLedgerByHash()`** and silently drops the request. This happens despite:
- The hash being the peer's OWN closed ledger from 1-3 seconds ago
- `querytype=qtINDIRECT` being set (enables relay, but relay also fails)
- `ledger_history=256` meaning the peer should keep 256 recent ledgers

## Server Access

```bash
ssh -i ~/.ssh/xrpld-testnet.pem ubuntu@54.172.224.150
```

- **Quaxar repo**: `/home/ubuntu/quaxar`
- **Branch**: `fix/persist-dirty-nodes-to-nudb`
- **Config**: `/home/ubuntu/quaxar-testnet.cfg`
- **DB path**: `/var/lib/quaxar/db/`
- **Logs**: `/tmp/quaxar.log`
- **Build**: `source ~/.cargo/env && ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu cargo build --release -p xrpld-main`
- **Run**: `nohup quaxar --conf /home/ubuntu/quaxar-testnet.cfg > /tmp/quaxar.log 2>&1 &`
- **Clean restart**: `pkill -f 'quaxar --conf'; rm -rf /var/lib/quaxar/db/*; mkdir -p /var/lib/quaxar/db/nudb/xrpldb.0000 /var/lib/quaxar/db/nudb/xrpldb.0001`

## Testing Commands

```bash
# Check server state
curl -s http://127.0.0.1:5005 -d '{"method":"server_info","params":[]}' | python3 -m json.tool

# Check NuDB growth
watch -n 5 'stat --format="%s bytes %y" /var/lib/quaxar/db/nudb/xrpldb.*/nudb.dat'

# Check if data is being delivered
grep -c "fallback delivery OK" /tmp/quaxar.log

# Run with debug tracing (shows packet flow)
RUST_LOG="overlay=debug,inbound_ledger=debug,consensus=trace" timeout 15 quaxar --conf /home/ubuntu/quaxar-testnet.cfg

# Run with overlay trace (shows all message types decoded)
RUST_LOG="overlay=trace" timeout 10 quaxar --conf /home/ubuntu/quaxar-testnet.cfg 2>&1 | grep "msg_type="
```

## Codebase Structure

### Key files in quaxar (`/Users/tusharpardhe/Documents/xrpl/quaxar`):

| File | Purpose |
|------|---------|
| `xrpld/app/src/ledger/shared_inbound_ledgers.rs` | SharedInboundLedgers: worker spawning, routing, acquire() |
| `xrpld/app/src/bootstrap/bootstrap.rs` | Start-mode consensus loop, ledger_data_router callback |
| `xrpld/ledger/src/acquisition/inbound_ledger.rs` | InboundLedgerLocal: request building, data processing |
| `xrpld/overlay/src/runtime/overlay_impl.rs` | Message dispatch (on_ledger_data → inbound_handler) |
| `xrpld/overlay/src/transport/session.rs` | Wire framing, message decode |
| `xrpld/overlay/src/transport/message.rs` | Protobuf encode/decode for TmGetLedger, TmLedgerData |

### Key files in rippled (`/Users/tusharpardhe/Documents/xrpl/rippled`):

| File | Purpose |
|------|---------|
| `src/xrpld/overlay/detail/PeerImp.cpp` | `processLedgerRequest`, `getLedger`, `sendLedgerBase` |
| `src/xrpld/overlay/detail/Tuning.h` | `kDropSendQueue=192`, `kSoftMaxReplyNodes=8192` |
| `src/xrpld/app/ledger/detail/InboundLedger.cpp` | `trigger`, `filterNodes`, `onTimer` |
| `src/xrpld/app/ledger/detail/LedgerMaster.cpp` | `getLedgerByHash`, `checkAccept` |
| `xrpl/protocol/proto/xrpl.proto` | TMGetLedger, TMLedgerData, TMLedgerType definitions |

## All Commits (20 total on branch)

```
0c6885d fix(sync): revert ltype for node requests, register response hash in registry
29beaf3 fix(sync): use ltype=ltCLOSED for state/tx node requests (not hash) [REVERTED]
d893559 fix(sync): add routing diagnostics to identify delivery path
4e5f761 fix(sync): rename worker thread to identify source
59d9c95 fix(sync): add spawn logging to trace worker creation
72df55c fix(sync): use AtomicBool hard gate to enforce single worker
0e49b3b fix(sync): block ALL concurrent acquisitions during initial bootstrap
c9e0402 fix(sync): ensure exactly one acquisition worker during initial bootstrap
722e5bc fix(sync): keep single acquisition worker alive during initial bootstrap
271fba3 fix(sync): exempt first_add_peers from skip-traversal guard
18b3631 fix(sync): route TmLedgerData responses to workers regardless of hash mismatch
7cdfe63 fix(sync): prevent eviction of workers actively receiving data
3c2ffe9 fix(sync): use ltype=ltCLOSED for initial ledger acquisition (seq=0)
8769f77 fix(sync): send peer updates to SharedInboundLedgers workers in start mode
89c3495 fix(sync): always set querytype=qtINDIRECT in GetLedger requests
5e3481d fix(memory): limit fetch_pack_ready to single check_local attempt per worker
a9e780e fix(memory): correct skip-traversal guard logic for fetch_pack_ready
9a16d00 fix(memory): strengthen skip-traversal guard to handle fetch_pack_ready
1eaf31d fix(memory): address unbounded RSS growth from acquisition worker NuDB traversals
a8d5de6 fix(nodestore): attach node_writer before persist_dirty_nodes_to_store
```

## What Needs Investigation

### Theory 1: `getLedgerByHash()` doesn't find the hash
The testnet peer's `LedgerMaster::getLedgerByHash()` returns nullptr for our hash, even though it was the peer's own closed ledger 1-3 seconds ago. Possible reasons:
- `getLedgerByHash` only checks a small in-memory map (not the full 256 ledger history)
- The peer's closed ledger moves from "closed" to "validated" and the lookup path changes
- There's a timing gap between when the peer sends `sendLedgerBase` and when it stores the ledger in LedgerMaster

**To verify**: Read `LedgerMaster::getLedgerByHash()` in rippled to see exactly what it searches. Check if there's a `mHistLedger` vs `mClosedLedger` vs `mValidLedger` distinction.

### Theory 2: Request validation rejects before reaching `getLedger()`
The `onMessage(TMGetLedger)` handler (line 1415 of PeerImp.cpp) does validation BEFORE dispatching to the job queue. Check:
- Line 1472: `ledgerSeq > validLedgerIndex + 10` — we set `ledger_seq` when seq != 0; after accepting the header, seq becomes non-zero (e.g., 19137771). If the PEER's validLedgerIndex is 19137775, then 19137771 < 19137775 + 10, so this check passes. BUT — does our request include `ledger_seq`? Check `make_get_ledger_with_node_ids` — it passes `seq` which becomes `(seq != 0).then_some(seq)`.
- Line 1487: nodeIDs validation — check if our nodeID encoding matches what rippled expects (`deserializeSHAMapNodeID`)

### Theory 3: The nodeID serialization is wrong
Our `node_ids.iter().map(|id| id.get_raw_string()).collect()` produces bytes that rippled's `deserializeSHAMapNodeID()` rejects. This would cause `badData("Invalid SHAMap node ID")` and a resource charge.

**To verify**: Compare `SHAMapNodeId::get_raw_string()` in quaxar vs `SHAMapNodeID::getRawString()` in rippled. Check byte format (it should be depth:1byte + path:variable bytes).

### Theory 4: The peer's `sendQueue` fills up
After sending us `sendLedgerBase` (3 nodes), the peer immediately gets our 128-nodeID request. Processing this through `getNodeFat()` produces up to 8192 nodes. The resulting response message is huge. If the peer's TCP send buffer is full (we're slow to read because of NuDB traversals in other threads), the send queue grows past `kDropSendQueue=192` and subsequent requests are dropped.

**To verify**: Check if reducing the request size (fewer nodeIDs per request, e.g., 8 instead of 128) allows sustained flow. Try setting `REQ_NODES_REPLY` from 128 to 12.

### Theory 5: Multiple workers consuming overlay I/O
There are still 4 `xrpld-acq-process` threads from the `main.rs` `InboundLedgers` path (registered on `ApplicationRoot`). These are separate from our SharedInboundLedgers worker. They may be consuming CPU/I/O and causing backpressure.

**To verify**: Check if `application_root.rs` line 4175-4180 triggers the main.rs `InboundLedgers::acquire()` during start-mode bootstrap. Disable or no-op that path.

## Recommended Approach

1. **Set up a local rippled testnet node** on the same server (different port). Connect quaxar to it via `[ips] localhost 51236`. If quaxar syncs from local rippled, the problem is testnet infrastructure (load throttling). If it still fails, it's a protocol encoding bug.

2. **Compare wire bytes**: Capture a successful rippled-to-rippled `liAS_NODE` request/response using tcpdump between two rippled instances. Compare the exact protobuf bytes with what quaxar sends.

3. **Add a log inside quaxar's processLedgerRequest handler** (which serves OTHER peers' GetLedger requests). When another peer sends us `liAS_NODE`, log whether `getLedger` succeeds. This proves whether our OWN `getLedgerByHash` works for the hash in question.

4. **Reduce request batch size**: Change `REQ_NODES_REPLY` from 128 to 8. If the peer responds to small requests but not large ones, it's a send queue issue.

## Proto Reference

```protobuf
message TMGetLedger {
  required TMLedgerInfoType itype = 1;  // 0=liBASE, 1=liTX_NODE, 2=liAS_NODE, 3=liTS_CANDIDATE
  optional TMLedgerType ltype = 2;      // 0=ltACCEPTED, 2=ltCLOSED
  optional bytes ledgerHash = 3;
  optional uint32 ledgerSeq = 4;
  repeated bytes nodeIDs = 5;
  optional uint64 requestCookie = 6;
  optional TMQueryType queryType = 7;   // 0=qtINDIRECT
  optional uint32 queryDepth = 8;
}

message TMLedgerData {
  required bytes ledgerHash = 1;
  required uint32 ledgerSeq = 2;
  required TMLedgerInfoType type = 3;
  repeated TMLedgerNode nodes = 4;
  optional uint64 requestCookie = 5;
  optional TMReplyError error = 6;
}
```

## Quick Reproduction

```bash
# On the server:
ssh -i ~/.ssh/xrpld-testnet.pem ubuntu@54.172.224.150

# Clean start with tracing:
pkill -f 'quaxar --conf'; sleep 1
rm -rf /var/lib/quaxar/db/*
mkdir -p /var/lib/quaxar/db/nudb/xrpldb.0000 /var/lib/quaxar/db/nudb/xrpldb.0001

# Run with debug (shows data flow for ~30s then stops):
RUST_LOG="inbound_ledger=debug,consensus=trace" timeout 30 quaxar --conf /home/ubuntu/quaxar-testnet.cfg 2>&1 | grep -E "good:|ACQUIRED|router_callback|fallback|Acquisition flow"

# Expected output: "good:8343" lines for first ~5 seconds, then silence.
# The "Acquisition flow" line will show response_delta=0 after the first burst.
```
