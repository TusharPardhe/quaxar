# Running Quaxar

Guide for node operators running the Rust implementation of the XRP Ledger server.

## Capacity Planning

| Resource | Starting point | Notes |
|----------|----------------|-------|
| CPU | 4 modern cores | More cores help builds, RPC, acquisition, and jobs. |
| RAM | 16 GiB testnet; 32 GiB public-network evaluation | Size for the chosen cache profile and workload. |
| Disk | Fast SSD/NVMe | Capacity depends primarily on history and deletion policy. |
| Network | Stable broadband with public peer ingress | Sustained catch-up and peer relay can be bandwidth intensive. |

These are planning baselines, not guarantees. Measure the actual network,
history, RPC traffic, and `[node_size]` profile before production use.

## Supported Platforms

- Linux x86_64 (Ubuntu 22.04+, Debian 12+, RHEL 9+)
- macOS arm64 (Apple Silicon)
- macOS x86_64
- Windows has a build/install script; server-service operation is not yet part
  of the qualified platform matrix.

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
- Generate config files with guided essential settings; set
  `QUAXAR_ADVANCED_CONFIG=1` for the additional installer prompts
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
For the dedicated system service below, install a root-owned copy:

```bash
sudo install -o root -g root -m 0755 ~/.cargo/bin/quaxar /usr/local/bin/quaxar
```

### Build Troubleshooting

**RocksDB compilation segfault / OOM:**

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

### Build Notes

- **System RocksDB:** Install `librocksdb-dev` before setting `ROCKSDB_LIB_DIR` to the directory that actually contains the installed library.
- **Bundled RocksDB:** Without a system library, the crate compiles RocksDB from source; allow additional build time and memory.
- **Low-memory machines:** Use `CARGO_BUILD_JOBS=2` to limit parallelism
- **Faster linking:** `.cargo/config.toml` contains commented examples. Install `clang` and `lld` before enabling the stanza for your target.

## Configuration

Use the repository `quaxar.cfg` as the default starting point. The following is
a smaller bare-metal example; detailed parameter explanations are kept in
[CONFIGURATION.md](CONFIGURATION.md).

```ini
[server]
port_rpc_admin_local
port_peer

[port_rpc_admin_local]
port = 5005
ip = 127.0.0.1
protocol = http
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
online_delete = 512
advisory_delete = 0

[database_path]
/var/lib/quaxar/db

[ledger_history]
256

[validator_list_sites]
https://vl.ripple.com

[validator_list_keys]
ED2677ABFFD1B33AC6FBC3062B71F1E8397C1505E1C42C64D11AD1B28FF73F4734

[ips]
s1.ripple.com 51235
s2.ripple.com 51235
```

### Validator List

Configure the trusted validator-list publisher directly in `quaxar.cfg`:

```ini
[validator_list_sites]
https://vl.ripple.com

[validator_list_keys]
ED2677ABFFD1B33AC6FBC3062B71F1E8397C1505E1C42C64D11AD1B28FF73F4734
```

### Configuration Sections

| Section | Purpose |
|---------|---------|
| `[server]` | Lists port definitions to activate |
| `[port_*]` | Port binding: port number, IP, protocol (http/ws/peer) |
| `[node_db]` | Database backend (NuDB), path, deletion policy |
| `[node_size]` | Memory tuning: tiny, small, medium, large, huge |
| `[validator_list_sites]` | URLs to fetch trusted validator lists |
| `[validator_list_keys]` | Public keys of validator list publishers |
| `[ips]` | Peer endpoints to connect to on startup |

See [CONFIGURATION.md](CONFIGURATION.md) for operator-facing config sections
and compatibility notes.

The root example binds HTTP/WebSocket administration to `127.0.0.1` and is safe
for the bare-metal service below. Docker Compose uses separate files under
`infra/docker/` that listen inside the container while publishing admin ports
only on host loopback. Never expose an admin listener publicly.

## Starting the Node

```bash
RUST_LOG=info ./target/release/quaxar --conf quaxar.cfg
```

Background:
```bash
RUST_LOG=info nohup ./target/release/quaxar --conf quaxar.cfg > quaxar.log 2>&1 &
```

## Systemd Service

Create a dedicated account and writable state directories when not using the
interactive installer:

```bash
sudo useradd --system --home /var/lib/quaxar --shell /usr/sbin/nologin quaxar
sudo install -d -o quaxar -g quaxar /var/lib/quaxar /var/log/quaxar
sudo install -d -o root -g quaxar -m 0750 /etc/quaxar
sudo install -o root -g quaxar -m 0640 quaxar.cfg /etc/quaxar/quaxar.cfg
```

Then create `/etc/systemd/system/quaxar.service`:

