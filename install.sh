#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# xrpld installer — interactive setup for the Rust XRPL node
# Usage: ./install.sh [-y] [--prefix PATH]
# ─────────────────────────────────────────────────────────────────────────────

VERSION="0.1.0"
REPO="https://github.com/TusharPardhe/xrpld.git"
AUTO_YES=false
PREFIX=""

# ── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
ORANGE='\033[38;5;208m'
DIM='\033[2m'
BOLD='\033[1m'
RESET='\033[0m'

ok()   { echo -e "  ${GREEN}✓${RESET} $1"; }
warn() { echo -e "  ${YELLOW}⚠${RESET} $1"; }
fail() { echo -e "  ${RED}✗${RESET} $1"; }
info() { echo -e "  ${DIM}$1${RESET}"; }
header() {
    echo ""
    echo -e "${ORANGE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "  ${BOLD}$1${RESET}"
    echo -e "${ORANGE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo ""
}

ask() {
    local prompt="$1" default="$2" var="$3"
    if [ "$AUTO_YES" = true ]; then
        eval "$var='$default'"
        return
    fi
    read -rp "  $prompt [$default]: " input
    eval "$var='${input:-$default}'"
}

ask_yn() {
    local prompt="$1" default="$2"
    if [ "$AUTO_YES" = true ]; then
        return 0
    fi
    read -rp "  $prompt [$default]: " input
    input="${input:-$default}"
    [[ "$input" =~ ^[Yy] ]]
}

# ── Parse args ───────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        -y|--yes) AUTO_YES=true; shift ;;
        --prefix) PREFIX="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: ./install.sh [-y] [--prefix PATH]"
            echo "  -y, --yes     Non-interactive mode (use all defaults)"
            echo "  --prefix      Install prefix (default: ~/.cargo/bin)"
            exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ── Banner ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${ORANGE}  ██╗  ██╗██████╗ ██████╗ ██╗     ██████╗ ${RESET}"
echo -e "${ORANGE}  ╚██╗██╔╝██╔══██╗██╔══██╗██║     ██╔══██╗${RESET}"
echo -e "${ORANGE}   ╚███╔╝ ██████╔╝██████╔╝██║     ██║  ██║${RESET}"
echo -e "${ORANGE}   ██╔██╗ ██╔══██╗██╔═══╝ ██║     ██║  ██║${RESET}"
echo -e "${ORANGE}  ██╔╝ ██╗██║  ██║██║     ███████╗██████╔╝${RESET}"
echo -e "${ORANGE}  ╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚══════╝╚═════╝ ${RESET}"
echo -e "                    ${DIM}v${VERSION} installer${RESET}"
echo ""

# ── System Detection ─────────────────────────────────────────────────────────
header "System Detection"

OS="unknown"
ARCH=$(uname -m)
PKG_MGR="none"

if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        OS="$NAME $VERSION_ID"
    fi
    if command -v apt-get &>/dev/null; then PKG_MGR="apt";
    elif command -v dnf &>/dev/null; then PKG_MGR="dnf";
    elif command -v pacman &>/dev/null; then PKG_MGR="pacman";
    fi
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macOS $(sw_vers -productVersion)"
    PKG_MGR="brew"
fi

echo -e "  OS              ${BOLD}$OS${RESET} (${ARCH})"
echo -e "  Package Mgr     ${BOLD}$PKG_MGR${RESET}"

if [ "$PKG_MGR" = "none" ]; then
    fail "No supported package manager found"
    exit 1
fi

# ── Hardware Assessment ──────────────────────────────────────────────────────
header "Hardware Assessment"

# CPU
if [[ "$OSTYPE" == "darwin"* ]]; then
    CPU_CORES=$(sysctl -n hw.ncpu)
    CPU_MODEL=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "Apple Silicon")
else
    CPU_CORES=$(nproc)
    CPU_MODEL=$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)
fi

# RAM (in GB)
if [[ "$OSTYPE" == "darwin"* ]]; then
    RAM_BYTES=$(sysctl -n hw.memsize)
    RAM_GB=$((RAM_BYTES / 1073741824))
else
    RAM_GB=$(free -g | awk '/^Mem:/{print $2}')
fi

# Disk (available in GB)
if [[ "$OSTYPE" == "darwin"* ]]; then
    DISK_AVAIL=$(df -g / | awk 'NR==2{print $4}')
