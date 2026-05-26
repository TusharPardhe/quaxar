# Running xrpld

Guide for node operators running the Rust implementation of the XRP Ledger server.

## Hardware Requirements

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| CPU | 4 cores | 8+ cores |
| RAM | 16 GB | 32 GB |
| Disk | 500 GB NVMe SSD | 1 TB NVMe SSD |
| Network | 100 Mbps | 1 Gbps |

Disk usage grows over time with ledger history. NVMe is strongly recommended for NuDB performance.

## Supported Platforms

- Linux x86_64 (Ubuntu 22.04+, Debian 12+, RHEL 9+)
- macOS arm64 (Apple Silicon)
- macOS x86_64

## Building from Source

### Automated Setup (recommended)

```bash
# Download and run the interactive installer
curl -sSf https://raw.githubusercontent.com/TusharPardhe/xrpld/main/install.sh -o install.sh
chmod +x install.sh
./install.sh

# Or non-interactive (all defaults, local build)
./install.sh -y
```

The installer will:
- Assess your hardware and warn if below requirements
- Let you choose Docker or local build
- Install all dependencies
- Build and install `xrpld` to your PATH
- Generate config files (all fields configurable)
- Optionally set up a systemd service

### Manual Setup

### Prerequisites

**Rust toolchain (1.90+):**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Linux (Ubuntu/Debian):**
```bash
sudo apt install build-essential pkg-config libssl-dev librocksdb-dev clang cmake
```

**macOS:**
```bash
brew install openssl rocksdb cmake
```

### Build & Install

```bash
git clone https://github.com/TusharPardhe/xrpld.git
cd xrpld
cargo install --path xrpld/main
```

This builds the release binary and installs it to `~/.cargo/bin/xrpld` (already in PATH).

### Build Troubleshooting

**RocksDB compilation segfault / OOM (common on ≤16GB RAM):**

The RocksDB C++ library compiles from source by default and can exhaust memory. Fix by installing the system package:

```bash
# Linux
sudo apt install librocksdb-dev
ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu cargo install --path xrpld/main
```

**Rustc segfault during build (too many parallel jobs):**

```bash
CARGO_BUILD_JOBS=2 cargo install --path xrpld/main
```

**OpenSSL build failure:**

```bash
sudo apt install libssl-dev pkg-config
```

**`.cargo/config.toml` linker error:**

The repo includes an optional `lld` linker config for faster builds. If `lld` is not installed:

```bash
rm .cargo/config.toml
# Or install lld:
sudo apt install lld clang
```

### Build Notes

- **Linux without librocksdb-dev:** Set `ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu` or RocksDB will compile from source (slow, may OOM on 16GB machines)
- **Low-memory machines:** Use `CARGO_BUILD_JOBS=2` to limit parallelism
- **`.cargo/config.toml`:** The repo includes an optional lld linker config for faster builds. If `lld` is not installed, remove this file: `rm .cargo/config.toml`

## Configuration

Create a configuration file (e.g., `xrpld.cfg`):

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

[node_db]
type = NuDB
path = /var/lib/xrpld/db/nudb
online_delete = 2000
advisory_delete = 0

[database_path]
/var/lib/xrpld/db

[validators_file]
validators.txt

[ips]
s1.ripple.com 51235
s2.ripple.com 51235
```

### Validator List (validators.txt)

The `[validators_file]` directive loads a separate file containing trusted validator list sources. Create `validators.txt` alongside your config:

```ini
[validator_list_sites]
https://vl.ripple.com

[validator_list_keys]
ED2677ABFFD1B33AC6FBC3062B71F1E8397C1505E1C42C64D11AD1B28FF73F4734
```

Alternatively, place these sections directly in `xrpld.cfg`.

### Configuration Sections

| Section | Purpose |
|---------|---------|
| `[server]` | Lists port definitions to activate |
| `[port_*]` | Port binding: port number, IP, protocol (http/ws/peer) |
| `[node_db]` | Database backend (NuDB), path, deletion policy |
| `[node_size]` | Memory tuning: tiny, small, medium, large, huge |
| `[validators_file]` | Path to file with validator list sites and keys |
| `[validator_list_sites]` | URLs to fetch trusted validator lists |
| `[validator_list_keys]` | Public keys of validator list publishers |
| `[ips]` | Peer endpoints to connect to on startup |

## Starting the Node

```bash
RUST_LOG=info ./target/release/xrpld --conf xrpld.cfg
```

Background:
```bash
RUST_LOG=info nohup ./target/release/xrpld --conf xrpld.cfg > xrpld.log 2>&1 &
```

## Systemd Service

Create `/etc/systemd/system/xrpld.service`:

```ini
[Unit]
Description=XRP Ledger Node (Rust)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=xrpld
Group=xrpld
ExecStart=/usr/local/bin/xrpld --conf /etc/xrpld/xrpld.cfg
Restart=on-failure
RestartSec=10
LimitNOFILE=65536
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now xrpld
```

## Monitoring

### Health Check

```bash
xrpld health
# Exit code 0 = reachable (healthy or syncing)
# Exit code 1 = unreachable (down)
```

Or via RPC:

```bash
curl -s http://127.0.0.1:5005 -d '{"method":"server_info"}' | jq .result.info.server_state
```

Expected healthy states: `full`, `proposing`, `validating`.

### Database Usage

```bash
xrpld db-stats
```

Shows NuDB data file size, key file size, and total disk usage.

## Log Management

Control log verbosity with the `RUST_LOG` environment variable:

```bash
# Levels: error, warn, info, debug, trace
RUST_LOG=info ./xrpld --conf xrpld.cfg

# Per-module control
RUST_LOG=info,consensus=debug,overlay=warn ./xrpld --conf xrpld.cfg
```

Change at runtime:

```bash
xrpld log-level debug
```

## Common Issues

| Problem | Cause | Fix |
|---------|-------|-----|
| OOM during sync | Insufficient RAM for state acquisition | Increase RAM to 32 GB or use `[node_size] medium` |
| RocksDB build segfault | GCC OOM during compilation | `sudo apt install librocksdb-dev` or `CARGO_BUILD_JOBS=1` |
| OpenSSL build failure | Missing system OpenSSL | `sudo apt install libssl-dev pkg-config` |
| Node stuck in "connected" | Validator list not loading | Ensure `[validators_file]` points to valid file, or add `[validator_list_sites]` directly to config |
| Slow sync | Spinning disk or limited bandwidth | Use NVMe SSD, ensure 100+ Mbps |
| Port already in use | Another process on same port | Check with `lsof -i :51235`, change port in config |
| No peers connecting | Firewall blocking port 51235 | Open TCP 51235 inbound |
| `.cargo/config.toml` error | lld linker not installed | `rm .cargo/config.toml` or `sudo apt install lld clang` |
