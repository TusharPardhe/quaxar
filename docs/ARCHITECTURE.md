# Architecture

Quaxar is a Rust implementation of an XRP Ledger server. Its runtime follows
the ownership and sequencing model used by `rippled`, while using Rust crates,
channels, and explicit shared-state boundaries. The `xrpld/` directory name is
retained intentionally so source paths remain easy to compare with upstream;
operator-facing package prefixes, installed programs, configuration, metrics,
and services use the Quaxar name. Generic internal crates keep concise domain
names such as `ledger`, `rpc`, and `server`.

## Runtime ownership

`ApplicationRoot` owns the application-wide services and long-lived state. It
wires the overlay, PeerFinder, resource manager, JobQueue, NodeStore, SHAMap
`NodeFamily`, the typed acquisition coordinator and per-hash registries,
LedgerMaster state, validations, validator lists, consensus, RPC, and shutdown
tree.

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
          |
          +--> moving preferred-LCL policy
          |          |
          |          +--> independent per-hash acquisitions
          |          v
          +--> stable recovery anchor / coordinator phase
                     |
                     v
    per-hash ledger / transaction-set acquisition
          |
          +--> shared NodeFamily caches and NodeStore reads
          +--> bounded peer requests and fetch-pack reuse
          v
    complete immutable state and transaction SHAMaps
          v
    durable completion handoff -> NetworkOps LCL reconciliation
          |
          v
    LedgerMaster: validated -> published
          v
    consensus, RPC views, history, relational metadata
```

The acquisition coordinator serializes typed events into phase transitions and
effects. While syncing, its phase owns a stable recovery anchor. NetworkOps'
latest preferred-LCL policy is separate moving state and may prioritize another
per-hash acquisition without replacing that anchor. Only a verified header for
the same hash may refine a hash-only anchor with its sequence. The coordinator
also owns retry policy, durable completion handoff, and per-hash session
lifecycle. Older sessions may finish after policy advances; their partial
SHAMaps and fetched nodes remain reusable through shared caches and NodeStore.

NetworkOps owns the closed-ledger/LCL switch and reconciles it with the latest
preferred-ledger policy. LedgerMaster owns validation acceptance plus validated
and published advancement; publication advances in sequence and does not
silently cross an unresolved gap. The application/NetworkOps
`need_network_ledger` startup/recovery latch is independent from the
coordinator's visible phase and clears only on an authoritative LCL switch or
full-ledger publication. Accepted local closes and contiguous publications
advance the coordinator's separate LCL and publication identities. Once an LCL
is installed, ordinary Consensus, Generic, and History acquisitions are
phase-neutral; only a serialized actionable preferred-LCL divergence demotes
Tracking or Full. During recovery, historical acquisition is subordinate to
acquiring and publishing the current validated chain.

See [SYNCING.md](SYNCING.md) for the operator-visible state transitions.

## Payment and offer flow

Path/offer execution creates one shared AMM context for the complete flow, not
one context per `BookStep`. Reverse and forward book passes receive that same
context. It records the transaction account, whether more than one strand is
active, the selected strand's AMM-use/iteration state, and the AMM's initial
balances in both book directions. Candidate state is cleared before each strand
is evaluated and committed only for the strand that is selected; discarded
candidates therefore cannot consume the multipath AMM iteration budget.

Within a book quality, AMM liquidity is considered before the CLOB offer, as in
`rippled`. Single-path maximum offers use pool spot quality. CLOB-targeted AMM
offers use their generated amount quality, account-aware auction-slot fees, and
the active AMM amendments. Multipath execution derives its bounded Fibonacci
offers from the shared initial balances and selected-iteration state. These
ordering and ownership rules are consensus-critical: changing them can produce
a different transaction result and ledger hash even when the same offers and
pool balances are present.

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
| `xrpld/acquisition` | Typed coordinator, phase machine, sessions, retries and durable handoff |
| `xrpld/app` | Application owner, bootstrap, NetworkOps, jobs and orchestration |
| `xrpld/core` | Port configuration, RPC status and SQLite state utilities (`quaxar-core`) |
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

`xrpld/rpc-integration-tests` is a test-only workspace member and is not a
runtime package.

## Concurrency rules

When changing the runtime, preserve these invariants:

1. One owner mutates network/consensus lifecycle state.
2. One active inbound acquisition exists per object hash.
3. Blocking work runs outside the owner strand and returns through an explicit
   completion path.
4. Ledger heads only reference complete, immutable ledgers.
5. Caches are shared and swept by policy; they are not cleared on every ledger.
6. Shutdown stops producers before queues, storage, and shared state.
7. A moving preferred tip never replaces an in-flight stable recovery anchor.
8. The phase LCL tracks the canonical local closed ledger, while publication
   advances independently only on the proven contiguous chain.
9. Full requires coherent advancing LCL/publication state, not merely a
   completed acquisition.

For behavior-parity changes, compare the corresponding `rippled` owner and
call sequence, not just the shape of an individual function.