else
    DISK_AVAIL=$(df -BG / | awk 'NR==2{print $4}' | tr -d 'G')
fi

# Display with status indicators
cpu_status="${GREEN}✓${RESET}"
[ "$CPU_CORES" -lt 4 ] && cpu_status="${RED}✗${RESET}"
[ "$CPU_CORES" -ge 4 ] && [ "$CPU_CORES" -lt 8 ] && cpu_status="${YELLOW}⚠${RESET}"

ram_status="${GREEN}✓${RESET}"
[ "$RAM_GB" -lt 16 ] && ram_status="${RED}✗${RESET}"
[ "$RAM_GB" -ge 16 ] && [ "$RAM_GB" -lt 32 ] && ram_status="${YELLOW}⚠${RESET}"

disk_status="${GREEN}✓${RESET}"
[ "$DISK_AVAIL" -lt 500 ] && disk_status="${RED}✗${RESET}"
[ "$DISK_AVAIL" -ge 500 ] && [ "$DISK_AVAIL" -lt 1000 ] && disk_status="${YELLOW}⚠${RESET}"

printf "  CPU             %-4s cores  %-40s %b\n" "$CPU_CORES" "($CPU_MODEL)" "$cpu_status"
printf "  RAM             %-4s GB     %-40s %b\n" "$RAM_GB" "(min: 16 GB, recommended: 32 GB)" "$ram_status"
printf "  Disk            %-4s GB     %-40s %b\n" "$DISK_AVAIL" "(min: 500 GB, recommended: 1 TB)" "$disk_status"

echo ""
info "Minimum: 4 cores, 16 GB RAM, 500 GB disk"
info "Recommended: 8+ cores, 32 GB RAM, 1 TB NVMe"

# Warn if below minimum
if [ "$RAM_GB" -lt 16 ] || [ "$CPU_CORES" -lt 4 ] || [ "$DISK_AVAIL" -lt 500 ]; then
    echo ""
    warn "Your system does not meet minimum requirements."
    warn "The node may fail during initial sync (OOM) or run out of disk space."
    if [ "$AUTO_YES" = false ]; then
        read -rp "  Continue anyway? [y/N]: " confirm
        [[ ! "$confirm" =~ ^[Yy] ]] && echo "  Aborted." && exit 1
    fi
fi

# ── Install Method ────────────────────────────────────────────────────────────
header "Install Method"

INSTALL_METHOD="local"
echo -e "  ${BOLD}1)${RESET} Local build (cargo install)"
echo -e "  ${BOLD}2)${RESET} Docker (requires Docker installed)"
echo ""

if [ "$AUTO_YES" = false ]; then
    read -rp "  Choose install method [1]: " method_choice
    case "${method_choice:-1}" in
        2) INSTALL_METHOD="docker" ;;
        *) INSTALL_METHOD="local" ;;
    esac
fi

if [ "$INSTALL_METHOD" = "docker" ]; then
    if ! command -v docker &>/dev/null; then
        fail "Docker is not installed"
        info "Install Docker: https://docs.docker.com/engine/install/"
        exit 1
    fi
    ok "Docker detected: $(docker --version | awk '{print $3}' | tr -d ',')"
    
    CLONE_DIR="${PREFIX:-$HOME/xrpld}"
    if [ -d "$CLONE_DIR/.git" ]; then
        cd "$CLONE_DIR" && git pull --ff-only
    else
        info "Cloning $REPO..."
        git clone "$REPO" "$CLONE_DIR"
        cd "$CLONE_DIR"
    fi
    
    info "Building Docker image..."
    docker compose build
    ok "Docker image built"
    
    if ask_yn "Start xrpld container now?" "Y"; then
        docker compose up -d
        ok "xrpld container started"
        echo ""
        echo -e "  ${BOLD}Next steps:${RESET}"
        echo -e "    docker logs -f xrpld       ${DIM}Follow logs${RESET}"
        echo -e "    docker exec xrpld xrpld status  ${DIM}Check status${RESET}"
        echo -e "    docker compose down         ${DIM}Stop${RESET}"
    fi
    exit 0
fi

# ── Dependencies (local build only) ─────────────────────────────────────────
header "Dependencies"

MISSING=()

# Rust
if command -v rustc &>/dev/null; then
    RUST_VER=$(rustc --version | awk '{print $2}')
    ok "Rust $RUST_VER"
