#!/usr/bin/env bash
# Rendered and launched by provision-testnet.sh. Do not run without setting
# QUAXAR_REF and PUBLIC_IP.
set -Eeuo pipefail

QUAXAR_REF="__QUAXAR_REF__"
PUBLIC_IP="__PUBLIC_IP__"
QUAXAR_REPOSITORY="https://github.com/TusharPardhe/quaxar.git"
SKIP_BUILD="${SKIP_BUILD:-0}"

if [[ ! "$QUAXAR_REF" =~ ^[A-Za-z0-9._/-]+$ ]]; then
  echo "Invalid Quaxar branch or tag: $QUAXAR_REF" >&2
  exit 64
fi
if [[ ! "$PUBLIC_IP" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]]; then
  echo "Invalid public IPv4 address: $PUBLIC_IP" >&2
  exit 64
fi
if [[ "$SKIP_BUILD" != "0" && "$SKIP_BUILD" != "1" ]]; then
  echo "SKIP_BUILD must be 0 or 1" >&2
  exit 64
fi

exec > >(tee -a /var/log/quaxar-bootstrap.log) 2>&1
echo "[$(date -Is)] starting Quaxar testnet bootstrap"

retry() {
  local attempt=1
  local max_attempts=8
  until "$@"; do
    if (( attempt == max_attempts )); then
      echo "Command failed after ${attempt} attempts: $*" >&2
      return 1
    fi
    sleep $((attempt * 5))
    ((attempt += 1))
  done
}

export DEBIAN_FRONTEND=noninteractive
retry apt-get update
retry apt-get install -y --no-install-recommends \
  ca-certificates curl git build-essential pkg-config libssl-dev librocksdb-dev \
  clang cmake lld

if ! id -u xrpld >/dev/null 2>&1; then
  useradd --system --create-home --home-dir /home/xrpld --shell /usr/sbin/nologin xrpld
fi
install -d -m 0750 /var/lib/xrpld /var/lib/xrpld/db /var/lib/xrpld/db/nudb /var/log/xrpld
chown -R xrpld:xrpld /var/lib/xrpld /var/log/xrpld
if [[ "$SKIP_BUILD" == "1" ]]; then
  test -d /opt/quaxar/.git
  test -x /usr/local/bin/quaxar
  echo "[$(date -Is)] reusing existing Quaxar build"
else
  rm -rf /opt/quaxar
  install -d -o xrpld -g xrpld -m 0755 /opt
  runuser -u xrpld -- git clone --depth 1 --branch "$QUAXAR_REF" --single-branch \
    "$QUAXAR_REPOSITORY" /opt/quaxar

  runuser -u xrpld -- bash -lc '
    set -Eeuo pipefail
    curl --fail --silent --show-error --proto "=https" --tlsv1.2 https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain 1.90.0
    export PATH="$HOME/.cargo/bin:$PATH"
    rustup toolchain install 1.90.0 --profile minimal
    cd /opt/quaxar
    ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu CARGO_BUILD_JOBS=2 CC=clang CXX=clang++ \
      cargo +1.90.0 build --release -p xrpld-main
  '
  install -m 0755 /opt/quaxar/target/release/quaxar /usr/local/bin/quaxar
fi
install -d -m 0755 /etc/quaxar
test -r /opt/quaxar/infra/aws/testnet-amendments.txt
install -m 0644 /dev/stdin /etc/quaxar-validators-testnet.txt <<'VALIDATORS'
[validator_list_sites]
https://vl.altnet.rippletest.net

[validator_list_keys]
ED264807102805220DA0F312E71FC2C69E1552C9C5790F6C25E3729DEB573D5860
VALIDATORS

cat >/etc/quaxar/xrpld.cfg <<CONFIG
[server]
port_rpc_admin_local
port_ws_admin_local
port_peer

[port_rpc_admin_local]
port = 5005
ip = 127.0.0.1
admin = 127.0.0.1
protocol = http

[port_ws_admin_local]
port = 6006
ip = 127.0.0.1
admin = 127.0.0.1
protocol = ws
send_queue_limit = 500

[port_peer]
port = 51235
ip = 0.0.0.0
protocol = peer

[node_size]
medium

[node_db]
type = NuDB
path = /var/lib/xrpld/db/nudb
online_delete = 256
advisory_delete = 0

[database_path]
/var/lib/xrpld/db

[ledger_history]
256

[debug_logfile]
/var/log/xrpld/debug.log

[network_id]
1

[features]
$(grep -E '^[A-Za-z][A-Za-z0-9_]*$' /opt/quaxar/infra/aws/testnet-amendments.txt)

[overlay]
public_ip = $PUBLIC_IP
verify_endpoints = 1

[ips]
s.altnet.rippletest.net 51235

[validators_file]
/etc/quaxar-validators-testnet.txt
CONFIG

cat >/etc/systemd/system/quaxar.service <<'UNIT'
[Unit]
Description=Quaxar XRP Ledger testnet node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=xrpld
Group=xrpld
ExecStart=/usr/local/bin/quaxar --conf /etc/quaxar/xrpld.cfg
Restart=on-failure
RestartSec=10
LimitNOFILE=65536
Environment=RUST_LOG=info
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=full
ReadWritePaths=/var/lib/xrpld /var/log/xrpld

[Install]
WantedBy=multi-user.target
UNIT

cat >/etc/logrotate.d/quaxar <<'ROTATE'
/var/log/xrpld/*.log /var/log/quaxar-bootstrap.log {
    rotate 7
    daily
    missingok
    notifempty
    compress
    delaycompress
    copytruncate
}
ROTATE

{
  echo "repository=$QUAXAR_REPOSITORY"
  echo "requested_ref=$QUAXAR_REF"
  echo "commit=$(runuser -u xrpld -- git -C /opt/quaxar rev-parse HEAD)"
  echo "built_at=$(date -Is)"
} >/etc/quaxar/build-info

systemctl daemon-reload
systemctl enable quaxar.service
systemctl restart quaxar.service
echo "[$(date -Is)] Quaxar bootstrap completed"
