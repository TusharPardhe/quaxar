# Synchronization

Quaxar can join an existing XRPL network from an empty database. Synchronizing
means acquiring a validated ledger header plus its complete state and
transaction SHAMaps, persisting the required nodes, and advancing LedgerMaster
onto the network's validated chain.

## Server states

The normal progression is:

```text
disconnected -> connected -> syncing -> tracking -> full
                              ^                 |     |
                              +-----------------+     +-> proposing (validator gates satisfied)
```

- `connected`: enough peer connectivity exists to observe the network, but no
  suitable current ledger is installed.
- `syncing`: the node is acquiring or reconciling the preferred validated
  ledger and its maps.
- `tracking`: the node has a recent validated chain and is following closes,
  but has not yet satisfied every readiness gate.
- `full`: the canonical local closed ledger is current, and validated and
  published heads are advancing coherently within the readiness bounds.
- `proposing`: a configured validator is current and participating after the
  full-readiness, trust/quorum, clock, and validation gates pass. Whether
  other operators trust it is a separate validator-list decision.

An actionable branch change can legitimately produce
`full -> syncing -> tracking -> full`. Repeated flapping, growing local-closed
versus validated lag, a static coordinator identity, or acquisitions that
continually reset require investigation.

On an empty database, Quaxar may temporarily close a small bootstrap chain
while the preferred network ledger's state tree is still being assembled. In
that interval `server_state` can remain `connected`, local sequence numbers are
not network progress, and consensus diagnostics report a wrong LCL. Readiness
starts only after a complete preferred ledger is installed and the canonical
validated/published chain begins advancing.

## Current-ledger acquisition

1. Trusted validations and peer status select a preferred ledger hash.
2. The typed coordinator records a stable recovery anchor. The latest
   preferred policy may continue moving and prioritize independent per-hash
   acquisitions without replacing that anchor.
3. The ledger header identifies state-map and transaction-map roots.
4. Each map consults the shared NodeFamily/tree cache, fetch-pack cache, and
   NodeStore before requesting missing nodes from peers.
5. Peer replies are validated by hash and inserted into the incomplete tree.
   A retained session preserves its exact frontier across retries; after
   cleanup, verified nodes remain reusable through shared caches and storage.
6. Once both maps are complete, immutable, and match their roots, the ledger is
   published by exact hash to history, persisted, and handed back through the
   coordinator's durable completion path. Validation-trie residency is updated
   only for validators still waiting on that exact sequence and hash. If a
   newer validation already replaced the waiter, a late older completion stays
   reusable in history/storage but does not revive superseded trie support;
   durability failure retracts provisional support.
7. At the serialized accepted boundary, NetworkOps recomputes current
   preference and installs only the complete, current-compatible preferred
   ledger. LedgerMaster then applies validation
   gates and advances validated/publication state in sequence.

A hash-only anchor is refined only by a verified header for that same hash;
later hash-only observations cannot discard its sequence. Installing it moves
the coordinator to `tracking`. A published head may be ahead of or behind the
installed LCL; proof that both lie on the same chain plus fresh current-open
timing permits `full`. Subsequent accepted closes and publications advance the
phase's LCL and published identities independently. The independent
`need_network_ledger` startup/recovery latch clears only after an authoritative
LCL switch or full-ledger publication, not merely because a fetch completed.

Publication observations and mode promotion are deliberately separate.
NetworkOps forwards a chain-contiguous LedgerMaster publication once an LCL is
installed. In `tracking`, only a non-divergent observation with fresh
open-ledger timing promotes to `full` and records the published identity. Once
`full`, newer contiguous publication identities are recorded even on a
non-promoting pass.

Per-hash demand is coalesced while bounded schedulers, worker pools, read
brokers, mailboxes, and outbound credits control execution. The number of
coordinator session identities is not an operator-configured `node_size`
limit. `[node_size]` controls tree-cache capacity and age, maintenance cadence,
the adjacent history-fetch window, and selected JobQueue defaults.
An acquisition session can expire or be replaced without erasing its verified
nodes: the registry is short-lived ownership, while NodeFamily and NodeStore
are reusable storage. During an empty-database catch-up, RSS commonly rises as
these shared caches fill and can later level off or fall after age/size sweeps.

## Preferred-ledger changes

The preferred hash can change while a node catches up. NetworkOps updates that
moving policy and acquisition priority while the coordinator preserves the
active recovery anchor. Older and newer sessions may finish independently;
retained sessions keep their frontiers and cleaned-up sessions still contribute
verified nodes through shared caches and storage. After the stable anchor is
installed, NetworkOps reconciles the current preferred policy again. A header
or completed tree alone never authorizes installation or publication.

Consensus timer view changes do not pin their sampled preferred hash as a
recovery target. They only request the reference mode demotion; the serialized
`checkLastClosedLedger` path selects, verifies, and acquires an actionable
target. This prevents a transient timer observation from becoming a durable
recovery anchor.

## History acquisition

After the current chain is established, configured history can be filled in
without blocking live validated-ledger advancement. `[ledger_history]` controls
the desired history and `[node_db] online_delete` controls retained history.
During recovery, current-ledger work has priority over backfill.

## Monitoring

```bash
quaxar status
quaxar sync-status
quaxar ledger-closed
quaxar ledger-current
quaxar fetch-info
quaxar get-counts
quaxar server-info
```

Compare repeated samples of `ledger_closed`, the open-ledger index from
`ledger_current`, the validated and published heads, the coordinator LCL and
published identities at `result.info.coordinator.phase` in raw `fetch_info`
(`info.coordinator.phase` in CLI output), session completion counters, and mode
transitions. A single `full` or
`proposing` result is not readiness proof. Memory use alone is not a correctness signal:
it varies with `[node_size]`, cache occupancy, allocator behavior, history, and
network activity. Judge progress primarily by completed maps, advancement of
the validated/published sequence, and stable state transitions.

During initial acquisition, distinguish cache growth from completion. Rising
RSS and NodeStore writes show useful work but do not prove either root is
complete. Conversely, a bounded RSS plateau is normal after the cache profile
reaches steady state. Require a current canonical LCL plus consecutive
validated and published advances.

For logs, temporarily raise the relevant partition and restore it afterward:

```bash
quaxar log-level debug
journalctl -u quaxar.service -f
quaxar log-level info
```

For a typed coordinator comparison during a controlled diagnostic restart, set
`QUAXAR_ACQUISITION_SHADOW=1` in the service environment. The shadow is
read-only and bounded; remove the variable after collecting the comparison.

## Diagnosing a stuck node

Check these in order:

1. `quaxar peers` shows stable, protocol-compatible peers.
2. `quaxar validator-list-sites` and `quaxar unl-list` show usable trust data.
3. `quaxar fetch-info` shows acquisitions receiving nodes rather than being
   repeatedly recreated.
4. `quaxar get-counts` shows SHAMap/NodeStore activity and bounded caches.
5. Local closed, validated, published, and coordinator identities do not show
   growing lag or remain static while other heads advance.
6. Logs do not show recurring Full/Syncing transitions; inspect
   `last_recovery_lcl_decision` and session completion/cancellation counters.
7. The configured database and log directories are writable by the service
   account and disk space is available.
8. Host time is synchronized.
9. `network_id`, validator publishers, and fixed peers belong to the same
   network.

Capture `server_info`, `fetch_info`, `get_counts`, recent service logs, the
commit from `quaxar version`, and a redacted config when reporting a defect.
Always remove validation seeds and other private credentials.
