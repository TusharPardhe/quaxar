#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# quaxar installer — interactive setup for the Rust XRPL node
# Usage: ./install.sh [-y] [--prefix PATH]
# ─────────────────────────────────────────────────────────────────────────────

VERSION="0.1.0"
REPO="https://github.com/TusharPardhe/quaxar.git"
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

invalid() {
    echo -e "  ${RED}✗${RESET} $1"
}

is_uint() {
    [[ "$1" =~ ^[0-9]+$ ]]
}

ask_choice() {
    local prompt="$1" default="$2" var="$3" choices="$4"
    local input normalized
    if [ "$AUTO_YES" = true ]; then
        eval "$var='$default'"
        return
    fi
    while true; do
        read -rp "  $prompt [$default]: " input
        input="${input:-$default}"
        normalized="$(printf "%s" "$input" | tr '[:upper:]' '[:lower:]')"
        if [[ " $choices " == *" $normalized "* ]]; then
            eval "$var='$normalized'"
            return
        fi
        invalid "Invalid value '$input'. Allowed values: $choices"
    done
}

ask_bool_value() {
    local prompt="$1" default="$2" var="$3"
    local input normalized
    if [ "$AUTO_YES" = true ]; then
        eval "$var='$default'"
        return
    fi
    while true; do
        read -rp "  $prompt [$default]: " input
        input="${input:-$default}"
        normalized="$(printf "%s" "$input" | tr '[:upper:]' '[:lower:]')"
        case "$normalized" in
            1|true|yes|y) eval "$var='1'"; return ;;
            0|false|no|n) eval "$var='0'"; return ;;
            *) invalid "Invalid value '$input'. Use 1/0, true/false, or yes/no." ;;
        esac
    done
}

ask_int_range() {
    local prompt="$1" default="$2" var="$3" min="$4" max="$5"
    local input
    if [ "$AUTO_YES" = true ]; then
        eval "$var='$default'"
        return
    fi
    while true; do
        read -rp "  $prompt [$default]: " input
        input="${input:-$default}"
        if is_uint "$input" && [ "$input" -ge "$min" ] && [ "$input" -le "$max" ]; then
            eval "$var='$input'"
            return
        fi
        invalid "Invalid value '$input'. Must be an integer from $min to $max."
    done
}

ask_optional_int_range() {
    local prompt="$1" default="$2" var="$3" min="$4" max="$5"
    local input
    if [ "$AUTO_YES" = true ]; then
        eval "$var='$default'"
        return
    fi
    while true; do
        read -rp "  $prompt [$default]: " input
        input="${input:-$default}"
        if [ -z "$input" ]; then
            eval "$var=''"
            return
        fi
        if is_uint "$input" && [ "$input" -ge "$min" ] && [ "$input" -le "$max" ]; then
            eval "$var='$input'"
            return
        fi
        invalid "Invalid value '$input'. Leave blank or use an integer from $min to $max."
    done
}

ask_port() {
    ask_int_range "$1" "$2" "$3" 1 65535
}

ask_ledger_history() {
    local prompt="$1" default="$2" var="$3"
    local input normalized
    if [ "$AUTO_YES" = true ]; then
        eval "$var='$default'"
        return
    fi
    while true; do
        read -rp "  $prompt [$default]: " input
        input="${input:-$default}"
        normalized="$(printf "%s" "$input" | tr '[:upper:]' '[:lower:]')"
        if [ "$normalized" = "full" ] || is_uint "$input"; then
            eval "$var='$normalized'"
            return
        fi
        invalid "Invalid value '$input'. Use a ledger count or 'full'."
    done
}

