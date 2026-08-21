<h1 align="center">
  <img src="assets/quaxar-icon.webp" alt="quaxar" width="120">
  <br>
  quaxar
</h1>

<p align="center">
  <strong>A Rust implementation of the XRP Ledger protocol</strong>
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

`quaxar` is a Rust implementation of the XRP Ledger server. It is designed to
sync ledger data, participate in the peer overlay, serve HTTP and WebSocket
JSON RPC requests, and provide an operator focused command line interface.

The project follows `rippled` behavior closely while using Rust ownership,
typed protocol models, structured errors, and explicit runtime boundaries. The
current implementation is suitable for development, parity testing, and
testnet node and validator operation.

## Current Status

`quaxar` is beta software. It has been validated on XRPL testnet as a synced
node and as a validator reaching `proposing`, publishing signed validations,
and advancing with the validated chain. Live submission coverage includes XRP
payments, issued-token payments, NFT minting, AMM creation, account queries,
and expected rejection cases.

Production validator operation is not recommended yet. Interfaces and runtime
configuration may still change as parity work continues.

## Capabilities

| Area | Current support |
| --- | --- |
| Protocol | XRP Ledger serialization, field definitions, amendments, transaction models, and SHAMap support. |
| Ledger sync | Parallel ledger acquisition, shared fetch cache, NuDB persistence, snapshot export/import, and configurable acquisition limits. |
| Storage | NuDB node store with bulk import mode, streaming export, and RocksDB configuration surfaces where implemented. |
| RPC | HTTP and WebSocket JSON RPC with public and admin command handling. |
| Transactions | Core payment, account, trust line, NFT, AMM, MPT, vault, lending, queue, and invariant paths under active parity coverage. |
| Operations | Interactive CLI, health checks, sync status, peer inspection, database statistics, log controls, and validator key tools. |
| Configuration | Interactive installer, explicit config validation, node size profiles, endpoint validation, and operator diagnostics. |

## Installation

### Interactive Installer

**macOS / Linux:**
```bash
curl -sSf https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.sh -o install.sh
bash install.sh
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.ps1 | iex
```

On macOS/Linux, `install.sh` checks host requirements, installs build
dependencies, builds `quaxar`, generates configuration, and can install a
systemd service on Linux hosts that provide systemd. It asks for runtime
settings interactively and applies defaults in unattended mode. The Windows script installs and verifies the
binary; create `quaxar.cfg` separately from the repository example.

For unattended installation with defaults:

```bash
curl -sSf https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.sh -o install.sh
bash install.sh -y
```

```powershell
irm https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.ps1 -OutFile install.ps1
.\install.ps1 -y
```

### Manual Build

Install Rust `1.90` or newer and the required native dependencies.

Linux:

```bash
sudo apt install build-essential pkg-config libssl-dev librocksdb-dev clang cmake git
```

macOS:

```bash
brew install openssl rocksdb cmake
```

Build and install from source:

```bash
git clone https://github.com/TusharPardhe/quaxar.git
cd quaxar
CC=clang CXX=clang++ cargo install --path xrpld/main
```

Run with an explicit configuration file:

```bash
quaxar --conf ./quaxar.cfg
```

### Docker

```bash
docker compose up -d
```

Docker Compose mounts `infra/docker/quaxar.cfg` into the container at
`/etc/quaxar/quaxar.cfg`. That container-only config listens on all container
interfaces while Compose publishes admin ports only on host loopback. Its
branded `quaxar-data` mount deliberately retains the common legacy
`quaxar_xrpld-data` volume identity so ordinary upgrades reuse that database.
Fresh installations can select a clean name with
`QUAXAR_DATA_VOLUME=quaxar-data`. Do not mount an old
`xrpld.cfg` verbatim: its data/log paths and relative validator-file references
belong to the legacy container layout. Start from the new Docker config, merge
the protected credentials/settings, and select that reviewed file:

```bash
cp infra/docker/quaxar.cfg quaxar.local.cfg
# Merge required legacy sections, changing /var/lib/xrpld to /var/lib/quaxar
# and /var/log/xrpld to /var/log/quaxar.
export QUAXAR_CONFIG_FILE=./quaxar.local.cfg
docker compose up -d
```

If the legacy config uses a relative `[validators_file]`, either merge those
public keys into `[validators]` or mount the file separately at an explicit
container path and update the setting. Validate the final config before the
upgrade and keep the original config as a protected rollback artifact.

