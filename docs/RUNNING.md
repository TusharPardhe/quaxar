# Running Quaxar

Guide for node operators running the Rust implementation of the XRP Ledger server.

## Hardware Requirements

| Resource | Minimum (mainnet) | Recommended |
|----------|-------------------|-------------|
| CPU | 4 cores | 8+ cores |
| RAM | 32 GB | 32 GB |
| Disk | 1 TB NVMe SSD | 1 TB NVMe SSD |
| Network | 100 Mbps | 1 Gbps |

For testnet, 16 GB RAM and a 500 GB SSD are sufficient.

Disk usage grows over time with ledger history. NVMe is strongly recommended for NuDB performance.

## Supported Platforms

- Linux x86_64 (Ubuntu 22.04+, Debian 12+, RHEL 9+)
- macOS arm64 (Apple Silicon)
- macOS x86_64

## Building from Source

### Automated Setup (recommended)

```bash
# Download and run the interactive installer
curl -sSf https://raw.githubusercontent.com/TusharPardhe/quaxar/main/install.sh -o install.sh
chmod +x install.sh
./install.sh

# Or non-interactive (all defaults, local build)
./install.sh -y
```

The installer will:
- Assess your hardware and warn if below requirements
- Let you choose Docker or local build
- Install all dependencies
- Build and install `quaxar` to your PATH
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
sudo apt install build-essential pkg-config libssl-dev librocksdb-dev clang cmake git
```

**macOS:**
```bash
brew install openssl rocksdb cmake
```

### Build & Install

```bash
git clone https://github.com/TusharPardhe/quaxar.git
cd quaxar
CC=clang CXX=clang++ cargo install --path xrpld/main
```

This builds the release binary and installs it to `~/.cargo/bin/quaxar` (already in PATH).

### Build Troubleshooting

**RocksDB compilation segfault / OOM (common on ≤16GB RAM):**

The RocksDB C++ library compiles from source by default and can exhaust memory. Fix by installing the system package:

```bash
# Linux
sudo apt install librocksdb-dev
ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu CC=clang CXX=clang++ cargo install --path xrpld/main
```

**Rustc segfault during build (too many parallel jobs):**

```bash
CARGO_BUILD_JOBS=2 CC=clang CXX=clang++ cargo install --path xrpld/main
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

Use the repository `xrpld.cfg` as the default starting point. It is intentionally
small; detailed parameter explanations are kept in
[CONFIGURATION.md](CONFIGURATION.md).

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
| `[ledger_acquisition]` | Optional expert override for cold-bootstrap active ledger acquisition count |
| `[validators_file]` | Path to file with validator list sites and keys |
| `[validator_list_sites]` | URLs to fetch trusted validator lists |
| `[validator_list_keys]` | Public keys of validator list publishers |
| `[ips]` | Peer endpoints to connect to on startup |

See [CONFIGURATION.md](CONFIGURATION.md) for every supported config section and
for guidance on `ledger_fetch_limit` tuning.

## Starting the Node

```bash
RUST_LOG=info ./target/release/quaxar --conf xrpld.cfg
```

Background:
```bash
RUST_LOG=info nohup ./target/release/quaxar --conf xrpld.cfg > quaxar.log 2>&1 &
```

## Systemd Service

Create `/etc/systemd/system/quaxar.service`:

```ini
[Unit]
Description=Quaxar XRP Ledger Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=xrpld
Group=xrpld
ExecStart=/usr/local/bin/quaxar --conf /etc/xrpld/xrpld.cfg
Restart=on-failure
RestartSec=10
LimitNOFILE=65536
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now quaxar
```

## Monitoring

### Health Check

```bash
quaxar health
# Exit code 0 = reachable (healthy or syncing)
# Exit code 1 = unreachable (down)
```

Or via RPC:

```bash
curl -s http://127.0.0.1:5005 -d '{"method":"server_info"}' | jq .result.info.server_state
```

A non-validator node normally progresses through `connected`, `syncing`, `tracking`, and `full`; it must not enter `proposing` without validator credentials.

### System Time

Quaxar reads the host operating system's UTC clock. It does not currently consume
rippled-style `[sntp_servers]` entries, so configure and monitor host NTP rather
than adding an inert configuration section. On supported Linux hosts:

```bash
timedatectl status
sudo timedatectl set-ntp true
```

Use your platform's standard NTP service or a managed time source before running
a production node.

### Database Usage

```bash
quaxar db-stats --conf /etc/xrpld.cfg
```

Shows the configured node-store path, NuDB data/key/log file sizes, total disk
usage, and live node-store counters when the local RPC server is reachable.

For raw counters:

```bash
quaxar get-counts
```

For one-off RPC checks:

```bash
quaxar server-info
quaxar server-state
quaxar rpc ledger '{"ledger_index":"validated"}'
```

## Log Management

Control log verbosity with the `RUST_LOG` environment variable:

```bash
# Levels: error, warn, info, debug, trace
RUST_LOG=info ./quaxar --conf xrpld.cfg

# Per-module control
RUST_LOG=info,consensus=debug,overlay=warn ./quaxar --conf xrpld.cfg
```

Change at runtime:

```bash
quaxar log-level debug
quaxar log-rotate
```

## Bootstrapping from Snapshot

The fastest way to bring up a new node is to load a snapshot exported from an
existing synced node. This bypasses the multi-hour network sync entirely.

**On the source node (online):**

```bash
quaxar rpc export_snapshot
```

The export runs in a background thread. On NVMe, 26.5M nodes export in ~3
minutes. The snapshot file uses LZ4 compressed chunks with SHA-256 integrity
verification.

**On the new node (stopped):**

```bash
quaxar load-snapshot --input /path/to/snapshot.lz4 --conf /etc/xrpld/xrpld.cfg
```

After import completes, start the node normally. It will resume from the
snapshot state and catch up to the network tip within minutes.

See [CLI.md](CLI.md) for command details and [SYNCING.md](SYNCING.md) for
alternative sync methods.

## Common Issues

| Problem | Cause | Fix |
|---------|-------|-----|
| OOM during sync | Insufficient RAM for state acquisition | Ensure 32 GB RAM for mainnet or use `[node_size] medium` |
| RocksDB build segfault | GCC OOM during compilation | `sudo apt install librocksdb-dev` or `CARGO_BUILD_JOBS=1` |
| OpenSSL build failure | Missing system OpenSSL | `sudo apt install libssl-dev pkg-config` |
| Node stuck in "connected" | Validator list not loading | Ensure `[validators_file]` points to valid file, or add `[validator_list_sites]` directly to config |
| Slow sync | Spinning disk or limited bandwidth | Use NVMe SSD, ensure 100+ Mbps |
| Port already in use | Another process on same port | Check with `lsof -i :51235`, change port in config |
| No peers connecting | Firewall blocking port 51235 | Open TCP 51235 inbound |
| `.cargo/config.toml` error | lld linker not installed | `rm .cargo/config.toml` or `sudo apt install lld clang` |
