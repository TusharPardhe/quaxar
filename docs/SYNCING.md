# Synchronization

Quaxar can join an existing XRPL network from an empty database. Synchronizing
means acquiring a validated ledger header plus its complete state and
transaction SHAMaps, persisting the required nodes, and advancing LedgerMaster
onto the network's validated chain.

## Server states

The normal progression is:

```text
disconnected -> connected -> syncing -> tracking -> full
                                                \-> proposing (validator)
```

- `connected`: enough peer connectivity exists to observe the network, but no
  suitable current ledger is installed.
- `syncing`: the node is acquiring or reconciling the preferred validated
  ledger and its maps.
- `tracking`: the node has a recent validated chain and is following closes,
  but has not yet satisfied every readiness gate.
- `full`: a non-validating node is current and ready to serve its complete
  locally available data.
- `proposing`: a configured validator is current and participating. Whether
  other operators trust it is a separate validator-list decision.

Short transitions or an occasional missing sequence in `complete_ledgers` can
occur while metadata is committed. A state that remains `syncing`, a validated
sequence that does not advance, or acquisition attempts that continually reset
requires investigation.

## Current-ledger acquisition

1. Trusted validations and peer status select a preferred ledger hash.
2. The inbound-ledger registry reuses an existing acquisition for that hash or
   creates one bounded acquisition.
3. The ledger header identifies state-map and transaction-map roots.
4. Each map consults the shared NodeFamily/tree cache, fetch-pack cache, and
   NodeStore before requesting missing nodes from peers.
5. Peer replies are validated by hash and inserted into the incomplete tree.
   Partial trees remain reusable across retries.
6. Once both maps are complete, immutable, and match their roots, the ledger is
   persisted and handed to LedgerMaster.
7. LedgerMaster applies validation/quorum gates, installs the validated head,
   and advances publication in sequence.

The active-acquisition limit controls concurrent targets, not the number of
tree nodes retained. `[node_size]` controls tree-cache capacity and age,
maintenance cadence, the adjacent history-fetch window, and selected JobQueue
defaults.

## Preferred-ledger changes

The preferred hash can change while a node catches up. Quaxar serializes the
selection and promotion decision through NetworkOps. An acquisition for an old
candidate may stop being current, but valid nodes it placed in shared caches or
NodeStore remain available to another tree with the same hashes. The node does
not publish a ledger merely because its header arrived.

## History acquisition

After the current chain is established, configured history can be filled in
without blocking live validated-ledger advancement. `[ledger_history]` controls
the desired history and `[node_db] online_delete` controls retained history.
During recovery, current-ledger work has priority over backfill.

## Monitoring

```bash
quaxar status
quaxar sync-status
quaxar fetch-info
quaxar get-counts
quaxar server-info
```

Useful fields include `server_state`, `validated_ledger`, `complete_ledgers`,
peer count, fetch-pack status, active inbound ledgers, cache counters, and
NodeStore read/write counters. Memory use alone is not a correctness signal:
it varies with `[node_size]`, cache occupancy, allocator behavior, history, and
network activity. Judge progress primarily by completed maps, advancement of
the validated/published sequence, and stable state transitions.

For logs, temporarily raise the relevant partition and restore it afterward:

```bash
quaxar log-level debug
journalctl -u quaxar.service -f
quaxar log-level info
```

## Diagnosing a stuck node

Check these in order:

1. `quaxar peers` shows stable, protocol-compatible peers.
2. `quaxar validator-list-sites` and `quaxar unl-list` show usable trust data.
3. `quaxar fetch-info` shows acquisitions receiving nodes rather than being
   repeatedly recreated.
4. `quaxar get-counts` shows SHAMap/NodeStore activity and bounded caches.
5. The configured database and log directories are writable by the service
   account and disk space is available.
6. Host time is synchronized.
7. `network_id`, validator publishers, and fixed peers belong to the same
   network.

Capture `server_info`, `fetch_info`, `get_counts`, recent service logs, the
commit from `quaxar version`, and a redacted config when reporting a defect.
