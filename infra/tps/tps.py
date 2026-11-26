#!/usr/bin/env python3
"""Private quaxar network for the throughput test
(docs/perf/2026-11-tps-test-plan.md).

Local network (gate G2), all nodes as processes on this machine:

  tps.py local-init  --dir D --quaxar BIN [--validators 5] [--p2p 2]
  tps.py local-start --dir D --mode start|load
  tps.py local-stop  --dir D
  tps.py local-status --dir D
  tps.py local-fork  --dir D --fork BIN --account rADDR --regular-key rADDR

Docker network (one container per node, each with its own IP):

  tps.py docker-init --dir D --bin-dir DIR --keys-quaxar BIN
  tps.py docker-start --dir D --mode start|load
  tps.py docker-fork / docker-status / docker-stop --dir D

AWS network (gates G4, G5 and the run), in order:

  aws-check, aws-up, aws-keys, aws-bundle, aws-sync, aws-fork, aws-config,
  aws-distribute, aws-isolate, aws-start --mode load, aws-setup,
  aws-baseline --action save, aws-run, aws-down

Standard library only.
"""

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

NETWORK_ID = 1000

# Mainnet validator settings kept as they are; see the plan, section 2.
VOTING = """[voting]
reference_fee = 10
account_reserve = 1000000
owner_reserve = 200000
"""


# RippleX 2022 values that switch fee escalation off. Used only while the
# test accounts and market are set up; measured runs use the default queue.
SETUP_TXQ = """[transaction_queue]
minimum_txn_in_ledger = 4294967295
target_txn_in_ledger = 4294967295
normal_consensus_increase_percent = 4294967295
"""


def render_config(node, nodes, data_dir, bind_ip, log_level="warning", setup_txq=False):
    """quaxar.cfg for `node` in `nodes`.

    node/nodes entries: dict(name, role, peer_ip, peer_port, rpc_port, ws_port,
    public_key, seed?). Only these differ from the mainnet validator's config:
    network_id, peers ([ips_fixed] only, peer_private), [validators],
    node_size huge (no explicit cache_mb), RPC/WS reachable inside the
    private network, and the WebSocket send queue for pipelined submits.
    """
    fixed = "\n".join(f"{n['peer_ip']} {n['peer_port']}" for n in nodes if n["name"] != node["name"])
    validators = "\n".join(n["public_key"] for n in nodes if n["role"] == "validator")
    seed = f"[validation_seed]\n{node['seed']}\n" if node["role"] == "validator" else ""
    return f"""[server]
port_rpc
port_ws
port_peer

[port_rpc]
ip = {bind_ip}
port = {node['rpc_port']}
protocol = http
admin = 127.0.0.1

[port_ws]
ip = {bind_ip}
port = {node['ws_port']}
protocol = ws
admin = 127.0.0.1
send_queue_limit = 65535

[port_peer]
ip = 0.0.0.0
port = {node['peer_port']}
protocol = peer

[network_id]
{NETWORK_ID}

[node_size]
huge

[node_db]
type = fjall
path = {data_dir}/db/fjall
online_delete = 512
advisory_delete = 0
reserve_mb = 1024
compaction_base_mb = 2048

[database_path]
{data_dir}/db

[ledger_history]
256

[ips_fixed]
{fixed}

[peer_private]
1

[validators]
{validators}

{seed}{VOTING}
{SETUP_TXQ if setup_txq else ""}
[debug_logfile]
{data_dir}/debug.log

[rpc_startup]
{{ "command": "log_level", "severity": "{log_level}" }}

[ssl_verify]
0
"""


def generate_validator_keys(quaxar, count, workdir):
    """`quaxar validator-keys generate` once per validator."""
    keys = []
    for i in range(count):
        d = Path(workdir) / f"keys-{i}"
        if d.exists():
            shutil.rmtree(d)
        d.mkdir(parents=True, mode=0o700)
        subprocess.run([quaxar, "validator-keys", "generate"], cwd=d, check=True,
                       stdout=subprocess.DEVNULL)
        k = json.loads((d / "validator-keys.json").read_text())
        keys.append({"public_key": k["validation_public_key"], "seed": k["validation_seed"]})
        shutil.rmtree(d)
    return keys


def rpc(url, method, params=None, timeout=5):
    body = json.dumps({"method": method, "params": [params or {}]}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)["result"]


def node_status(url):
    try:
        info = rpc(url, "server_info")["info"]
        v = info.get("validated_ledger", {})
        return {"state": info.get("server_state"), "validated": v.get("seq"),
                "hash": v.get("hash"), "peers": info.get("peers")}
    except Exception as e:  # noqa: BLE001 - status is best-effort
        return {"state": f"down ({e.__class__.__name__})"}


# --- local network -----------------------------------------------------------


def local_nodes(state):
    return state["nodes"]


