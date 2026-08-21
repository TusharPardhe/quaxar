# CLI Reference

## Starting the Node

Start the server with an explicit config file:

```bash
quaxar --conf /etc/quaxar/quaxar.cfg
```

For an isolated lab or explicitly controlled deployment, a minimum validation
quorum can be overridden at startup:

```bash
quaxar --conf /etc/quaxar/quaxar.cfg --quorum 2
```

`--quorum` overrides the quorum derived from the trusted validator list and can
be unsafe on a public network. Use it only when the validator set and failure
model are deliberately controlled.

Running `quaxar` without arguments prints help and exits. Installed services
should pass `--conf /etc/quaxar/quaxar.cfg` explicitly to start the node.
The executable comes from the `quaxar-main` Cargo package and uses the
`quaxar-cli` library; the retained `xrpld/main` source path is not an installed
program name.

## Interactive Mode

Launch an interactive shell with fuzzy search and inline suggestions:

```bash
quaxar cli
```

Features:
- Type to filter — suggestions appear below the prompt with descriptions
- Arrow keys to scroll through the suggestion list
- Tab to autocomplete the selected command
- Enter to execute
- Command history (Up arrow when no suggestions visible)
- Ctrl+C to exit

## Subcommands

| Command | Description |
|---------|-------------|
| `status` | Point-in-time server state, uptime, peers, and complete-ledger range |
| `health` | RPC reachability with a point-in-time state label; not a readiness gate |
| `peers` | Connected peers with latency and version |
| `fee` | Current transaction fee |
| `ledger [seq]` | Ledger info (latest validated or by sequence) |
| `account <address>` | Account balance and details |
| `sync-status` | Current operating mode and validated-ledger progress |
| `rpc <method> [params]` | Call any JSON-RPC method directly |
| `ping` | Ping the local RPC server |
| `server-info` | Raw `server_info` output |
| `server-state` | Raw `server_state` output |
| `server-definitions` | Raw `server_definitions` output |
| `ledger-closed` | Canonical local closed-ledger sequence and hash |
| `ledger-current` | Current open ledger index |
| `ledger-header` | Validated ledger header |
| `fetch-info` | Coordinator phase/anchors, sessions, counters, and recovery decision |
| `get-counts` | Raw cache, ledger, and node-store counters |
| `can-delete [value]` | Get or set advisory online-delete ledger |
| `config` | Validate a configuration file without starting the node |
| `connect <address>` | Request an outbound peer connection |
| `log-rotate` | Compatibility command; currently a successful no-op |
| `random` | Generate random bytes through RPC |
| `validator-info` | Raw validator node information |
| `validator-list-sites` | Raw validator list site status |
| `unl-list` | Raw UNL list |
| `consensus-info` | Raw consensus state |
| `tx-reduce-relay` | Raw transaction relay reduction state |
| `validators` | Trusted validator list and agreement |
| `amendments` | Amendment status and voting |
| `db-stats` | NuDB disk usage and database statistics |
| `log-level <level>` | Set log level; the no-argument query is not yet populated |
| `benchmark` | Run internal performance benchmarks |
| `validator-keys` | Key management (see below) |
| `export-snapshot` | Export node store to snapshot file (admin RPC) |
| `load-snapshot` | Import snapshot into node store (offline) |
| `doctor` | Diagnose common configuration issues |
| `stop` | Graceful shutdown |
| `version` | Build version, commit hash, and build time |

### validator-keys Subcommands

| Command | Description |
|---------|-------------|
| `validator-keys generate` | Generate a new validator key pair |
| `validator-keys create-token` | Create an experimental token payload |
| `validator-keys sign` | Experimental local signing helper |
| `validator-keys revoke` | Create an experimental revocation payload |
| `validator-keys show` | Display the saved public key and creation time |

`generate` writes a non-overwriting, mode-`0600` `validator-keys.json` in the
current directory. The supported server configuration path uses its
`validation_seed` value in `[validation_seed]`. The token/sign/revoke helpers
are not yet documented as production-compatible with `rippled`; see
[VALIDATORS.md](VALIDATORS.md).