```ini
[Unit]
Description=Quaxar XRP Ledger Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=quaxar
Group=quaxar
ExecStart=/usr/local/bin/quaxar --conf /etc/quaxar/quaxar.cfg
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

For an older or custom-layout host, first record the unit's `User`, `Group`,
`ExecStart`, config path, database paths, and a tested rollback point. Build and
stage the new binary before the outage; then stop both old and new units before
moving data or changing ownership. Copy (do not remove) the protected legacy
config to `/etc/quaxar/quaxar.cfg`, update and validate its paths/listeners, and
transfer only the directories actually named by that config. Start Quaxar
without disabling the old unit, require sustained `full`/`proposing` operation,
advancing local-closed and validated ledgers, no recurring mode churn, and a
writable database before enabling Quaxar and disabling the old unit. On
failure, stop Quaxar, restore paths/ownership, and restart the old unit. Never
run both daemons against one NuDB. Deployments that still intentionally run
`quaxar.service` under an `xrpld` account must keep that ownership in deployment
commands until this explicit cutover is performed.

## Monitoring

### Liveness and readiness

```bash
quaxar health
# Exit code 0 = reachable (including while syncing)
# Exit code 1 = unreachable (down)
```

`health` is a liveness check, not a readiness gate. Sample these commands over
multiple ledger closes before declaring a node ready:

```bash
quaxar ledger-closed
quaxar ledger-current
quaxar server-info
quaxar fetch-info
```

Or via RPC:

```bash
curl -s http://127.0.0.1:5005 -d '{"method":"server_info"}' | jq .result.info.server_state
```

A non-validator node normally progresses through `connected`, `syncing`, `tracking`, and `full`; it must not enter `proposing` without validator credentials.

### System Time

Quaxar reads the host operating system's UTC clock. On hosts where NTP can be
configured, use your platform's standard time synchronisation:

```bash
timedatectl status
sudo timedatectl set-ntp true
```

**For LXC containers, Docker, or managed VPS** where host NTP cannot be
configured, quaxar supports a built-in SNTP client via the `[sntp_servers]`
configuration section (matching rippled's former `[sntp_servers]` support):

```ini
[sntp_servers]
time.windows.com
time.apple.com
time.nist.gov
pool.ntp.org
```

When configured, quaxar queries these servers in the background and applies the
computed clock offset to the node's time. This ensures correct time
synchronisation even in containerised environments where the host kernel's NTP
daemon is inaccessible.

Use your platform's standard NTP service or the `[sntp_servers]` section before
running a production node.

### Database Usage

```bash
quaxar db-stats --conf /etc/quaxar/quaxar.cfg
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
RUST_LOG=info ./quaxar --conf quaxar.cfg

# Per-module control
RUST_LOG=info,consensus=debug,overlay=warn ./quaxar --conf quaxar.cfg
```

Change at runtime:

```bash
quaxar log-level debug
```

`quaxar log-rotate` is currently a compatibility no-op because the runtime
does not own a logfile; use the systemd/Docker supervisor's rotation policy.

## Priming the NodeStore from a Snapshot

A snapshot transfers immutable node objects from an existing node. Importing
one can reduce later network fetches, but it does not currently install
relational ledger metadata, select the snapshot ledger as the startup LCL, or
bypass normal network synchronization by itself.

**On the source node (online):**

```bash
quaxar export-snapshot --output /var/lib/quaxar/snapshots/bootstrap.xrpls
```

The node exports in a background job and remains online. The CLI displays a
spinner while it polls the admin `snapshot_status` RPC, then reports completion
or failure and the resulting file size. On an older node without
`snapshot_status`, the CLI reports that export was started and instructs the
operator to monitor snapshot logs instead. The snapshot uses LZ4-compressed
chunks with SHA-256 integrity verification; duration depends on store size and
host I/O.

**On the new node (stopped):**

```bash
quaxar load-snapshot --input /path/to/snapshot.xrpls --conf /etc/quaxar/quaxar.cfg
```

The CLI displays a spinner while it imports and verifies all chunk and final
file hashes. After it reports completion, start the node normally. Network
acquisition can reuse matching objects from the primed NodeStore while the node
establishes its current validated chain and relational metadata normally.

See [CLI.md](CLI.md) for command details and [SYNCING.md](SYNCING.md) for
alternative sync methods.

## Common Issues

| Problem | Cause | Fix |
|---------|-------|-----|
| OOM during sync | Cache/workload exceeds available memory | Select a smaller `[node_size]`, reduce competing workloads, or add RAM |
| RocksDB build segfault | GCC OOM during compilation | `sudo apt install librocksdb-dev` or `CARGO_BUILD_JOBS=1` |
| OpenSSL build failure | Missing system OpenSSL | `sudo apt install libssl-dev pkg-config` |
| Node stuck in "connected" | Validator list not loading | Ensure `[validators_file]` points to valid file, or add `[validator_list_sites]` directly to config |
| Slow sync | Slow storage, limited bandwidth, or unstable peers | Use fast SSD storage and verify sustained network throughput and peer stability |
| Full/proposing repeatedly returns to syncing | Preferred-LCL reconciliation is not converging | Compare repeated local closed, validated, published, and coordinator phase identities; capture mode-transition logs and `last_recovery_lcl_decision` |
| Validated advances while local closed/coordinator LCL lags | Local consensus or recovery-anchor state is stale | Capture `server_info`, `ledger-closed`, `fetch-info`, session counters, and a redacted config; do not treat RAM growth as proof of correctness |
| Port already in use | Another process on same port | Check with `lsof -i :51235`, change port in config |
| No peers connecting | Firewall blocking port 51235 | Open TCP 51235 inbound |
| Linker error after enabling optional config | The selected linker is unavailable | Install the configured linker or comment out only the operator-enabled target stanza |
