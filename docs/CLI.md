# CLI Reference

## Starting the Node

Run with no subcommand to start the server:

```bash
xrpld --conf /etc/xrpld/xrpld.cfg
```

## Interactive Mode

Launch an interactive shell with fuzzy search and inline suggestions:

```bash
xrpld cli
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
| `validators` | Trusted validator list and agreement |
| `amendments` | Amendment status and voting |
| `db-stats` | NuDB disk usage and database statistics |
| `log-level [level]` | Get or set log level |
| `benchmark` | Run internal performance benchmarks |
| `validator-keys` | Key management (see below) |
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
xrpld health

# View current sync progress
xrpld sync-status

# Check fee before submitting
xrpld fee

# Look up an account
xrpld account rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh

# See connected peers
xrpld peers

# Change log level to debug
xrpld log-level debug

# View latest ledger
xrpld ledger

# View specific ledger
xrpld ledger 95000000

# Database statistics (NuDB disk usage)
xrpld db-stats

# Generate validator keys
xrpld validator-keys generate

# Diagnose issues
xrpld doctor

# Show version
xrpld version

# Graceful stop
xrpld stop
```

## Exit Codes

| Command | Code | Meaning |
|---------|------|---------|
| `health` | 0 | Node is reachable (healthy or syncing) |
| `health` | 1 | Node is unreachable (down) |
| All others | 0 | Success |
| All others | 1 | Error |

## Health States

| State | Display | Meaning |
|-------|---------|---------|
| `full` / `proposing` / `validating` | ● Healthy (green) | Fully synced |
| `tracking` / `syncing` / `connected` | ◐ Syncing (yellow) | Alive, not yet ready |
| Unreachable | ● Down (red) | Cannot connect |
