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

`quaxar` is beta software. Testnet runs have reached `full` and `proposing`,
published signed validations, and advanced with the validated chain. Treat
these as observed capabilities rather than a production-readiness guarantee;
sustained convergence remains part of active parity validation. Live
submission coverage includes XRP
payments, issued-token payments, NFT minting, AMM creation, account queries,
and expected rejection cases.

Production validator operation is not recommended yet. Interfaces and runtime
configuration may still change as parity work continues.

## Capabilities

| Area | Current support |
| --- | --- |
| Protocol | XRP Ledger serialization, field definitions, amendments, transaction models, and SHAMap support. |
| Ledger sync | Parallel coalesced per-hash acquisition, one shared NodeFamily cache family, reusable verified SHAMap nodes, NuDB persistence, snapshot export/import, and bounded execution resources. |
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

For unattended macOS/Linux installation with defaults:

```bash
curl -sSf https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.sh -o install.sh
bash install.sh -y
```

The Windows installer is already non-interactive. To save and inspect it before
execution, use `irm https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.ps1 -OutFile install.ps1`, then run `.\install.ps1`.

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
CC=clang CXX=clang++ cargo install --path xrpld/main --locked
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
logical mount is `quaxar-data`, while the default physical volume deliberately
retains the common legacy `quaxar_xrpld-data` identity so an ordinary upgrade
does not attach an empty database. A confirmed fresh installation can opt into
the clean physical name:

```bash
export QUAXAR_DATA_VOLUME=quaxar-data
docker compose up -d
```

Do not mount an old
`xrpld.cfg` verbatim: its data/log paths and relative validator-file references
belong to the legacy container layout. Start from the new Docker config, merge
the protected credentials/settings, and select that reviewed file:

```bash
cp infra/docker/quaxar.cfg quaxar.local.cfg
# Merge required legacy sections and change /var/lib/xrpld to /var/lib/quaxar.
# Omit legacy debug_logfile paths; the container writes logs to stdout/stderr.
export QUAXAR_CONFIG_FILE=./quaxar.local.cfg
docker compose up -d
```

If the legacy config uses a relative `[validators_file]`, either merge those
public keys into `[validators]` or mount the file separately at an explicit
container path and update the setting. Validate the final config before the
upgrade and keep the original config as a protected rollback artifact.

Before an upgrade, use `docker volume ls` to confirm the exact old physical
volume name and set `QUAXAR_DATA_VOLUME` if it differs from the compatibility
default. Never select `quaxar-data` merely to rename an existing volume; copy
and verify data through a planned migration instead. Admin scope is
unrestricted inside the default Compose network so Docker's host forwarding
can reach it; never attach untrusted containers or an externally shared
network to this stack.

The container entrypoint performs a one-time ownership migration on an existing
root-owned data volume, then drops privileges to the `quaxar` user.

## Quick Start

Start a node:

```bash
quaxar --conf quaxar.cfg
```

Check sync and health:

```bash
quaxar health
quaxar sync-status
quaxar ledger-closed
quaxar ledger-current
quaxar fetch-info
quaxar server-info
```

Sample these across several ledger closes. `health` proves RPC reachability,
including while syncing; one green `full` or `proposing` response does not prove
sustained readiness.

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
| `health` | Check RPC reachability and show a point-in-time state label. |
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
| `ledger-closed` | Show the canonical local closed ledger. |
| `ledger-current` | Show the current open ledger index. |
| `ledger-header` | Show the validated ledger header. |
| `fetch-info` | Show coordinator phase/anchors, per-hash sessions, and recovery counters. |
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
| `cli` | Open the interactive operator shell. |
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
`ledger_history = full` requires `[node_db] online_delete = 0`; with online
deletion enabled, numeric history cannot exceed the deletion interval.

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
| Prometheus | The metrics package records selected acquisition/mode instruments, but normal bootstrap does not start its HTTP exporter; packaged deployments have no `/metrics` endpoint. |

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

NetworkOps owns the independently moving preferred-LCL policy. The typed
coordinator preserves a stable recovery anchor while per-hash acquisitions
continue, and tracks canonical local and contiguous publication identities
without coupling ordinary cache acquisition to public operating mode.
NetworkOps forwards installed, chain-contiguous publication observations
independently of freshness. A fresh non-divergent observation promotes
`tracking` to `full`; once full, newer contiguous identities remain observable
even on a non-promoting pass.
Payment flow shares one AMM context across reverse/forward strand evaluation so
only the selected strand advances multipath AMM iteration state; AMM liquidity
is evaluated before CLOB execution at each book quality. Issued-currency
execution uses spendable `accountHolds` orientation, and direct self-cross
cancellation follows the reference funding/quality/authorization callback
order so speculative strand handling cannot change the canonical ledger hash.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design document.

## Documentation

| Document | Purpose |
| --- | --- |
| [RUNNING.md](docs/RUNNING.md) | Installation, service setup, operations, and troubleshooting. |
| [CONFIGURATION.md](docs/CONFIGURATION.md) | Operator configuration reference. |
| [CLI.md](docs/CLI.md) | Full command line reference. |
| [SYNCING.md](docs/SYNCING.md) | Sync behavior, acquisition flow, and operator checks. |
| [ACQUISITION.md](docs/ACQUISITION.md) | Coordinator ownership, per-hash sessions, data flow, backpressure, and durability architecture. |
| [VALIDATORS.md](docs/VALIDATORS.md) | Validator identity, configuration, and operational guidance. |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate layout and runtime design. |
| [RPC.md](docs/RPC.md) | Supported RPC methods and examples. |

## Contributing

Contributions should follow the repository coding standards, test expectations,
and Conventional Commits. See [CONTRIBUTING.md](CONTRIBUTING.md)
before opening a pull request.

## License

[Apache License 2.0](LICENSE)