ask_network_id() {
    local prompt="$1" default="$2" var="$3"
    local input normalized
    if [ "$AUTO_YES" = true ]; then
        eval "$var='$default'"
        return
    fi
    while true; do
        read -rp "  $prompt [$default]: " input
        input="${input:-$default}"
        normalized="$(printf "%s" "$input" | tr '[:upper:]' '[:lower:]')"
        if [ -z "$normalized" ] || [[ " main testnet devnet " == *" $normalized "* ]] || is_uint "$normalized"; then
            eval "$var='$normalized'"
            return
        fi
        invalid "Invalid value '$input'. Leave blank, use main/testnet/devnet, or use a numeric network ID."
    done
}

ensure_history_not_above_online_delete() {
    if [ "$LEDGER_HISTORY" = "full" ]; then
        if [ "$ONLINE_DELETE" != "0" ]; then
            invalid "ledger_history=full requires online_delete=0 because online deletion cannot retain full history."
            if [ "$AUTO_YES" = true ]; then
                exit 1
            fi
            if ask_yn "Set online_delete to 0 and keep full history?" "Y"; then
                ONLINE_DELETE="0"
            else
                ask_ledger_history "Ledger history" "$ONLINE_DELETE" LEDGER_HISTORY
            fi
        fi
        return
    fi
    if [ "$ONLINE_DELETE" = "0" ]; then
        return
    fi
    while [ "$LEDGER_HISTORY" -gt "$ONLINE_DELETE" ]; do
        invalid "ledger_history ($LEDGER_HISTORY) cannot be greater than online_delete ($ONLINE_DELETE)."
        if [ "$AUTO_YES" = true ]; then
            exit 1
        fi
        ask_ledger_history "Ledger history" "$ONLINE_DELETE" LEDGER_HISTORY
    done
}

validate_config_inputs() {
    local failed=false

    if [ "$RPC_PORT" = "$PEER_PORT" ] || [ "$RPC_PORT" = "$WS_PORT" ] || [ "$PEER_PORT" = "$WS_PORT" ]; then
        fail "RPC, peer, and WebSocket ports must be distinct."
        failed=true
    fi

    for value_name in DATA_DIR DB_PATH SQLITE_PATH LOG_FILE VALIDATORS_FILE; do
        if [ -z "${!value_name}" ]; then
            fail "$value_name cannot be empty."
            failed=true
        fi
    done

    if [ "$DB_TYPE" = "RocksDB" ] && [ "$LEDGER_HISTORY" = "full" ]; then
        warn "RocksDB with full history can use significant disk and IO. NuDB is the default for non-validator/full-history testing."
    fi

    if [ "$failed" = true ]; then
        exit 1
    fi
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

as_lines() {
    printf "%s" "$1" | tr ',' '\n' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | sed '/^$/d'
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

# Interactive input must come from the operator's terminal rather than the
# script stream, so `curl ... | bash` remains usable from an interactive shell.
if [ "$AUTO_YES" = false ]; then
    if ! { : </dev/tty; }; then
        fail "Interactive setup requires a terminal. Download the script first or re-run with -y."
        exit 1
    fi
    read() {
        if ! builtin read "$@" </dev/tty; then
            fail "Unable to read from the terminal. Download the script first or re-run with -y."
            exit 1
        fi
    }
fi

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
elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "mingw"* || "$OSTYPE" == "cygwin" ]]; then
    OS="Windows ($(uname -s))"
    PKG_MGR="none"
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
    while true; do
        read -rp "  Choose install method [1]: " method_choice
        case "${method_choice:-1}" in
        2) INSTALL_METHOD="docker" ;;
        1) INSTALL_METHOD="local" ;;
        *) invalid "Invalid install method '${method_choice}'. Choose 1 or 2."; continue ;;
        esac
        break
    done
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
    
    if ask_yn "Start quaxar container now?" "Y"; then
        docker compose up -d
        ok "quaxar container started"
        echo ""
        echo -e "  ${BOLD}Next steps:${RESET}"
        echo -e "    docker logs -f quaxar       ${DIM}Follow logs${RESET}"
        echo -e "    docker exec quaxar quaxar status  ${DIM}Check status${RESET}"
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

