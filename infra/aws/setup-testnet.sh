#!/usr/bin/env bash
# Rendered and launched by provision-testnet.sh. Do not run without setting
# QUAXAR_REF and PUBLIC_IP.
set -Eeuo pipefail

QUAXAR_REF="__QUAXAR_REF__"
PUBLIC_IP="__PUBLIC_IP__"
QUAXAR_REPOSITORY="https://github.com/TusharPardhe/quaxar.git"
SKIP_BUILD="${SKIP_BUILD:-0}"
RPC_READY_TIMEOUT_SECONDS="${RPC_READY_TIMEOUT_SECONDS:-180}"

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
if [[ ! "$RPC_READY_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || (( RPC_READY_TIMEOUT_SECONDS > 900 )); then
  echo "RPC_READY_TIMEOUT_SECONDS must be between 1 and 900" >&2
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

# This provisioner owns only fresh hosts and its canonical /var/lib/quaxar
# layout. Refuse legacy in-place state before changing services, ownership, or
# validator configuration; use docs/RUNNING.md for a deliberate host cutover.
XRPLD_LOAD_STATE="$(systemctl show -p LoadState --value xrpld.service 2>/dev/null || true)"
if systemctl is-active --quiet xrpld.service \
  || [[ -n "$XRPLD_LOAD_STATE" && "$XRPLD_LOAD_STATE" != "not-found" ]] \
  || [[ -e /var/lib/xrpld ]] \
  || [[ -e /var/log/xrpld ]] \
  || [[ -e /etc/quaxar/xrpld.cfg ]] \
  || [[ -e /etc/xrpld/xrpld.cfg ]]; then
  echo "Legacy xrpld state detected; automatic in-place migration is intentionally refused" >&2
  echo "Follow docs/RUNNING.md and verify database, config, validator identity, service user, and rollback paths" >&2
  exit 1
fi

# A custom deployment may already use the Quaxar service name while retaining
# a legacy account or /srv layout. Accept only this provisioner's canonical
# unit and config; otherwise refuse before replacing binaries or ownership.
QUAXAR_LOAD_STATE="$(systemctl show -p LoadState --value quaxar.service 2>/dev/null || true)"
if [[ -n "$QUAXAR_LOAD_STATE" && "$QUAXAR_LOAD_STATE" != "not-found" ]]; then
  QUAXAR_UNIT_USER="$(systemctl show -p User --value quaxar.service)"
  QUAXAR_UNIT_GROUP="$(systemctl show -p Group --value quaxar.service)"
  QUAXAR_UNIT_EXEC="$(systemctl show -p ExecStart --value quaxar.service)"
  if [[ "$QUAXAR_UNIT_USER" != "quaxar" ]] \
    || [[ "$QUAXAR_UNIT_GROUP" != "quaxar" ]] \
    || [[ "$QUAXAR_UNIT_EXEC" != *"/usr/local/bin/quaxar --conf /etc/quaxar/quaxar.cfg"* ]]; then
    echo "Existing quaxar.service is not owned by the canonical AWS provisioner layout" >&2
    echo "Refusing to rewrite a custom service; follow docs/RUNNING.md" >&2
    exit 1
  fi
fi
if [[ -e /srv/quaxar ]]; then
  echo "Custom /srv/quaxar layout detected; refusing canonical AWS provisioning" >&2
  exit 1
fi
if [[ -e /etc/quaxar/quaxar.cfg ]]; then
  if ! grep -Fxq 'path = /var/lib/quaxar/db/nudb' /etc/quaxar/quaxar.cfg \
    || ! grep -Fxq '/var/lib/quaxar/db' /etc/quaxar/quaxar.cfg \
    || grep -Eq '/srv/|/var/lib/xrpld|/var/log/xrpld' /etc/quaxar/quaxar.cfg; then
    echo "Existing quaxar.cfg uses a noncanonical data layout; refusing to rewrite its service" >&2
    exit 1
  fi
fi

if ! id -u quaxar >/dev/null 2>&1; then
  useradd --system --create-home --home-dir /home/quaxar --shell /usr/sbin/nologin quaxar
fi
install -d -o quaxar -g quaxar -m 0750 \
  /var/lib/quaxar /var/lib/quaxar/db /var/lib/quaxar/db/nudb /var/log/quaxar
if [[ "$SKIP_BUILD" == "1" ]]; then
  test -d /opt/quaxar/.git
  test -x /usr/local/bin/quaxar
  echo "[$(date -Is)] reusing existing Quaxar build"
else
  rm -rf /opt/quaxar
  install -d -o quaxar -g quaxar -m 0755 /opt/quaxar
  runuser -u quaxar -- git clone --depth 1 --branch "$QUAXAR_REF" --single-branch \
    "$QUAXAR_REPOSITORY" /opt/quaxar

  runuser -u quaxar -- bash -lc '
    set -Eeuo pipefail
    curl --fail --silent --show-error --proto "=https" --tlsv1.2 https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain 1.90.0
    export PATH="$HOME/.cargo/bin:$PATH"
    rustup toolchain install 1.90.0 --profile minimal
    cd /opt/quaxar
    ROCKSDB_LIB_DIR=/usr/lib/x86_64-linux-gnu CARGO_BUILD_JOBS=2 CC=clang CXX=clang++ \
      cargo +1.90.0 build --release -p quaxar-main
  '
  install -m 0755 /opt/quaxar/target/release/quaxar /usr/local/bin/quaxar
fi
install -d -m 0755 /etc/quaxar
test -r /opt/quaxar/infra/aws/testnet-amendments.txt

if [[ ! -e /etc/quaxar/quaxar.cfg ]]; then
  cat >/etc/quaxar/quaxar.cfg <<CONFIG
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

[port_peer]
port = 51235
ip = 0.0.0.0
protocol = peer

[node_size]
medium

[node_db]
type = NuDB
path = /var/lib/quaxar/db/nudb
online_delete = 256
advisory_delete = 0

[database_path]
/var/lib/quaxar/db

[ledger_history]
256

[network_id]
1

[features]
$(grep -E '^[A-Za-z][A-Za-z0-9_]*$' /opt/quaxar/infra/aws/testnet-amendments.txt)

[overlay]
public_ip = $PUBLIC_IP
verify_endpoints = 1

[ips]
s.altnet.rippletest.net 51235

[validator_list_sites]
https://vl.altnet.rippletest.net

[validator_list_keys]
ED264807102805220DA0F312E71FC2C69E1552C9C5790F6C25E3729DEB573D5860
CONFIG
fi
chown root:quaxar /etc/quaxar/quaxar.cfg
chmod 0640 /etc/quaxar/quaxar.cfg
CONFIG_CHECK_OUTPUT="$(/usr/local/bin/quaxar --conf /etc/quaxar/quaxar.cfg config 2>&1)"
echo "$CONFIG_CHECK_OUTPUT"
if ! grep -q 'Config looks good' <<<"$CONFIG_CHECK_OUTPUT"; then
  echo "Generated Quaxar config failed validation" >&2
  exit 1
fi

cat >/etc/systemd/system/quaxar.service <<'UNIT'
[Unit]
Description=Quaxar XRP Ledger testnet node
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
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=full
ReadWritePaths=/var/lib/quaxar /var/log/quaxar

[Install]
WantedBy=multi-user.target
UNIT

cat >/etc/logrotate.d/quaxar <<'ROTATE'
/var/log/quaxar-bootstrap.log {
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
  echo "commit=$(runuser -u quaxar -- git -C /opt/quaxar rev-parse HEAD)"
  echo "built_at=$(date -Is)"
} >/etc/quaxar/build-info

systemctl daemon-reload
systemctl restart quaxar.service
RPC_READY=0
RPC_READY_ATTEMPTS=$(( (RPC_READY_TIMEOUT_SECONDS + 1) / 2 ))
for _ in $(seq 1 "$RPC_READY_ATTEMPTS"); do
  if systemctl is-active --quiet quaxar.service \
    && curl -fsS --max-time 2 \
      -H 'Content-Type: application/json' \
      -d '{"method":"server_info","params":[{}]}' \
      http://127.0.0.1:5005 \
      | grep -q '"status"[[:space:]]*:[[:space:]]*"success"'; then
    RPC_READY=1
    break
  fi
  sleep 2
done
QUAXAR_PID="$(systemctl show -p MainPID --value quaxar.service)"
if [[ "$RPC_READY" != "1" ]] || [[ ! "$QUAXAR_PID" =~ ^[1-9][0-9]*$ ]]; then
  systemctl --no-pager --full status quaxar.service || true
  journalctl -u quaxar.service -n 100 --no-pager || true
  systemctl stop quaxar.service || true
  echo "Quaxar service failed RPC readiness validation" >&2
  exit 1
fi
systemctl enable quaxar.service
echo "[$(date -Is)] Quaxar bootstrap completed"