else
    fail "Rust (not installed)"
    MISSING+=("rust")
fi

# System packages
check_pkg() {
    local name="$1" cmd="$2" pkg="$3"
    if command -v "$cmd" &>/dev/null || pkg-config --exists "$name" 2>/dev/null; then
        ok "$name"
    else
        fail "$name → will install ${DIM}($pkg)${RESET}"
        MISSING+=("$pkg")
    fi
}

if [ "$PKG_MGR" = "apt" ]; then
    check_pkg "OpenSSL" "openssl" "libssl-dev"
    check_pkg "RocksDB" "" "librocksdb-dev"
    dpkg -s librocksdb-dev &>/dev/null 2>&1 && ok "RocksDB (librocksdb-dev)" || { fail "RocksDB → will install ${DIM}(librocksdb-dev)${RESET}"; MISSING+=("librocksdb-dev"); }
    check_pkg "clang" "clang" "clang"
    check_pkg "cmake" "cmake" "cmake"
    check_pkg "pkg-config" "pkg-config" "pkg-config"
elif [ "$PKG_MGR" = "brew" ]; then
    check_pkg "OpenSSL" "openssl" "openssl"
    check_pkg "RocksDB" "" "rocksdb"
    check_pkg "cmake" "cmake" "cmake"
fi