check_pkg "Git" "git" "git"

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

info "Building quaxar (this may take a few minutes)..."
if command -v clang &>/dev/null && command -v clang++ &>/dev/null; then
    export CC=clang CXX=clang++
    info "Using clang and clang++ for native dependencies"
fi
if [ "$RAM_GB" -le 16 ]; then
    CARGO_BUILD_JOBS=2 cargo install --path xrpld/main --locked --force 2>&1 | tail -1
else
    cargo install --path xrpld/main --locked --force 2>&1 | tail -1
fi

ok "quaxar installed to $(which quaxar || echo ~/.cargo/bin/quaxar)"

# ── Configuration ────────────────────────────────────────────────────────────
header "Configuration"

CONF_DIR="/etc/xrpld"
CONF_FILE="$CONF_DIR/xrpld.cfg"
GENERATE_CONF=true

# Fall back to user-local config if sudo not available
if ! sudo -n true 2>/dev/null; then
    CONF_DIR="$HOME/.config/xrpld"
    CONF_FILE="$CONF_DIR/xrpld.cfg"
fi

if [ -f "$CONF_FILE" ] && [ "$AUTO_YES" = false ]; then
    warn "Config already exists at $CONF_FILE"
    ask_yn "Overwrite?" "N" || GENERATE_CONF=false
fi