def cmd_local_init(a):
    d = Path(a.dir)
    if d.exists():
        sys.exit(f"{d} exists; remove it first")
    d.mkdir(parents=True)
    keys = generate_validator_keys(a.quaxar, a.validators, d)
    nodes = []
    for i in range(a.validators + a.p2p):
        role = "validator" if i < a.validators else "p2p"
        n = {"name": f"{'v' if role == 'validator' else 'p'}{i}", "role": role,
             "peer_ip": "127.0.0.1", "peer_port": 52000 + i, "rpc_port": 53000 + i, "ws_port": 54000 + i}
        if role == "validator":
            n.update(keys[i])
        nodes.append(n)
    for n in nodes:
        nd = d / n["name"]
        (nd / "db").mkdir(parents=True)
        (nd / "quaxar.cfg").write_text(render_config(n, nodes, nd, "127.0.0.1"))
        os.chmod(nd / "quaxar.cfg", 0o600)
    (d / "state.json").write_text(json.dumps({"quaxar": a.quaxar, "nodes": nodes}, indent=1))
    os.chmod(d / "state.json", 0o600)
    print(f"{len(nodes)} nodes in {d}")


def load_state(d):
    return json.loads((Path(d) / "state.json").read_text())


def cmd_local_start(a):
    st = load_state(a.dir)
    for n in st["nodes"]:
        nd = Path(a.dir) / n["name"]
        args = [st["quaxar"], "--conf", str(nd / "quaxar.cfg")]
        args += ["--start", "--valid"] if a.mode == "start" else ["--load", "--valid"]
        log = open(nd / "stdout.log", "ab")
        p = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        (nd / "pid").write_text(str(p.pid))
        print(f"{n['name']} pid {p.pid}")


def cmd_local_stop(a):
    st = load_state(a.dir)
    pids = []
    for n in st["nodes"]:
        f = Path(a.dir) / n["name"] / "pid"
        if f.exists():
            pid = int(f.read_text())
            try:
                os.kill(pid, signal.SIGTERM)
                pids.append(pid)
            except ProcessLookupError:
                pass
            f.unlink()
    deadline = time.time() + 60
    for pid in pids:
        while time.time() < deadline:
            try:
                os.kill(pid, 0)
                time.sleep(0.5)
            except ProcessLookupError:
                break
        else:
            os.kill(pid, signal.SIGKILL)
    print(f"stopped {len(pids)}")


def cmd_local_status(a):
    st = load_state(a.dir)
    for n in st["nodes"]:
        print(n["name"], json.dumps(node_status(f"http://127.0.0.1:{n['rpc_port']}/")))


def cmd_local_fork(a):
    """Fork the first validator's newest ledger and copy it to every node."""
    st = load_state(a.dir)
    src = Path(a.dir) / st["nodes"][0]["name"] / "db"
    subprocess.run([a.fork, str(src / "fjall"), str(src), a.account, a.regular_key], check=True)
    for n in st["nodes"][1:]:
        dst = Path(a.dir) / n["name"] / "db"
        shutil.rmtree(dst)
        shutil.copytree(src, dst, ignore=shutil.ignore_patterns("*.db-shm", "*.db-wal", "peerfinder.db"))
    print(f"forked store copied to {len(st['nodes']) - 1} nodes")


# --- Docker network: one container per node, each with its own IP ----------

DOCKER_SUBNET = "172.30.0"


def cmd_docker_init(a):
    d = Path(a.dir)
    if d.exists():
        sys.exit(f"{d} exists; remove it first")
    d.mkdir(parents=True)
    keys = generate_validator_keys(a.keys_quaxar, a.validators, d)
    nodes = []
    for i in range(a.validators + a.p2p):
        role = "validator" if i < a.validators else "p2p"
        n = {"name": f"{'v' if role == 'validator' else 'p'}{i}", "role": role,
             "peer_ip": f"{DOCKER_SUBNET}.{10 + i}", "peer_port": 51235, "rpc_port": 5005, "ws_port": 6006,
             "host_rpc": 53000 + i, "host_ws": 54000 + i}
        if role == "validator":
            n.update(keys[i])
        nodes.append(n)
    services = []
    for n in nodes:
        nd = d / n["name"]
        (nd / "db").mkdir(parents=True)
        (nd / "quaxar.cfg").write_text(render_config(n, nodes, "/data", "0.0.0.0"))
        services.append(f"""  {n['name']}:
    image: debian:bookworm-slim
    platform: linux/amd64
    command: ["/bin/quaxar", "--conf", "/data/quaxar.cfg", ${{MODE_ARGS}}]
    volumes: ["./{n['name']}:/data", "{Path(a.bin_dir).resolve()}:/bin-q:ro"]
    entrypoint: ["/bin/sh", "-c", "cp /bin-q/quaxar /bin/quaxar && exec \\"$$0\\" \\"$$@\\""]
    networks: {{tps: {{ipv4_address: {n['peer_ip']}}}}}
    ports: ["127.0.0.1:{n['host_rpc']}:5005", "127.0.0.1:{n['host_ws']}:6006"]
    stop_grace_period: 60s
""")
    compose = "services:\n" + "".join(services) + f"""networks:
  tps:
    ipam:
      config: [{{subnet: {DOCKER_SUBNET}.0/24}}]
"""
    (d / "docker-compose.yml").write_text(compose)
    (d / "state.json").write_text(json.dumps({"nodes": nodes}, indent=1))
    os.chmod(d / "state.json", 0o600)
    print(f"{len(nodes)} nodes in {d}")