# Install missing
if [ ${#MISSING[@]} -gt 0 ]; then
    echo ""
    if ask_yn "Install missing dependencies?" "Y"; then
        # Rust
        if [[ " ${MISSING[*]} " =~ " rust " ]]; then
            info "Installing Rust..."
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            source "$HOME/.cargo/env"
            ok "Rust installed"
        fi

        # System packages
        SYS_PKGS=()
        for pkg in "${MISSING[@]}"; do
            [ "$pkg" != "rust" ] && SYS_PKGS+=("$pkg")
        done

        if [ ${#SYS_PKGS[@]} -gt 0 ]; then
            info "Installing system packages: ${SYS_PKGS[*]}"
            if [ "$PKG_MGR" = "apt" ]; then
                sudo apt-get update -qq && sudo apt-get install -y -qq "${SYS_PKGS[@]}"
            elif [ "$PKG_MGR" = "dnf" ]; then
                sudo dnf install -y "${SYS_PKGS[@]}"
            elif [ "$PKG_MGR" = "brew" ]; then
                brew install "${SYS_PKGS[@]}"
            fi
            ok "System packages installed"
        fi
    else
        warn "Skipping dependency installation. Build may fail."
    fi
fi

# ── Build & Install ──────────────────────────────────────────────────────────
header "Build & Install"

# Ensure cargo is in PATH
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

CLONE_DIR="${PREFIX:-$HOME/xrpld}"

if [ -d "$CLONE_DIR/.git" ]; then
    info "Repository exists at $CLONE_DIR, pulling latest..."
    cd "$CLONE_DIR" && git pull --ff-only
else
    info "Cloning $REPO..."
    git clone "$REPO" "$CLONE_DIR"
    cd "$CLONE_DIR"
fi

# Remove .cargo/config.toml if lld not available
if [ -f .cargo/config.toml ] && ! command -v lld &>/dev/null; then
    rm -f .cargo/config.toml
    info "Removed .cargo/config.toml (lld not installed)"
fi

# Set ROCKSDB_LIB_DIR if system lib available
if [ "$PKG_MGR" = "apt" ] && dpkg -s librocksdb-dev &>/dev/null 2>&1; then
    export ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu
fi

info "Building xrpld (this may take a few minutes)..."
if [ "$RAM_GB" -le 16 ]; then
    CARGO_BUILD_JOBS=2 cargo install --path xrpld/main --force 2>&1 | tail -1
else
    cargo install --path xrpld/main --force 2>&1 | tail -1
fi

ok "xrpld installed to $(which xrpld || echo ~/.cargo/bin/xrpld)"

# ── Configuration ────────────────────────────────────────────────────────────
header "Configuration"

CONF_DIR="/etc/xrpld"
CONF_FILE="$CONF_DIR/xrpld.cfg"
GENERATE_CONF=true

if [ -f "$CONF_FILE" ] && [ "$AUTO_YES" = false ]; then
    warn "Config already exists at $CONF_FILE"
    ask_yn "Overwrite?" "N" || GENERATE_CONF=false
fi

if [ "$GENERATE_CONF" = true ]; then
    # Defaults
    RPC_PORT="5005"
    RPC_IP="127.0.0.1"
    PEER_PORT="51235"
    PEER_IP="0.0.0.0"
    WS_PORT="6006"
    WS_IP="127.0.0.1"
    DB_TYPE="NuDB"
    DATA_DIR="/var/lib/xrpld"
    DB_PATH="$DATA_DIR/db/nudb"
    SQLITE_PATH="$DATA_DIR/db"
    ONLINE_DELETE="512"
    NODE_SIZE="medium"
    LEDGER_HISTORY="256"
    NETWORK="mainnet"
    LOG_FILE="/var/log/xrpld/xrpld.log"
    LOG_LEVEL="info"

    if [ "$AUTO_YES" = false ]; then
        echo -e "  ${DIM}Configure your node (press Enter for defaults):${RESET}"
        echo ""

        echo -e "  ${BOLD}── Ports ──${RESET}"
        ask "RPC port" "$RPC_PORT" RPC_PORT
        ask "RPC bind IP" "$RPC_IP" RPC_IP
        ask "Peer port" "$PEER_PORT" PEER_PORT
        ask "Peer bind IP" "$PEER_IP" PEER_IP
        ask "WebSocket port" "$WS_PORT" WS_PORT
        ask "WebSocket bind IP" "$WS_IP" WS_IP

        echo ""
        echo -e "  ${BOLD}── Database ──${RESET}"
        ask "Database type (NuDB/RocksDB)" "$DB_TYPE" DB_TYPE
        ask "Data directory" "$DATA_DIR" DATA_DIR
        DB_PATH="$DATA_DIR/db/nudb"
        SQLITE_PATH="$DATA_DIR/db"
        ask "Online delete (ledgers)" "$ONLINE_DELETE" ONLINE_DELETE

        echo ""
        echo -e "  ${BOLD}── Node ──${RESET}"
        info "Node size determines memory usage:"
        info "  tiny=4GB  small=8GB  medium=16GB  large=32GB  huge=64GB"
        ask "Node size" "$NODE_SIZE" NODE_SIZE
        ask "Ledger history (number or 'full')" "$LEDGER_HISTORY" LEDGER_HISTORY

        echo ""
        echo -e "  ${BOLD}── Network ──${RESET}"
        ask "Network (mainnet/testnet/devnet)" "$NETWORK" NETWORK

        echo ""
        echo -e "  ${BOLD}── Logging ──${RESET}"
        ask "Log file" "$LOG_FILE" LOG_FILE
        ask "Log level (error/warn/info/debug)" "$LOG_LEVEL" LOG_LEVEL
    fi

    # Network-specific settings
    case "$NETWORK" in
        mainnet)
            VL_SITE="https://vl.ripple.com"
            VL_KEY="ED2677ABFFD1B33AC6FBC3062B71F1E8397C1505E1C42C64D11AD1B28FF73F4734"
            PEERS="s1.ripple.com 51235\ns2.ripple.com 51235"
            ;;
        testnet)
            VL_SITE="https://vl.altnet.rippletest.net"
            VL_KEY="ED264807102805220DA0F312E71FC2C69E1552C9C5790F6C25E3729DEB573D5860"
            PEERS="s.altnet.rippletest.net 51235"
            ;;
        devnet)
            VL_SITE="https://vl.devnet.rippletest.net"
            VL_KEY="EDDF2F53DFEC79C1EAAB2C1E8B1F2B4C85B0C264B37C2B8B8E4E3E6F0D5A7C8B9"
            PEERS="s.devnet.rippletest.net 51235"
            ;;
    esac

    # Create directories
    sudo mkdir -p "$CONF_DIR" "$DATA_DIR/db/nudb" "$(dirname "$LOG_FILE")"
    sudo chown -R "$(whoami)" "$DATA_DIR" "$(dirname "$LOG_FILE")"

    # Write xrpld.cfg
    sudo tee "$CONF_FILE" > /dev/null << EOF
[server]
port_rpc_admin_local
port_peer
port_ws_admin_local

[port_rpc_admin_local]
port = $RPC_PORT
ip = $RPC_IP
protocol = http

[port_peer]
port = $PEER_PORT
ip = $PEER_IP
protocol = peer

[port_ws_admin_local]
port = $WS_PORT
ip = $WS_IP
protocol = ws

[node_size]
$NODE_SIZE

[node_db]
type = $DB_TYPE
path = $DB_PATH
online_delete = $ONLINE_DELETE
advisory_delete = 0

[database_path]
$SQLITE_PATH

[ledger_history]
$LEDGER_HISTORY

[validators_file]
$CONF_DIR/validators.txt

[debug_logfile]
$LOG_FILE

[ips]
$(echo -e "$PEERS")

[rpc_startup]
{"command": "log_level", "severity": "$LOG_LEVEL"}

[ssl_verify]
0
EOF

    # Write validators.txt
    sudo tee "$CONF_DIR/validators.txt" > /dev/null << EOF
[validator_list_sites]
$VL_SITE

[validator_list_keys]
$VL_KEY
EOF

    ok "Config written to $CONF_FILE"
    ok "Validators written to $CONF_DIR/validators.txt"
fi

# ── Systemd Service ──────────────────────────────────────────────────────────
header "Service Setup"

INSTALL_SERVICE=true
if [ "$AUTO_YES" = false ]; then
    ask_yn "Install systemd service?" "Y" || INSTALL_SERVICE=false
fi

if [ "$INSTALL_SERVICE" = true ] && command -v systemctl &>/dev/null; then
    XRPLD_BIN=$(which xrpld 2>/dev/null || echo "$HOME/.cargo/bin/xrpld")

    sudo tee /etc/systemd/system/xrpld.service > /dev/null << EOF
[Unit]
Description=XRP Ledger Node (Rust)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$(whoami)
ExecStart=$XRPLD_BIN --conf $CONF_FILE
Restart=on-failure
RestartSec=10
LimitNOFILE=65536
Environment=RUST_LOG=$LOG_LEVEL

[Install]
WantedBy=multi-user.target
EOF

    sudo systemctl daemon-reload
    sudo systemctl enable xrpld

    ok "Service installed (xrpld.service)"

    if ask_yn "Start xrpld now?" "Y"; then
        sudo systemctl start xrpld
        sleep 3
        ok "xrpld started"
    fi
elif [ "$INSTALL_SERVICE" = true ]; then
    warn "systemd not available — skipping service setup"
    info "Start manually: RUST_LOG=$LOG_LEVEL xrpld --conf $CONF_FILE"
fi

# ── Verification ─────────────────────────────────────────────────────────────
header "Verification"

XRPLD_BIN=$(which xrpld 2>/dev/null || echo "$HOME/.cargo/bin/xrpld")

if [ -x "$XRPLD_BIN" ]; then
    VER=$("$XRPLD_BIN" version 2>/dev/null | grep -i version | head -1 || echo "installed")
    ok "Binary: $XRPLD_BIN"
    ok "$VER"
else
    fail "Binary not found"
fi

if systemctl is-active xrpld &>/dev/null 2>&1; then
    sleep 2
    "$XRPLD_BIN" health --rpc-url "http://127.0.0.1:${RPC_PORT:-5005}" 2>/dev/null || true
fi

# ── Summary ──────────────────────────────────────────────────────────────────
header "Installation Complete"

echo -e "  Binary:     ${BOLD}$XRPLD_BIN${RESET}"
echo -e "  Config:     ${BOLD}${CONF_FILE:-/etc/xrpld/xrpld.cfg}${RESET}"
echo -e "  Data:       ${BOLD}${DATA_DIR:-/var/lib/xrpld}${RESET}"
echo -e "  Network:    ${BOLD}${NETWORK:-mainnet}${RESET}"
echo -e "  RPC:        ${BOLD}http://${RPC_IP:-127.0.0.1}:${RPC_PORT:-5005}${RESET}"
echo -e "  Peer:       ${BOLD}${PEER_IP:-0.0.0.0}:${PEER_PORT:-51235}${RESET}"
echo ""
echo -e "  ${BOLD}Next steps:${RESET}"
echo -e "    xrpld status          ${DIM}Check node status${RESET}"
echo -e "    xrpld cli             ${DIM}Interactive mode${RESET}"
echo -e "    xrpld peers           ${DIM}View connected peers${RESET}"
echo -e "    journalctl -u xrpld   ${DIM}Follow logs${RESET}"
echo ""
