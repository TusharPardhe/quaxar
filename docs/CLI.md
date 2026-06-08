# CLI Reference

## Starting the Node

Start the server with an explicit config file:

```bash
quaxar --conf /etc/xrpld/xrpld.cfg
```

Running `quaxar` without a subcommand prints help. This avoids accidentally
starting a node with an unintended default configuration.

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
| `status` | Server state, uptime, ledger range |
| `health` | Health check with semantic states |
| `peers` | Connected peers with latency and version |
| `fee` | Current transaction fee |
| `ledger [seq]` | Ledger info (latest validated or by sequence) |
| `account <address>` | Account balance and details |
| `sync-status` | Current sync progress and state |
| `rpc <method> [params]` | Call any JSON-RPC method directly |
| `ping` | Ping the local RPC server |
| `server-info` | Raw `server_info` output |
| `server-state` | Raw `server_state` output |
| `server-definitions` | Raw `server_definitions` output |
| `ledger-closed` | Latest closed ledger sequence and hash |
| `ledger-current` | Current open ledger index |
| `ledger-header` | Validated ledger header |
| `fetch-info` | Ledger acquisition/fetch state |
| `get-counts` | Raw cache, ledger, and node-store counters |
| `can-delete [value]` | Get or set advisory online-delete ledger |
| `log-rotate` | Request runtime log rotation |
| `random` | Generate random bytes through RPC |
| `validator-info` | Raw validator node information |
| `validator-list-sites` | Raw validator list site status |
| `unl-list` | Raw UNL list |
| `consensus-info` | Raw consensus state |
| `tx-reduce-relay` | Raw transaction relay reduction state |
| `validators` | Trusted validator list and agreement |
| `amendments` | Amendment status and voting |
| `db-stats` | NuDB disk usage and database statistics |
| `log-level [level]` | Get or set log level |
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
| `validator-keys create-token` | Create a validator token from master key |
| `validator-keys sign` | Sign a message with the validator key |
| `validator-keys revoke` | Revoke a validator key (publish revocation) |
| `validator-keys show` | Display public key and manifest |

## RPC Port Auto-Discovery

The CLI automatically finds the RPC port by:
1. Reading `--conf <path>` if provided
2. Looking for `xrpld.cfg` in the current directory
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

# Rotate logs
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
quaxar db-stats --conf /etc/xrpld.cfg

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

Trigger a snapshot export via the admin RPC. The node remains online while the
export runs in a background thread.

```bash
quaxar export-snapshot
```

This calls the `export_snapshot` RPC method. The output file is written to the
configured node store path. See [RPC.md](RPC.md) for details on the RPC method.

### load-snapshot

Import a snapshot file into the node store. The node must be stopped before
running this command.

```bash
quaxar load-snapshot --input /path/to/snapshot.lz4 --conf /etc/xrpld/xrpld.cfg
```

| Flag | Required | Description |
|------|----------|-------------|
| `--input` | Yes | Path to snapshot file |
| `--conf` | Yes | Path to config file (determines NuDB path) |

The import uses bulk loading mode with pre-allocated NuDB hash tables. On NVMe,
26.5M nodes load in approximately 3 minutes.

## Exit Codes

| Command | Code | Meaning |
|---------|------|---------|
| `health` | 0 | Node is reachable (healthy or syncing) |
| `health` | 1 | Node is unreachable (down) |
| RPC-backed commands | 0 | RPC returned `status: success` |
| RPC-backed commands | 1 | RPC connection failed or returned an error |
| All others | 0 | Success |
| All others | 1 | Error |

## Health States

| State | Display | Meaning |
|-------|---------|---------|
| `full` / `proposing` / `validating` | ● Healthy (green) | Fully synced |
| `tracking` / `syncing` / `connected` | ◐ Syncing (yellow) | Alive, not yet ready |
| Unreachable | ● Down (red) | Cannot connect |
