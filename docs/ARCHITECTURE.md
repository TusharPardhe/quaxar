# Architecture

Quaxar is a Rust implementation of an XRP Ledger server. Its runtime follows
the ownership and sequencing model used by `rippled`, while using Rust crates,
channels, and explicit shared-state boundaries. The `xrpld/` directory name is
retained intentionally so source paths remain easy to compare with upstream;
installed programs, packages, configuration, metrics, and services use the
Quaxar name.

## Runtime ownership

`ApplicationRoot` owns the application-wide services and long-lived state. It
wires the overlay, PeerFinder, resource manager, JobQueue, NodeStore, SHAMap
`NodeFamily`, inbound acquisition registries, LedgerMaster state, validations,
validator lists, consensus, RPC, and shutdown tree.

Network and consensus mutations are serialized through the application-owned
NetworkOps strand. Work that may block or run in parallel is admitted to the
JobQueue or a bounded subsystem worker; completion is handed back to the owner
that is allowed to publish the result. This avoids competing owners for the
closed, validated, and published ledger heads.

The main runtime also owns HTTP/WebSocket listeners, the peer overlay runtime,
periodic maintenance, and ordered shutdown. Instrumentation records through
the metrics crate, but the current bootstrap does not initialize a Prometheus
exporter. Components register with the stop tree so producers stop before the
consumers and storage they use.

## Ledger flow

```text
peer messages and local transactions
                |
                v
       overlay dispatch / JobQueue
                |
                v
       NetworkOps strand (single owner)
          |                 |
          |                 +--> validations and preferred-ledger choice
          v
    inbound ledger / transaction-set acquisition
          |
          +--> shared NodeFamily caches and NodeStore reads
          +--> bounded peer requests and fetch-pack reuse
          v
    complete immutable state and transaction SHAMaps
          v
    LedgerMaster: closed -> validated -> published
          v
    consensus, RPC views, history, relational metadata
```

An inbound-ledger registry admits one active acquisition for a ledger hash.
Acquisition state is retained between rounds, so partial SHAMaps and fetched
nodes can be reused rather than rebuilding each tree from an empty map. Reads
consult shared in-memory caches and the NodeStore before requesting missing
nodes from peers. A ledger is eligible for promotion only after its header and
required SHAMaps are complete and consistent with their hashes.

LedgerMaster owns advancement of the closed, validated, and published heads.
Validation arrival invokes its acceptance checks; publication advances in
sequence and does not silently cross an unresolved gap. During recovery,
historical acquisition is subordinate to acquiring and publishing the current
validated chain.

See [SYNCING.md](SYNCING.md) for the operator-visible state transitions.

## Storage and caches

- `NodeStore` persists immutable ledger objects, normally in NuDB.
- `NodeFamily` supplies the shared tree-node and full-below caches used by
  ledger acquisition and SHAMap reads.
- LedgerMaster owns the fetch-pack cache; NodeStore and acquisition services
  own their read, negative, and write-dedup caches.
- Ledger history indexes complete ledger objects by sequence and hash.
- Relational databases store ledger and transaction metadata used by RPC.
- Online deletion prunes retained history according to configuration; it is
  independent of the short-lived acquisition registry.

Cache limits and sweep cadence derive primarily from `[node_size]`. Normal
closed-ledger advancement does not clear the NodeFamily. Periodic application
maintenance performs bounded sweeping based on cache age and size.

## Networking and trust

The overlay handles encrypted peer sessions, XRPL handshakes, message framing,
relay policy, and endpoint exchange. PeerFinder owns slot accounting,
boot-cache selection, fixed peers, connection attempts, and inbound/outbound
budgets. The resource manager applies per-client load accounting before work is
accepted.

Validator-list publishers, configured validators, manifests, revocations, and
the local validation identity are application services. Peer connectivity does
not establish trust: only trusted validations contribute to preferred-ledger
and quorum decisions.

## Crate map

| Path | Responsibility |
|------|----------------|
| `xrpl/basics` | Configuration, time, base integers, utility types |
| `xrpl/core` | XRPL cryptography and key/seed handling |
| `xrpl/protocol` | Serialized types, protocol fields, messages and amendments |
| `xrpl/shamap` | SHAMap nodes, traversal, synchronization and caches |
| `xrpl/resource` | Resource/load accounting |
| `xrpld/acquisition` | Generic acquisition scheduling primitives |
| `xrpld/app` | Application owner, bootstrap, NetworkOps, jobs and orchestration |
| `xrpld/consensus` | Consensus algorithm and timing primitives |
| `xrpld/ledger` | Ledger domain model, history, acquisition and LedgerMaster |
| `xrpld/overlay` | Peer protocol, PeerFinder and overlay runtime |
| `xrpld/nodestore` | Immutable node-object storage and NuDB backend |
| `xrpld/rdb` | Relational ledger/transaction metadata |
| `xrpld/tx` | Transaction checks, application and queueing |
| `xrpld/rpc` | RPC handlers, request roles and subscriptions |
| `xrpld/server` | HTTP/WebSocket transport and RPC dispatch |
| `xrpld/perflog` | Runtime activity and performance counters |
| `xrpld/metrics` | Prometheus metrics (`quaxar-metrics` package) |
| `xrpld/cli` | Operator CLI (`quaxar-cli` package) |
| `xrpld/main` | Bootstrap and `quaxar` executable (`quaxar-main` package) |

## Concurrency rules

When changing the runtime, preserve these invariants:

1. One owner mutates network/consensus lifecycle state.
2. One active inbound acquisition exists per object hash.
3. Blocking work runs outside the owner strand and returns through an explicit
   completion path.
4. Ledger heads only reference complete, immutable ledgers.
5. Caches are shared and swept by policy; they are not cleared on every ledger.
6. Shutdown stops producers before queues, storage, and shared state.

For behavior-parity changes, compare the corresponding `rippled` owner and
call sequence, not just the shape of an individual function.
