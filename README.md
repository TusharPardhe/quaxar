<h1 align="center">

```
██╗  ██╗██████╗ ██████╗ ██╗     ██████╗
╚██╗██╔╝██╔══██╗██╔══██╗██║     ██╔══██╗
 ╚███╔╝ ██████╔╝██████╔╝██║     ██║  ██║
 ██╔██╗ ██╔══██╗██╔═══╝ ██║     ██║  ██║
██╔╝ ██╗██║  ██║██║     ███████╗██████╔╝
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚══════╝╚═════╝
```

</h1>

<p align="center">
  <strong>A full Rust implementation of the XRP Ledger protocol</strong>
</p>

<p align="center">
  <a href="https://github.com/TusharPardhe/xrpld/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/TusharPardhe/xrpld/ci.yml?style=flat&logo=github&label=CI" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-ISC-blue.svg" alt="License: ISC"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.90%2B-orange.svg" alt="Rust"></a>
  <a href="https://github.com/TusharPardhe/xrpld/releases"><img src="https://img.shields.io/badge/status-beta-yellow.svg" alt="Beta"></a>
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="docs/RUNNING.md">Running</a> •
  <a href="docs/CLI.md">CLI Reference</a> •
  <a href="docs/ARCHITECTURE.md">Architecture</a> •
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

---

## What is xrpld?

