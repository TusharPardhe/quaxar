#!/bin/bash
# First boot of a throughput-test host (docs/perf/2026-11-tps-test-plan.md).
# Rendered by tps.py: __RUN_PUBKEY__ and __LIFETIME_MIN__ are replaced.
#
# - Schedules its own shutdown; the instance terminates on shutdown, so
#   nothing outlives the run.
# - Puts /data on the instance-store NVMe when there is one.
# - Raises file and socket limits for many WebSocket connections.
# - Trusts the run's own SSH key, so hosts can copy to each other over the
#   private network.
set -euxo pipefail
exec >/var/log/tps-boot.log 2>&1

shutdown -h +__LIFETIME_MIN__ "throughput test lifetime reached"

DISK=$(lsblk -dpno NAME,MODEL | awk '/Instance Storage/{print $1; exit}')
mkdir -p /data
if [ -n "$DISK" ]; then
  mkfs.ext4 -q -F -L tpsdata "$DISK"
  mount -o noatime "$DISK" /data
fi
chown ubuntu:ubuntu /data

cat >/etc/security/limits.d/90-tps.conf <<'EOF'
* soft nofile 1048576
* hard nofile 1048576
EOF
cat >/etc/sysctl.d/90-tps.conf <<'EOF'
fs.file-max = 2097152
net.core.somaxconn = 4096
net.ipv4.ip_local_port_range = 10240 65000
net.ipv4.tcp_tw_reuse = 1
EOF
sysctl --system >/dev/null

echo "__RUN_PUBKEY__" >>/home/ubuntu/.ssh/authorized_keys
touch /var/lib/tps-boot-done
