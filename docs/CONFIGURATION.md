# Configuration Reference

This file explains the runtime configuration used by `xrpld`. Keep `xrpld.cfg`
focused on actual values; use this document for operational guidance.

## Loading A Config

Run with an explicit config path:

```bash
xrpld --conf /etc/xrpld/xrpld.cfg
```

Docker Compose mounts the repository `xrpld.cfg` to `/etc/xrpld/xrpld.cfg`
inside the container.

## Core Sections

### `[server]`

Lists enabled server port sections. Each line should be the name of a
`[port_*]` section.

```ini
[server]
port_rpc_admin_local
port_peer
```

### `[port_*]`

Configures one listening port.

| Key | Meaning |
|-----|---------|
| `port` | TCP port to bind. |
| `ip` | Bind address, for example `127.0.0.1` or `0.0.0.0`. |
| `protocol` | Comma-separated protocols, commonly `http`, `ws`, or `peer`. |
| `admin` | Admin access scope, typically `127.0.0.1` for local-only admin RPC. |
| `secure_gateway` | Trusted proxy/gateway IP for forwarded client metadata. |
| `send_queue_limit` | Optional websocket send queue limit. |

### `[node_size]`

Selects the resource profile. This is the recommended primary tuning knob.

```ini
[node_size]
medium
```

Profiles currently map to acquisition and cache defaults:

| Size | Active ledger fetch default | Intended use |
|------|-----------------------------|--------------|
| `tiny` | `2` | Small/dev machines. |
| `small` | `3` | Light nodes. |
| `medium` | `4` | Default balanced profile. |
| `large` | `5` | Stronger machines. |
| `huge` | `8` | High-throughput machines with ample memory and fast disk. |

The profile also influences SHAMap tree cache size, cache age, fetch-pack cache
size, write-dedup size, and run-data concurrency. Prefer changing `node_size`
before using expert overrides.

### `[ledger_acquisition]`

Optional expert tuning for cold-bootstrap ledger acquisition.

```ini
[ledger_acquisition]
ledger_fetch_limit = 8
```

| Key | Meaning |
|-----|---------|
| `ledger_fetch_limit` | Overrides the `[node_size]` cold-bootstrap active inbound-ledger acquisition limit. Valid range: `1..8`. |

If omitted, `node_size` decides the limit. Set `ledger_fetch_limit = 1` for the
most conservative cold-bootstrap behavior. Raising the value can improve
bootstrap throughput, but it increases memory, disk, CPU, and peer request
pressure. This override does not resize caches or change post-bootstrap
validated-ledger/history behavior; use a larger `node_size` when running high
values for long periods.

### `[node_db]`

Configures persistent ledger object storage.

| Key | Meaning |
|-----|---------|
| `type` | Storage backend, commonly `NuDB` or `RocksDB`. |
| `path` | Filesystem path for the node database. |
| `nudb_block_size` | Optional NuDB block size. |
| `online_delete` | Number of ledgers to retain before online deletion. |
| `advisory_delete` | Whether deletion requires explicit advisory control. |

### `[database_path]`

Directory for relational metadata databases.

### `[ledger_history]`

How much validated ledger history to keep available. Use a number or `full`.

### `[validators_file]`

Path to a validator-list config file. Relative paths are resolved from the
directory containing `xrpld.cfg`.

### `[validator_list_sites]`

Validator list publisher URLs.

### `[validator_list_keys]`

Validator list publisher public keys.

### `[ips]`

Fixed peer endpoints to try on startup.

```ini
[ips]
s1.ripple.com 51235
s2.ripple.com 51235
```

### `[network_id]`

Optional network selector. Common values are `main`, `testnet`, `devnet`, or a
numeric network ID.

### `[overlay]`

Peer overlay settings.

| Key | Meaning |
|-----|---------|
| `public_ip` | Advertised public endpoint IP. |
| `ip_limit` | Incoming peer limit per public IP. |
| `peer_private` | Request peers not broadcast your address. |
| `max_unknown_time` | Time to keep unknown peers before pruning. |
| `verify_endpoints` | Validate advertised endpoints before using them. |

### `[debug_logfile]`

Path for the debug log.

### `[rpc_startup]`

JSON RPC commands to run at startup, commonly to set log level.

```ini
[rpc_startup]
{ "command": "log_level", "severity": "warning" }
```

### `[ssl_verify]`

Set to `1` to validate HTTPS certificates or `0` to allow self-signed/internal
certificates.

## Recommended Examples

Balanced default:

```ini
[node_size]
medium
```

High-throughput machine:

```ini
[node_size]
large

[ledger_acquisition]
ledger_fetch_limit = 8
```

Conservative catchup:

```ini
[node_size]
medium

[ledger_acquisition]
ledger_fetch_limit = 1
```