`fetch-info` exposes two related but distinct facts. NetworkOps forwards an
installed, chain-contiguous publication independently of freshness, but a
Tracking phase records it only when a fresh, non-divergent observation promotes
the phase to Full. Once Full, newer contiguous identities are recorded even on
a non-promoting pass. During initial catch-up, session expiry or replacement
does not imply its verified SHAMap nodes were discarded; pair this command with
`get-counts` and repeated ledger-head samples.

## RPC Port Auto-Discovery

The CLI automatically finds the RPC port by:
1. Reading `--conf <path>` if provided
2. Looking for `quaxar.cfg` in the current directory
3. Parsing the first `[port_*]` section with `protocol = http`
4. Falling back to `http://127.0.0.1:5005`

Override with `--rpc-url http://host:port`.

## Examples

```bash
# Check node health
quaxar health

# View current sync progress
quaxar sync-status

# Check fee before submitting
quaxar fee

# Look up an account
quaxar account rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh

# See connected peers
quaxar peers

# Raw RPC call with JSON params
quaxar rpc ledger '{"ledger_index":"validated"}'

# Compact JSON output for scripts
quaxar rpc server_info --raw

# Show raw server information
quaxar server-info

# Show live cache and node-store counters
quaxar get-counts

# Show ledger acquisition state
quaxar fetch-info

# Call the compatibility log-rotation endpoint (currently a no-op)
quaxar log-rotate

# Change log level to debug
quaxar log-level debug

# View latest ledger
quaxar ledger

# View specific ledger
quaxar ledger 95000000

# Database statistics (NuDB path, file sizes, counters)
quaxar db-stats

# Database statistics using a specific config file
quaxar db-stats --conf /etc/quaxar/quaxar.cfg

# Generate validator keys
quaxar validator-keys generate

# Diagnose issues
quaxar doctor

# Show version
quaxar version

# Graceful stop
quaxar stop
```

## Snapshot Commands

### export-snapshot

Export a snapshot through the admin RPC while the node remains online. The
node performs the export in a background job; the CLI shows a spinner until the
node reports that job as completed or failed.

```bash
quaxar export-snapshot --output /var/lib/quaxar/snapshots/testnet.xrpls
```

`--output` is required and names the snapshot file to create. The CLI polls the
admin `snapshot_status` RPC once per second and reports the final file size on
success. Against an older node that does not support that RPC, it truthfully
reports only that export was started and directs the operator to monitor the
node's snapshot logs. See [RPC.md](RPC.md) for the RPC details.

### load-snapshot

Import a snapshot file into the node store. The node must be stopped before
running this command.

```bash
quaxar load-snapshot --input /path/to/snapshot.xrpls --conf /etc/quaxar/quaxar.cfg
```

| Flag | Required | Description |
|------|----------|-------------|
| `--input` | Yes | Path to snapshot file |
| `--conf` | No | Config path determining the NodeStore; defaults to `/etc/quaxar/quaxar.cfg` |

The import uses bulk loading mode with pre-allocated NuDB hash tables. The CLI
shows a spinner throughout the synchronous import and reports success only
after the loader has verified every chunk and the final file hash. Runtime
depends on snapshot size, storage, CPU, and available memory.

## Exit Codes

`health` exits 0 when the node is reachable (including while syncing) and 1
when it is unreachable. RPC-backed commands generally return 1 for connection
or RPC errors, and command-line parse errors return 1. The current `config`,
`doctor`, `benchmark`, interactive CLI, and `validator-keys` dispatch paths
return 0 after invocation even when the command printed a diagnostic; do not
use their exit status as an automation contract yet. Readiness automation must
sample closed, validated, and coordinator phase progress across several ledger
closes.

## Health States

| State | Display | Meaning |
|-------|---------|---------|
| `full` / `proposing` / `validating` | ● Reachable (green) | Point-in-time state; verify sustained convergence |
| `tracking` / `syncing` / `connected` | ◐ Syncing (yellow) | Alive, not yet ready |
| Unreachable | ● Down (red) | Cannot connect |