if [ "$GENERATE_CONF" = true ]; then
    # Defaults
    RPC_PORT="5005"
    RPC_IP="0.0.0.0"
    RPC_ADMIN="127.0.0.1"
    RPC_SECURE_GATEWAY=""
    PEER_PORT="51235"
    PEER_IP="0.0.0.0"
    WS_PORT="6006"
    WS_IP="0.0.0.0"
    WS_ADMIN="127.0.0.1"
    WS_SECURE_GATEWAY=""
    WS_SEND_QUEUE_LIMIT="500"
    DB_TYPE="NuDB"
    DATA_DIR="$HOME/.local/share/quaxar"
    DB_PATH="$DATA_DIR/db/nudb"
    SQLITE_PATH="$DATA_DIR/db"
    NUDB_BLOCK_SIZE="4096"
    ONLINE_DELETE="512"
    ADVISORY_DELETE="0"
    NODE_SIZE="medium"
    LEDGER_HISTORY="256"
    NETWORK="mainnet"
    NETWORK_ID=""
    VALIDATORS_FILE="$CONF_DIR/validators.txt"
    SSL_VERIFY="1"
    OVERLAY_PUBLIC_IP=""
    OVERLAY_IP_LIMIT="0"
    OVERLAY_VERIFY_ENDPOINTS="1"
    CRAWL_ENABLED="1"
    CRAWL_OVERLAY="1"
    CRAWL_SERVER="1"
    CRAWL_COUNTS="0"
    CRAWL_UNL="1"
    VL_ENABLED="1"
    REDUCE_RELAY_VP_ENABLE="0"
    REDUCE_RELAY_VP_MAX_SELECTED_PEERS="5"
    REDUCE_RELAY_TX_ENABLE="0"
    REDUCE_RELAY_TX_MIN_PEERS="20"
    REDUCE_RELAY_TX_RELAY_PERCENTAGE="25"
    LOG_FILE="$HOME/.local/share/quaxar/quaxar.log"
    LOG_LEVEL="info"

    if [ "$AUTO_YES" = false ]; then
        echo -e "  ${DIM}Configure the essential node settings (press Enter for defaults):${RESET}"
        echo ""
        ask_choice "Network" "$NETWORK" NETWORK "mainnet testnet devnet"
        ask "Data directory" "$DATA_DIR" DATA_DIR
        DB_PATH="$DATA_DIR/db/nudb"
        SQLITE_PATH="$DATA_DIR/db"
        ask_choice "Node size" "$NODE_SIZE" NODE_SIZE "tiny small medium large huge"
        ask_ledger_history "Ledger history" "$LEDGER_HISTORY" LEDGER_HISTORY
        ensure_history_not_above_online_delete
    fi

    # The installer intentionally emits the documented minimal configuration.
    # Keep the old exhaustive prompt flow available only for explicit debugging.
    if [ "$AUTO_YES" = false ] && [ "${QUAXAR_ADVANCED_CONFIG:-0}" = "1" ]; then
        echo -e "  ${DIM}Configure advanced runtime settings (press Enter for defaults):${RESET}"
        echo ""

        echo -e "  ${BOLD}── Ports ──${RESET}"
        ask_port "RPC port" "$RPC_PORT" RPC_PORT
        ask "RPC bind IP" "$RPC_IP" RPC_IP
        ask "RPC admin networks" "$RPC_ADMIN" RPC_ADMIN
        ask "RPC secure gateway networks (blank to disable)" "$RPC_SECURE_GATEWAY" RPC_SECURE_GATEWAY
        ask_port "Peer port" "$PEER_PORT" PEER_PORT
        ask "Peer bind IP" "$PEER_IP" PEER_IP
        ask_port "WebSocket port" "$WS_PORT" WS_PORT
        ask "WebSocket bind IP" "$WS_IP" WS_IP
        ask "WebSocket admin networks" "$WS_ADMIN" WS_ADMIN
        ask "WebSocket secure gateway networks (blank to disable)" "$WS_SECURE_GATEWAY" WS_SECURE_GATEWAY
        ask_int_range "WebSocket send queue limit" "$WS_SEND_QUEUE_LIMIT" WS_SEND_QUEUE_LIMIT 1 100000

        echo ""
        echo -e "  ${BOLD}── Database ──${RESET}"
        ask_choice "Database type" "$DB_TYPE" DB_TYPE "nudb rocksdb"
        case "$DB_TYPE" in
            nudb) DB_TYPE="NuDB" ;;
            rocksdb) DB_TYPE="RocksDB" ;;
        esac
        ask "Data directory" "$DATA_DIR" DATA_DIR
        DB_PATH="$DATA_DIR/db/nudb"
        SQLITE_PATH="$DATA_DIR/db"
        ask "Node DB path" "$DB_PATH" DB_PATH
        ask "Relational database path" "$SQLITE_PATH" SQLITE_PATH
        ask_choice "NuDB block size" "$NUDB_BLOCK_SIZE" NUDB_BLOCK_SIZE "4096 8192 16384 32768"
        ask_int_range "Online delete (ledgers, 0 disables)" "$ONLINE_DELETE" ONLINE_DELETE 0 100000000
        ask_bool_value "Advisory delete" "$ADVISORY_DELETE" ADVISORY_DELETE

        echo ""
        echo -e "  ${BOLD}── Node ──${RESET}"
        info "Node size determines memory usage:"
        info "  tiny=4GB  small=8GB  medium=16GB  large=32GB  huge=64GB"
        ask_choice "Node size" "$NODE_SIZE" NODE_SIZE "tiny small medium large huge"
        ask_ledger_history "Ledger history" "$LEDGER_HISTORY" LEDGER_HISTORY
        ensure_history_not_above_online_delete

        echo ""
        echo -e "  ${BOLD}── Network ──${RESET}"
        ask_choice "Network" "$NETWORK" NETWORK "mainnet testnet devnet"
        ask_network_id "Network ID (blank to omit, or main/testnet/devnet/number)" "$NETWORK_ID" NETWORK_ID
        ask_bool_value "SSL verify validator list HTTPS" "$SSL_VERIFY" SSL_VERIFY

        echo ""
        echo -e "  ${BOLD}── Overlay ──${RESET}"
        ask "Advertised public IP (blank to auto-detect/omit)" "$OVERLAY_PUBLIC_IP" OVERLAY_PUBLIC_IP
        ask_int_range "Peer IP limit (0 uses default)" "$OVERLAY_IP_LIMIT" OVERLAY_IP_LIMIT 0 100000
        ask_bool_value "Verify advertised endpoints" "$OVERLAY_VERIFY_ENDPOINTS" OVERLAY_VERIFY_ENDPOINTS
        ask_bool_value "Crawl enabled" "$CRAWL_ENABLED" CRAWL_ENABLED
        ask_bool_value "Crawl overlay peers" "$CRAWL_OVERLAY" CRAWL_OVERLAY
        ask_bool_value "Crawl server info" "$CRAWL_SERVER" CRAWL_SERVER
        ask_bool_value "Crawl counts" "$CRAWL_COUNTS" CRAWL_COUNTS
        ask_bool_value "Crawl UNL" "$CRAWL_UNL" CRAWL_UNL
        ask_bool_value "Validator list fetching enabled" "$VL_ENABLED" VL_ENABLED

        echo ""
        echo -e "  ${BOLD}── Relay Reduction ──${RESET}"
        ask_bool_value "Validation relay base squelch enabled" "$REDUCE_RELAY_VP_ENABLE" REDUCE_RELAY_VP_ENABLE
        ask_int_range "Validation relay max selected peers" "$REDUCE_RELAY_VP_MAX_SELECTED_PEERS" REDUCE_RELAY_VP_MAX_SELECTED_PEERS 3 10000
        ask_bool_value "Transaction reduce relay enabled" "$REDUCE_RELAY_TX_ENABLE" REDUCE_RELAY_TX_ENABLE
        ask_int_range "Transaction reduce relay min peers" "$REDUCE_RELAY_TX_MIN_PEERS" REDUCE_RELAY_TX_MIN_PEERS 10 100000
        ask_int_range "Transaction relay percentage" "$REDUCE_RELAY_TX_RELAY_PERCENTAGE" REDUCE_RELAY_TX_RELAY_PERCENTAGE 10 100

        echo ""
        echo -e "  ${BOLD}── Logging ──${RESET}"
        ask "Log file" "$LOG_FILE" LOG_FILE
        ask_choice "Log level" "$LOG_LEVEL" LOG_LEVEL "error warn info debug trace"
    fi

    ensure_history_not_above_online_delete
    validate_config_inputs

    # Network-specific settings
    case "$NETWORK" in
        mainnet)
            VL_SITE="https://vl.ripple.com"
            VL_KEY="ED2677ABFFD1B33AC6FBC3062B71F1E8397C1505E1C42C64D11AD1B28FF73F4734"
            PEERS="s1.ripple.com 51235,s2.ripple.com 51235"
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
        *)
            fail "Unsupported network: $NETWORK"
            exit 1
            ;;
    esac

    if [ -z "$NETWORK_ID" ] && [ "$NETWORK" != "mainnet" ]; then
        NETWORK_ID="$NETWORK"
    fi

    if [ "$AUTO_YES" = false ] && [ "${QUAXAR_ADVANCED_CONFIG:-0}" = "1" ]; then
        echo ""
        echo -e "  ${BOLD}── Validators and Peers ──${RESET}"
        ask "Validators file" "$VALIDATORS_FILE" VALIDATORS_FILE
        ask "Validator list sites (comma-separated)" "$VL_SITE" VL_SITE
        ask "Validator list keys (comma-separated)" "$VL_KEY" VL_KEY
        ask "Fixed peers (comma-separated host port entries)" "$PEERS" PEERS
    fi

    RPC_SECURE_GATEWAY_LINE=""
    [ -n "$RPC_SECURE_GATEWAY" ] && RPC_SECURE_GATEWAY_LINE="secure_gateway = $RPC_SECURE_GATEWAY"
    WS_SECURE_GATEWAY_LINE=""
    [ -n "$WS_SECURE_GATEWAY" ] && WS_SECURE_GATEWAY_LINE="secure_gateway = $WS_SECURE_GATEWAY"
    NETWORK_ID_SECTION=""
    if [ -n "$NETWORK_ID" ]; then
        NETWORK_ID_SECTION="[network_id]
