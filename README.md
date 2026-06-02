<h1 align="center">
  <img src="assets/xrpld-icon.png" alt="xrpld" width="120">
  <br>
  xrpld
</h1>

<p align="center">
  <strong>A Rust implementation of the XRP Ledger server</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache 2.0"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.90%2B-orange.svg" alt="Rust"></a>
  <img src="https://img.shields.io/badge/status-beta-yellow.svg" alt="Beta">
</p>

<p align="center">
  <a href="#overview">Overview</a> ·
  <a href="#installation">Installation</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="#operator-cli">Operator CLI</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#documentation">Documentation</a>
</p>

## Overview

`xrpld` is a Rust implementation of the XRP Ledger server. It is designed to
sync ledger data, participate in the peer overlay, serve HTTP and WebSocket
JSON RPC requests, and provide an operator focused command line interface.

The project follows `rippled` behavior closely while using Rust ownership,
typed protocol models, structured errors, and explicit runtime boundaries. The
current implementation is suitable for development, parity testing, testnet
operation, and non validator node evaluation.

## Current Status

`xrpld` is beta software. It has been validated on XRPL testnet as a synced
non validator node with live submission tests for XRP payments, issued token
payments, NFT minting, AMM creation, account queries, and expected rejection
cases.

Production validator operation is not recommended yet. Interfaces and runtime
configuration may still change as parity work continues.

## Capabilities

| Area | Current support |
| --- | --- |
| Protocol | XRP Ledger serialization, field definitions, amendments, transaction models, and SHAMap support. |
| Ledger sync | Parallel ledger acquisition, shared fetch cache, NuDB persistence, and configurable acquisition limits. |
| Storage | NuDB node store support with RocksDB configuration surfaces where implemented. |
| RPC | HTTP and WebSocket JSON RPC with public and admin command handling. |
| Transactions | Core payment, account, trust line, NFT, AMM, MPT, vault, lending, queue, and invariant paths under active parity coverage. |
| Operations | Interactive CLI, health checks, sync status, peer inspection, database statistics, log controls, and validator key tools. |
| Configuration | Interactive installer, explicit config validation, node size profiles, endpoint validation, and operator diagnostics. |

## Installation

### Interactive Installer

```bash
curl -sSf https://raw.githubusercontent.com/TusharPardhe/xrpld/main/install.sh | bash
```

The installer checks host requirements, installs build dependencies, builds
`xrpld`, generates configuration files, validates user input, and can install a
systemd service. It asks for available runtime settings interactively and
applies defaults when non interactive mode is requested.

For unattended installation with defaults:

```bash
curl -sSf https://raw.githubusercontent.com/TusharPardhe/xrpld/main/install.sh | bash -s -- -y
```

### Manual Build

Install Rust `1.90` or newer and the required native dependencies.

Linux:

```bash
sudo apt install build-essential pkg-config libssl-dev librocksdb-dev clang cmake
```

macOS:

```bash
brew install openssl rocksdb cmake
```

Build and install from source:

```bash
git clone https://github.com/TusharPardhe/xrpld.git
cd xrpld
cargo install --path xrpld/main
```

Run with an explicit configuration file:

```bash
xrpld --conf ./xrpld.cfg
```

### Docker

```bash
docker compose up -d
```

Docker Compose mounts the repository `xrpld.cfg` into the container at
`/etc/xrpld/xrpld.cfg`.

## Quick Start

Start a node:

```bash
xrpld --conf xrpld.cfg
```

Check sync and health:

```bash
xrpld sync-status
xrpld health
xrpld server-info
```

Call RPC directly:

```bash
xrpld rpc account_info '{"account":"rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh","ledger_index":"validated"}'
```

Open the interactive shell:

```bash
xrpld cli
```

Running `xrpld` without a subcommand prints help instead of starting a node
implicitly. This avoids accidental launches with an unintended configuration.

## Operator CLI

`xrpld` includes a first class operator CLI with interactive search, command
suggestions, clear errors for unknown commands, and direct RPC passthrough.