Before an upgrade, use `docker volume ls` to confirm the exact old physical
volume name and set `QUAXAR_DATA_VOLUME` if it differs from
`quaxar_xrpld-data`. Admin scope is unrestricted inside the default Compose
network so Docker's host forwarding can reach it; never attach untrusted
containers or an externally shared network to this stack.

The container entrypoint performs a one-time ownership migration on an existing
root-owned data volume, then drops privileges to the `quaxar` user.

## Quick Start

Start a node:

```bash
quaxar --conf quaxar.cfg
```

Check sync and health:

```bash
quaxar sync-status
quaxar health
quaxar server-info
```

Call RPC directly:

```bash
quaxar rpc account_info '{"account":"rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh","ledger_index":"validated"}'
```

Open the interactive shell:

```bash
quaxar cli
```

Running `quaxar` without arguments prints help. Start the node with an explicit
`--conf` path so the intended configuration is unambiguous.

## Operator CLI

`quaxar` includes a first class operator CLI with interactive search, command
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
| `log-level <level>` | Set runtime log level; the no-argument compatibility query is not yet populated. |
| `log-rotate` | Compatibility command; currently acknowledges without rotating a file. |
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
| `export-snapshot` | Export node store to a snapshot file while the node stays online; waits visibly for the job outcome. |
| `load-snapshot` | Import and integrity-verify a snapshot file into the node store (offline). |
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

[node_db]
type = NuDB
path = /var/lib/quaxar/db/nudb

[ledger_history]
256

[validator_list_sites]
https://vl.ripple.com

[validator_list_keys]
ED2677ABFFD1B33AC6FBC3062B71F1E8397C1505E1C42C64D11AD1B28FF73F4734
```

`node_size` is the recommended primary tuning setting. It controls the default
resource profile for acquisition, cache sizing, and runtime concurrency.

The repository includes a small runnable `quaxar.cfg`. See
[docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the operator configuration
reference and [docs/RUNNING.md](docs/RUNNING.md) for operational guidance.

## Runtime Notes

| Topic | Guidance |
| --- | --- |
| Testnet operation | A medium node with NuDB has been validated on public testnet. |
| Full history | Set `[ledger_history]` to `full` and provision storage accordingly. Full history requires significantly more disk and time. |
| NuDB | Recommended for node store operation and used by the validated testnet deployment. |
| RocksDB | Available where the Rust storage path exposes the matching backend. Use only when the target deployment requires it. |
| Public endpoints | Use `verify_endpoints` for stricter advertised peer endpoint validation. |
| RPC parameters | Pass JSON params as a single quoted JSON object, for example `quaxar rpc account_info '{"account":"...","ledger_index":"validated"}'`. |

## Architecture

```text
xrpl/                            xrpld/
├── basics                        ├── acquisition  ├── nodestore
├── core                          ├── app          ├── overlay
├── protocol                      ├── cli          ├── perflog
├── resource                      ├── consensus    ├── rdb
└── shamap                        ├── core         ├── rpc/server
                                  ├── ledger       ├── tx
                                  └── main/metrics
```

The `xrpl` crates hold shared protocol and data structure foundations. Runtime
crates remain under the intentionally retained `xrpld/` source layout for
side-by-side upstream comparison, while Cargo packages, binaries, services,
configuration, environment variables, and metrics avoid `xrpld` product
branding. Selected operator packages use `quaxar-*`; generic internal crates
keep domain names such as `ledger`, `rpc`, and `server`.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design document.

## Documentation

| Document | Purpose |
| --- | --- |
| [RUNNING.md](docs/RUNNING.md) | Installation, service setup, operations, and troubleshooting. |
| [CONFIGURATION.md](docs/CONFIGURATION.md) | Operator configuration reference. |
| [CLI.md](docs/CLI.md) | Full command line reference. |
| [SYNCING.md](docs/SYNCING.md) | Sync behavior, acquisition flow, and operator checks. |
| [VALIDATORS.md](docs/VALIDATORS.md) | Validator identity, configuration, and operational guidance. |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate layout and runtime design. |
| [RPC.md](docs/RPC.md) | Supported RPC methods and examples. |

## Contributing

Contributions should follow the repository coding standards, test expectations,
and Conventional Commits. See [CONTRIBUTING.md](CONTRIBUTING.md)
before opening a pull request.

## License

[Apache License 2.0](LICENSE)
