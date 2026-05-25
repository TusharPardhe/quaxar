# CLI Reference

## Starting the Node

Run with no subcommand to start the server:

```bash
xrpld --conf /etc/xrpld/xrpld.cfg
```

## Interactive Mode

Launch an interactive shell with autocomplete and fuzzy search:

```bash
xrpld cli
```

Features:
- Tab completion for all commands and parameters
- Fuzzy search — type partial command names to match
- Arrow keys for history navigation
- Ctrl+R for reverse history search

## Subcommands

| Command | Description |
|---------|-------------|
| `status` | Server state, uptime, ledger range |
| `health` | Quick health check (exit 0=healthy, 1=unhealthy) |
| `peers` | Connected peers with latency and version |
| `fee` | Current transaction fee |
| `ledger` | Latest validated ledger info |
| `account <address>` | Account balance and details |
| `sync-status` | Current sync progress and state |
| `validators` | Trusted validator list and agreement |
| `amendments` | Amendment status and voting |
| `db-stats` | Database size, cache hit rate, node counts |
| `log-level [level]` | Get or set log level |
| `benchmark` | Run internal performance benchmarks |
| `validator-keys` | Key management (see below) |
| `doctor` | Diagnose common configuration issues |
| `config` | Show active configuration |
| `stop` | Graceful shutdown |
| `version` | Build version and commit hash |

### validator-keys Subcommands

| Command | Description |
|---------|-------------|
| `validator-keys generate` | Generate a new validator key pair |
| `validator-keys create-token` | Create a validator token from master key |
| `validator-keys sign` | Sign a message with the validator key |
| `validator-keys revoke` | Revoke a validator key (publish revocation) |
| `validator-keys show` | Display public key and manifest |

## Examples

```bash
# Check if node is healthy
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

# Check amendment voting
xrpld amendments

# Database statistics
xrpld db-stats

# Generate validator keys
xrpld validator-keys generate

# Create token for config
xrpld validator-keys create-token

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
| `health` | 0 | Node is healthy and synced |
| `health` | 1 | Node is unhealthy or not synced |
| All others | 0 | Success |
| All others | 1 | Error |