| Command | Purpose |
| --- | --- |
| `status` | Show server state, uptime, peers, and ledger range. |
| `health` | Return a semantic health result for scripts and operators. |
| `sync-status` | Show whether the node is connected, syncing, tracking, or full. |
| `peers` | Show connected peers with latency and protocol details. |
| `fee` | Show the current transaction fee from RPC. |
| `ledger [seq]` | Show validated ledger details or a specific ledger. |
| `account <address>` | Show account balance and account root data. |
| `rpc <method> [params]` | Call any JSON RPC method with JSON parameters. |
| `ping` | Ping the configured RPC server. |
| `server-info` | Show raw `server_info` output. |
| `server-state` | Show raw `server_state` output. |
| `server-definitions` | Show protocol definitions from the node. |
| `ledger-closed` | Show the latest closed ledger. |
| `ledger-current` | Show the current open ledger index. |
| `ledger-header` | Show the validated ledger header. |
| `fetch-info` | Show ledger acquisition state. |
| `get-counts` | Show cache, ledger, and node store counters. |
| `db-stats` | Show NuDB file sizes and database counters. |
| `can-delete [value]` | Get or set the advisory online deletion ledger. |
| `config` | Validate the configuration file without starting the node. |
| `connect <address>` | Request a connection to a peer address. |
| `log-level [level]` | Get or set runtime log level. |
| `log-rotate` | Request runtime log rotation. |
| `random` | Generate random bytes through RPC. |
| `validator-info` | Show raw validator node information. |
| `validator-list-sites` | Show validator list site status. |
| `unl-list` | Show raw UNL list information. |
| `consensus-info` | Show raw consensus state. |
| `tx-reduce-relay` | Show transaction relay reduction state. |
| `validators` | Show trusted validator list status. |
| `amendments` | Show amendment voting status. |
| `validator-keys` | Generate, inspect, sign, and revoke validator keys. |
| `benchmark` | Run internal performance benchmarks. |
| `doctor` | Diagnose common configuration and runtime issues. |
| `stop` | Request graceful shutdown. |
| `version` | Show build version, commit hash, and build time. |

See [docs/CLI.md](docs/CLI.md) for the full reference.

## Configuration

Minimal runnable configuration:

```ini
[server]
port_rpc_admin_local
port_peer

[port_rpc_admin_local]
port = 5005
ip = 127.0.0.1
protocol = http,ws
admin = 127.0.0.1

[port_peer]
port = 51235
ip = 0.0.0.0
protocol = peer

[node_size]
medium

[ledger_acquisition]
# Optional expert override. Valid range: 1 through 8.
# If omitted, node_size selects the default.
# ledger_fetch_limit = 8

[node_db]
type = NuDB
path = /var/lib/xrpld/db/nudb

[ledger_history]
256

[validators_file]
validators.txt
```

`node_size` is the recommended primary tuning setting. It controls the default
resource profile for acquisition, cache sizing, and runtime concurrency.
`ledger_fetch_limit` is an expert override for active inbound ledger
acquisitions during cold bootstrap. Use `1` for conservative operation and up
to `8` on strong machines with enough memory, disk throughput, and peer
capacity.

The repository includes a small runnable `xrpld.cfg`. See
[docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the complete configuration
reference and [docs/RUNNING.md](docs/RUNNING.md) for operational guidance.

## Runtime Notes

| Topic | Guidance |
| --- | --- |
| Testnet operation | A medium node with NuDB and `ledger_fetch_limit = 8` has been validated on public testnet. |
| Full history | Use `ledger_history = full` and provision storage accordingly. Full history requires significantly more disk and time. |
| NuDB | Recommended for node store operation and used by the validated testnet deployment. |
| RocksDB | Available where the Rust storage path exposes the matching backend. Use only when the target deployment requires it. |
| Public endpoints | Use `verify_endpoints` for stricter advertised peer endpoint validation. |
| RPC parameters | Pass JSON params as a single quoted JSON object, for example `xrpld rpc account_info '{"account":"...","ledger_index":"validated"}'`. |

## Architecture

```text
xrpl/                            xrpld/
├── protocol                      ├── app
├── basics                        ├── consensus
├── shamap                        ├── ledger
├── core                          ├── overlay
└── resource                      ├── nodestore
                                  ├── rpc
                                  ├── server
                                  ├── tx
                                  └── main
```

The `xrpl` crates hold shared protocol and data structure foundations. The
`xrpld` crates hold the node runtime, peer overlay, ledger acquisition, storage
integration, RPC server, transaction engine, and operator command line.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design document.

## Documentation

| Document | Purpose |
| --- | --- |
| [RUNNING.md](docs/RUNNING.md) | Installation, service setup, operations, and troubleshooting. |
| [CONFIGURATION.md](docs/CONFIGURATION.md) | Complete runtime configuration reference. |
| [CLI.md](docs/CLI.md) | Full command line reference. |
| [SYNCING.md](docs/SYNCING.md) | Sync behavior, acquisition flow, and operator checks. |
| [VALIDATORS.md](docs/VALIDATORS.md) | Validator key and token guidance. |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate layout and runtime design. |
| [OPTIMIZATIONS.md](docs/OPTIMIZATIONS.md) | Performance characteristics and tuning notes. |
| [RPC.md](docs/RPC.md) | Supported RPC methods and examples. |

## Contributing

Contributions should follow the repository coding standards, test expectations,
and Commitizen style commit messages. See [CONTRIBUTING.md](CONTRIBUTING.md)
before opening a pull request.

## License

[ISC License](LICENSE)