$NETWORK_ID
"
    fi
    OVERLAY_PUBLIC_IP_LINE=""
    [ -n "$OVERLAY_PUBLIC_IP" ] && OVERLAY_PUBLIC_IP_LINE="public_ip = $OVERLAY_PUBLIC_IP"

    # Create directories
    mkdir -p "$CONF_DIR" "$DB_PATH" "$SQLITE_PATH" "$(dirname "$LOG_FILE")" "$(dirname "$VALIDATORS_FILE")"
    chown -R "$(whoami)" "$DATA_DIR" "$(dirname "$LOG_FILE")"

    # Write xrpld.cfg
    tee "$CONF_FILE" > /dev/null << EOF
[server]
port_rpc_admin_local
port_peer

[port_rpc_admin_local]
port = $RPC_PORT
ip = 127.0.0.1
admin = 127.0.0.1
protocol = http,ws

[port_peer]
port = $PEER_PORT
ip = $PEER_IP
protocol = peer

[node_size]
$NODE_SIZE

[node_db]
type = $DB_TYPE
path = $DB_PATH
nudb_block_size = $NUDB_BLOCK_SIZE
online_delete = $ONLINE_DELETE
advisory_delete = $ADVISORY_DELETE

[database_path]
$SQLITE_PATH

[ledger_history]
$LEDGER_HISTORY