def cmd_docker_start(a):
    mode = '"--start", "--valid"' if a.mode == "start" else '"--load", "--valid"'
    # MODE_ARGS is substituted into the command list.
    text = (Path(a.dir) / "docker-compose.yml").read_text()
    (Path(a.dir) / "docker-compose.run.yml").write_text(text.replace("${MODE_ARGS}", mode))
    subprocess.run(["docker", "compose", "-p", "tpsnet", "-f", str(Path(a.dir) / "docker-compose.run.yml"),
                    "up", "-d"], check=True)


def cmd_docker_stop(a):
    subprocess.run(["docker", "compose", "-p", "tpsnet", "-f", str(Path(a.dir) / "docker-compose.run.yml"),
                    "down", "-t", "60"], check=True)


def cmd_docker_config(a):
    """Re-render every node's config, with or without the setup queue."""
    st = load_state(a.dir)
    for n in st["nodes"]:
        (Path(a.dir) / n["name"] / "quaxar.cfg").write_text(
            render_config(n, st["nodes"], "/data", "0.0.0.0", log_level=a.log_level, setup_txq=a.setup_txq))
    print("configs written", "with setup queue" if a.setup_txq else "")


def cmd_docker_status(a):
    for n in load_state(a.dir)["nodes"]:
        print(n["name"], json.dumps(node_status(f"http://127.0.0.1:{n['host_rpc']}/")))


def cmd_docker_fork(a):
    """Fork the first validator's newest ledger and copy it to every node."""
    st = load_state(a.dir)
    src = Path(a.dir) / st["nodes"][0]["name"] / "db"
    subprocess.run([a.fork, str(src / "fjall"), str(src), a.account, a.regular_key], check=True)
    for n in st["nodes"][1:]:
        dst = Path(a.dir) / n["name"] / "db"
        shutil.rmtree(dst)
        shutil.copytree(src, dst, ignore=shutil.ignore_patterns("*.db-shm", "*.db-wal", "peerfinder.db"))
    print(f"forked store copied to {len(st['nodes']) - 1} nodes")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("docker-init")
    s.add_argument("--dir", required=True)
    s.add_argument("--bin-dir", required=True, help="directory with the linux/amd64 quaxar binary")
    s.add_argument("--keys-quaxar", required=True, help="a local quaxar binary for validator-keys")
    s.add_argument("--validators", type=int, default=5)
    s.add_argument("--p2p", type=int, default=2)
    s.set_defaults(fn=cmd_docker_init)
    s = sub.add_parser("docker-start")
    s.add_argument("--dir", required=True)
    s.add_argument("--mode", choices=["start", "load"], required=True)
    s.set_defaults(fn=cmd_docker_start)
    for name, fn in [("docker-stop", cmd_docker_stop), ("docker-status", cmd_docker_status)]:
        s = sub.add_parser(name)
        s.add_argument("--dir", required=True)
        s.set_defaults(fn=fn)
    s = sub.add_parser("docker-config")
    s.add_argument("--dir", required=True)
    s.add_argument("--setup-txq", action="store_true")
    s.add_argument("--log-level", default="warning")
    s.set_defaults(fn=cmd_docker_config)
    s = sub.add_parser("docker-fork")
    s.add_argument("--dir", required=True)
    s.add_argument("--fork", required=True)
    s.add_argument("--account", required=True)
    s.add_argument("--regular-key", required=True)
    s.set_defaults(fn=cmd_docker_fork)
    s = sub.add_parser("local-init")
    s.add_argument("--dir", required=True)
    s.add_argument("--quaxar", required=True)
    s.add_argument("--validators", type=int, default=5)
    s.add_argument("--p2p", type=int, default=2)
    s.set_defaults(fn=cmd_local_init)
    s = sub.add_parser("local-start")
    s.add_argument("--dir", required=True)
    s.add_argument("--mode", choices=["start", "load"], required=True)
    s.set_defaults(fn=cmd_local_start)
    for name, fn in [("local-stop", cmd_local_stop), ("local-status", cmd_local_status)]:
        s = sub.add_parser(name)
        s.add_argument("--dir", required=True)
        s.set_defaults(fn=fn)
    s = sub.add_parser("local-fork")
    s.add_argument("--dir", required=True)
    s.add_argument("--fork", required=True)
    s.add_argument("--account", required=True)
    s.add_argument("--regular-key", required=True)
    s.set_defaults(fn=cmd_local_fork)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import tps_aws
    tps_aws.register(sub)
    a = p.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
