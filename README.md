# xrpld (Beta)

```
██╗  ██╗██████╗ ██████╗ ██╗     ██████╗
╚██╗██╔╝██╔══██╗██╔══██╗██║     ██╔══██╗
 ╚███╔╝ ██████╔╝██████╔╝██║     ██║  ██║
 ██╔██╗ ██╔══██╗██╔═══╝ ██║     ██║  ██║
██╔╝ ██╗██║  ██║██║     ███████╗██████╔╝
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚══════╝╚═════╝
```

**A high performance XRPL node implementation in Rust (Beta)**

[![CI](https://github.com/TusharPardhe/rippled-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/TusharPardhe/rippled-rust/actions/workflows/ci.yml)
[![License: ISC](https://img.shields.io/badge/License-ISC-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org)

---

## Features

- **Mainnet Sync** — Full synchronization with XRPL mainnet from genesis or recent ledger
- **Parallel Acquisition** — Multi-threaded ledger and state acquisition for fast catch-up
- **RPC Parity** — Compatible JSON-RPC API matching rippled's interface
- **CLI Tools** — Beautiful interactive CLI with fuzzy search, validator key management, and diagnostics
- **Prometheus Metrics** — Built-in metrics endpoint for monitoring and alerting
- **Structured Logging** — `tracing`-based structured logs with configurable levels

## Quick Start

```bash
# Build the node
cargo build --release

# Run the node with config
./target/release/xrpld --conf mainnet_xrpld.cfg

# Use the interactive CLI
./target/release/xrpld cli
```

## CLI Usage

Launch the interactive CLI with `./target/release/xrpld cli`:

```
$ ./target/release/xrpld cli

  ██╗  ██╗██████╗ ██████╗ ██╗     ██████╗
  ╚██╗██╔╝██╔══██╗██╔══██╗██║     ██╔══██╗
   ╚███╔╝ ██████╔╝██████╔╝██║     ██║  ██║
   ██╔██╗ ██╔══██╗██╔═══╝ ██║     ██║  ██║
  ██╔╝ ██╗██║  ██║██║     ███████╗██████╔╝
  ╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚══════╝╚═════╝

  _
```

### Subcommands

| Command | Description |
|---------|-------------|
| `status` | Node status overview |
| `health` | Health check |
| `peers` | Connected peers |
| `fee` | Current fee info |
| `ledger [seq]` | Ledger details |
| `account <addr>` | Account info |
| `sync-status` | Sync progress |
| `validators` | Trusted validators |
| `amendments` | Amendment status |
| `db-stats` | Database statistics |
| `log-level [level]` | Get/set log level |
| `benchmark` | Run benchmarks |
| `validator-keys` | Key management (generate, create-token, sign, revoke, show) |
| `doctor` | Pre-flight diagnostics |
| `config` | Validate config file |
| `stop` | Graceful shutdown |
| `version` | Build version info |

## Architecture

The project is organized as a Cargo workspace with modular crates:

```
xrpl/                       xrpld/
├── protocol   (wire types, serialization)    ├── app        (application logic, acquisition)
├── basics     (utilities, tagged types)      ├── consensus  (XRPL consensus protocol)
├── shamap     (SHAMap trie implementation)   ├── ledger     (ledger state management)
├── core       (crypto, keys, signing)        ├── overlay    (P2P networking, peer protocol)
└── resource   (load management)             ├── nodestore  (NuDB storage backend)
                                              ├── rpc        (JSON-RPC handlers)
                                              ├── server     (HTTP/WS server)
                                              ├── tx         (transaction processing)
                                              ├── metrics    (Prometheus metrics)
                                              └── main       (binary entry point, CLI)
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design document.

## Configuration

Example `xrpld.cfg`:

```ini
[server]
port_rpc_admin_local
port_peer
port_ws_admin_local

[port_rpc_admin_local]
port = 5055
ip = 127.0.0.1
admin = 127.0.0.1
protocol = http

[port_peer]
port = 51235
ip = 0.0.0.0
protocol = peer

[node_db]
type=NuDB
path=/var/lib/xrpld/db/nudb
online_delete=512

[node_size]
medium

[ledger_history]
256
```

## Comparison with rippled (C++)

| Feature | xrpld (Rust) | rippled (C++) |
|---------|-------------|---------------|
| Ledger acquisition | Parallel (multi-threaded) | Sequential (single-threaded) |
| Structured logging | Built-in (tracing crate) | Custom text logging |
| Prometheus metrics | Built-in | Requires external tooling |
| Interactive CLI | Built-in with autocomplete | None |
| Validator key management | Built-in subcommand | Separate binary |
| Health check endpoint | Built-in (exit code) | Not available |














## Contributing

We welcome contributions! Please read [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on:

- Setting up your development environment
- Code style and linting
- PR process and commit conventions

## License

This project is licensed under the [ISC License](LICENSE) — the same license used by [rippled](https://github.com/XRPLF/rippled).

## Docker

Run xrpld in a container:

```bash
# Build the image
docker build -t xrpld .

# Run with default config
docker run -d --name xrpld \
  -p 5055:5055 -p 6066:6066 -p 51235:51235 \
  -v xrpld-data:/var/lib/xrpld \
  xrpld

# Or use Docker Compose
docker compose up -d

# Check health
docker exec xrpld xrpld health

# View logs
docker logs -f xrpld
```

The Docker image uses a multi-stage build producing a minimal (~50MB) runtime image based on `debian:bookworm-slim`. Data is persisted in the `xrpld-data` volume.

## Binary Releases

Pre-built binaries are available on the [Releases](https://github.com/TusharPardhe/xrpld/releases) page for:

- Linux x86_64
- Linux aarch64 (ARM)
- macOS x86_64 (Intel)
- macOS aarch64 (Apple Silicon)

```bash
# Download and run (example for Linux x86_64)
curl -LO https://github.com/TusharPardhe/xrpld/releases/latest/download/xrpld-x86_64-unknown-linux-gnu.tar.gz
tar xzf xrpld-x86_64-unknown-linux-gnu.tar.gz
./xrpld --conf xrpld.cfg
```

To create a new release, tag and push:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The CI will automatically build binaries for all platforms and attach them to the GitHub release.