xrpld is a ground-up Rust implementation of the [XRP Ledger](https://xrpl.org) node (rippled). It connects to the XRPL mainnet, syncs the full ledger state, and provides a compatible JSON-RPC interface.

### Goals

1. **Performance** — Parallel ledger acquisition, zero-copy serialization, and efficient NuDB storage
2. **Reliability** — Memory-safe by default, no undefined behavior, structured error handling
3. **Operability** — Built-in CLI with interactive mode, health checks, and diagnostics
4. **Compatibility** — Drop-in RPC replacement for rippled with the same API surface

### Status

> ⚠️ **Beta** — xrpld syncs with mainnet and serves RPC queries. Not yet recommended for validators or production transaction submission. APIs may change.

---

## Installation

### One-Line Setup (recommended)

```bash
curl -sSf https://raw.githubusercontent.com/TusharPardhe/xrpld/main/install.sh | bash
```

Or with all defaults (non-interactive):

```bash
curl -sSf https://raw.githubusercontent.com/TusharPardhe/xrpld/main/install.sh | bash -s -- -y
```

The setup script will:
- Check your system meets requirements (CPU, RAM, disk)
- Install dependencies (Rust, OpenSSL, RocksDB, etc.)
- Build and install `xrpld` to your PATH
- Generate an `xrpld.cfg` and validator list interactively
- Optionally set up a systemd service

### From Source (manual)

```bash
# Prerequisites: Rust 1.90+, system deps
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Linux
sudo apt install build-essential pkg-config libssl-dev librocksdb-dev clang cmake

# macOS
brew install openssl rocksdb cmake

# Install xrpld
git clone https://github.com/TusharPardhe/xrpld.git
cd xrpld
cargo install --path xrpld/main
```

After installation, `xrpld` is available in your PATH.

Use the repository default config directly while developing:

```bash
xrpld --conf ./xrpld.cfg
```

The checked-in `xrpld.cfg` is intentionally small and runnable. Detailed
parameter explanations live in [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

### Docker

```bash
docker compose up -d
```

Docker Compose mounts `./xrpld.cfg` into the container as
`/etc/xrpld/xrpld.cfg`, matching the Docker image default command.

### Binary Releases

Pre-built binaries for Linux and macOS are available on the [Releases](https://github.com/TusharPardhe/xrpld/releases) page.

---

## Quick Start

```bash
# Start the node
xrpld --conf xrpld.cfg

# Check status
xrpld status

# Interactive CLI
xrpld cli

# Health check
xrpld health
```

---

## CLI

xrpld includes a built-in CLI with an interactive mode featuring inline fuzzy search:

```
  ❯ st
  ──────────────────────────────────────────────────
  ▸ status          Node status overview
    stop            Graceful shutdown
    sync-status     Sync progress
  ──────────────────────────────────────────────────
```

| Command | Description |
|---------|-------------|
| `status` | Node state, peers, uptime, ledger range |
| `health` | Semantic health check (Healthy / Syncing / Down) |
| `peers` | Connected peers with latency and protocol version |
| `fee` | Current transaction fee |
| `ledger [seq]` | Ledger details |
| `account <addr>` | Account balance and info |
| `sync-status` | Sync progress |
| `db-stats` | NuDB disk usage |
| `validators` | Trusted validator list |
| `amendments` | Amendment voting status |
| `doctor` | Pre-flight diagnostics |
| `validator-keys` | Generate, rotate, and revoke validator keys |
| `version` | Build info |

See [docs/CLI.md](docs/CLI.md) for the full reference.

---

## Architecture

```
xrpl/                            xrpld/
├── protocol  (wire format)      ├── app        (application, bootstrap)
├── basics    (utilities)        ├── consensus  (XRPL consensus)
├── shamap    (Merkle trie)      ├── ledger     (state, acquisition)
├── core      (crypto, jobs)     ├── overlay    (P2P network)
└── resource  (load mgmt)       ├── nodestore  (NuDB backend)
                                  ├── rpc        (JSON-RPC handlers)
                                  ├── server     (HTTP/WS transport)
                                  ├── tx         (transaction engine)
                                  └── main       (binary, CLI)
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design document.

---

## Configuration

Minimal `xrpld.cfg`:

```ini
[server]
port_rpc_admin_local
port_peer

[port_rpc_admin_local]
port = 5005
ip = 127.0.0.1
protocol = http

[port_peer]
port = 51235
ip = 0.0.0.0
protocol = peer

[node_size]
medium

[ledger_acquisition]
# Optional cold-bootstrap acquisition override. Omit to use [node_size].
# ledger_fetch_limit = 8

[node_db]
type = NuDB
path = /var/lib/xrpld/db/nudb

[validators_file]
validators.txt
```

The repository includes this as a clean runnable default in `xrpld.cfg`. See
[docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the full configuration
reference and [docs/RUNNING.md](docs/RUNNING.md) for operational guidance.

---

## Comparison with rippled (C++)

| | xrpld (Rust) | rippled (C++) |
|---|---|---|
| Language | Rust (memory-safe) | C++ |
| Ledger acquisition | Parallel, multi-peer | Sequential |
| Interactive CLI | Built-in with fuzzy search | None |
| Validator key mgmt | Built-in subcommand | Separate binary |
| Health endpoint | Semantic (Healthy/Syncing/Down) | Not available |
| Structured logging | tracing crate | Custom text |
| Metrics | Prometheus built-in | External tooling |

---

## Documentation

| Document | Description |
|----------|-------------|
| [RUNNING.md](docs/RUNNING.md) | Installation, configuration, systemd setup |
| [CONFIGURATION.md](docs/CONFIGURATION.md) | Full configuration reference |
| [CLI.md](docs/CLI.md) | Full CLI reference |
| [SYNCING.md](docs/SYNCING.md) | How sync works, time estimates, troubleshooting |
| [VALIDATORS.md](docs/VALIDATORS.md) | Validator setup and key management |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crate structure and design decisions |
| [OPTIMIZATIONS.md](docs/OPTIMIZATIONS.md) | Performance characteristics |
| [RPC.md](docs/RPC.md) | Supported RPC methods |

---

## Contributing

We welcome contributions. Please read [CONTRIBUTING.md](CONTRIBUTING.md) for:

- Development setup
- Code style and linting (`just ci`)
- PR process and commit conventions

---

## License

[ISC License](LICENSE).