[validators_file]
$VALIDATORS_FILE

$NETWORK_ID_SECTION
[ips]
$(as_lines "$PEERS")
EOF

    # Write validators.txt
    tee "$VALIDATORS_FILE" > /dev/null << EOF
[validator_list_sites]
$(as_lines "$VL_SITE")

[validator_list_keys]
$(as_lines "$VL_KEY")
EOF

    ok "Config written to $CONF_FILE"
    ok "Validators written to $VALIDATORS_FILE"
fi

# ── Systemd Service ──────────────────────────────────────────────────────────
header "Service Setup"

INSTALL_SERVICE=true
if [ "$AUTO_YES" = false ]; then
    ask_yn "Install systemd service?" "Y" || INSTALL_SERVICE=false
fi

if [ "$INSTALL_SERVICE" = true ] && command -v systemctl &>/dev/null && sudo -n true 2>/dev/null; then
    XRPLD_BIN=$(which quaxar 2>/dev/null || echo "$HOME/.cargo/bin/quaxar")

    sudo tee /etc/systemd/system/quaxar.service > /dev/null << EOF
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
    sudo systemctl enable quaxar

    ok "Service installed (quaxar.service)"

    if ask_yn "Start quaxar now?" "Y"; then
        sudo systemctl start quaxar
        sleep 3
        ok "quaxar started"
    fi
elif [ "$INSTALL_SERVICE" = true ]; then
    warn "systemd not available — skipping service setup"
    info "Start manually: RUST_LOG=$LOG_LEVEL quaxar --conf $CONF_FILE"
fi

# ── Verification ─────────────────────────────────────────────────────────────
header "Verification"

XRPLD_BIN=$(which quaxar 2>/dev/null || echo "$HOME/.cargo/bin/quaxar")

if [ -x "$XRPLD_BIN" ]; then
    VER=$("$XRPLD_BIN" version 2>/dev/null | grep -i version | head -1 || echo "installed")
    ok "Binary: $XRPLD_BIN"
    ok "$VER"
else
    fail "Binary not found"
fi

if systemctl is-active quaxar &>/dev/null 2>&1; then
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
echo -e "    quaxar status          ${DIM}Check node status${RESET}"
echo -e "    quaxar cli             ${DIM}Interactive mode${RESET}"
echo -e "    quaxar peers           ${DIM}View connected peers${RESET}"
echo -e "    journalctl -u quaxar   ${DIM}Follow logs${RESET}"
echo ""
