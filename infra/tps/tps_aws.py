"""AWS side of tps.py: hosts, bundle, configs, the mainnet sync, the fork,
the runs and teardown. Uses the AWS CLI and ssh/scp. Every resource carries
the tag quaxar-tps=<run>; `aws-down` deletes by that tag and then checks
that nothing is left.

State lives in ~/.quaxar-tools/tps/<run>/ (keys, IPs, results), outside the
repository.
"""

import json
import os
import shlex
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
KEY_NAME = "quaxar-tps"
PEM = Path.home() / ".ssh" / "quaxar-testnet-2026.pem"
AMI_PARAM = "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"
LIFETIME_MIN = 300

# name -> (role, instance type)
LAYOUT = {
    **{f"v{i}": ("validator", "z1d.2xlarge") for i in range(5)},
    **{f"p{i}": ("p2p", "z1d.2xlarge") for i in range(4)},
    **{f"l{i}": ("load", "c7i.xlarge") for i in range(4)},
}
SYNC_HOST = "p0"
NETWORK_ROLES = ("validator", "p2p")

# Mainnet peers of the mainnet validator, used only by the sync host.
MAINNET_IPS = """r.ripple.com 51235
sahyadri.isrdc.in 51235
hubs.xrpkuwait.com 51235
zaphod.alloy.ee 51235
bacab.alloy.ee 51235"""


def run_dir(run):
    d = Path.home() / ".quaxar-tools" / "tps" / run
    d.mkdir(parents=True, exist_ok=True, mode=0o700)
    return d


def load(run):
    f = run_dir(run) / "state.json"
    return json.loads(f.read_text()) if f.exists() else {}


def save(run, st):
    f = run_dir(run) / "state.json"
    f.write_text(json.dumps(st, indent=1))
    os.chmod(f, 0o600)


def aws(region, *args, check=True):
    p = subprocess.run(["aws", "--region", region, "--output", "json", *args],
                       capture_output=True, text=True)
    if check and p.returncode != 0:
        raise RuntimeError(f"aws {' '.join(args[:2])}: {p.stderr.strip()}")
    return json.loads(p.stdout) if p.stdout.strip() else {}, p


def my_ip():
    import urllib.request
    return urllib.request.urlopen("https://checkip.amazonaws.com", timeout=10).read().decode().strip()


# --- SSH ---------------------------------------------------------------------

SSH_OPTS = ["-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR",
            "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=15", "-i", str(PEM), "-o", "IdentitiesOnly=yes"]


def ssh(st, name, command, check=True, capture=True, timeout=None):
    host = st["hosts"][name]["public_ip"]
    p = subprocess.run(["ssh", *SSH_OPTS, f"ubuntu@{host}", command], capture_output=capture, text=True,
                       timeout=timeout)
    if check and p.returncode != 0:
        raise RuntimeError(f"{name}: {command[:80]}: exit {p.returncode}: {(p.stderr or '').strip()[-400:]}")
    return p.stdout if capture else ""


def scp_to(st, name, local, remote):
    host = st["hosts"][name]["public_ip"]
    subprocess.run(["scp", "-q", *SSH_OPTS, str(local), f"ubuntu@{host}:{remote}"], check=True)


def scp_from(st, name, remote, local):
    host = st["hosts"][name]["public_ip"]
    subprocess.run(["scp", "-q", "-r", *SSH_OPTS, f"ubuntu@{host}:{remote}", str(local)], check=True)


def parallel(names, fn):
    with ThreadPoolExecutor(max_workers=max(1, len(names))) as ex:
        results = dict(zip(names, ex.map(fn, names)))
    return results


def names(st, roles=None):
    return [n for n in st["hosts"] if roles is None or st["hosts"][n]["role"] in roles]


# --- commands ----------------------------------------------------------------


def cmd_check(a):
    """Gate G4: permissions, AMI, subnet and spot capacity without launching."""
    ami = aws(a.region, "ssm", "get-parameter", "--name", AMI_PARAM)[0]["Parameter"]["Value"]
    subnet = subnet_for(a.region, a.az)
    print(f"AMI {ami}, subnet {subnet} ({a.az})")
    for itype in sorted({t for _, t in LAYOUT.values()}):
        for market in ("spot", "on-demand"):
            args = ["ec2", "run-instances", "--dry-run", "--image-id", ami, "--instance-type", itype,
                    "--subnet-id", subnet, "--count", "1"]
            if market == "spot":
                args += ["--instance-market-options", "MarketType=spot"]
            _, p = aws(a.region, *args, check=False)
            ok = "DryRunOperation" in p.stderr
            print(f"{itype:12} {market:9} {'OK' if ok else p.stderr.strip()[:200]}")


def subnet_for(region, az):
    subnets = aws(region, "ec2", "describe-subnets", "--filters", f"Name=availability-zone,Values={az}",
                  "Name=default-for-az,Values=true")[0]["Subnets"]
    if not subnets:
        sys.exit(f"no default subnet in {az}")
    return subnets[0]["SubnetId"]


def ensure_key_pair(region):
    have = aws(region, "ec2", "describe-key-pairs", "--filters", f"Name=key-name,Values={KEY_NAME}")[0]["KeyPairs"]
    if have:
        return
    pub = subprocess.run(["ssh-keygen", "-y", "-f", str(PEM)], check=True, capture_output=True, text=True).stdout
    tmp = Path("/tmp/quaxar-tps-key.pub")
    tmp.write_text(pub)
    aws(region, "ec2", "import-key-pair", "--key-name", KEY_NAME, "--public-key-material", f"fileb://{tmp}",
        "--tag-specifications", "ResourceType=key-pair,Tags=[{Key=quaxar-tps,Value=shared}]")


def ensure_run_key(run):
    k = run_dir(run) / "run_key"
    if not k.exists():
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", f"quaxar-tps-{run}", "-f", str(k)],
                       check=True)
    return k


def ensure_sg(region, run, st):
    if st.get("sg"):
        return st["sg"]
    vpc = aws(region, "ec2", "describe-vpcs", "--filters", "Name=isDefault,Values=true")[0]["Vpcs"][0]["VpcId"]
    sg = aws(region, "ec2", "create-security-group", "--group-name", f"quaxar-tps-{run}", "--vpc-id", vpc,
             "--description", f"quaxar throughput test {run}",
             "--tag-specifications", f"ResourceType=security-group,Tags=[{{Key=quaxar-tps,Value={run}}}]")[0]["GroupId"]
    aws(region, "ec2", "authorize-security-group-ingress", "--group-id", sg, "--ip-permissions",
        json.dumps([{"IpProtocol": "-1", "UserIdGroupPairs": [{"GroupId": sg}]},
                    {"IpProtocol": "tcp", "FromPort": 22, "ToPort": 22,
                     "IpRanges": [{"CidrIp": f"{my_ip()}/32", "Description": "operator"}]}]))
    st["sg"] = sg
    return sg


def cmd_allow_ssh(a):
    st = load(a.run)
    aws(st["region"], "ec2", "authorize-security-group-ingress", "--group-id", st["sg"], "--ip-permissions",
        json.dumps([{"IpProtocol": "tcp", "FromPort": 22, "ToPort": 22,
                     "IpRanges": [{"CidrIp": f"{my_ip()}/32", "Description": "operator"}]}]), check=False)


def cmd_up(a):
    st = load(a.run)
    st.update({"run": a.run, "region": a.region, "az": a.az})
    st.setdefault("hosts", {})
    ensure_key_pair(a.region)
    run_key = ensure_run_key(a.run)
    sg = ensure_sg(a.region, a.run, st)
    save(a.run, st)
    ami = aws(a.region, "ssm", "get-parameter", "--name", AMI_PARAM)[0]["Parameter"]["Value"]
    subnet = subnet_for(a.region, a.az)
    boot = (HERE / "boot.sh").read_text().replace("__RUN_PUBKEY__", Path(f"{run_key}.pub").read_text().strip())
    boot = boot.replace("__LIFETIME_MIN__", str(a.lifetime))
    boot_file = run_dir(a.run) / "boot.sh"
    boot_file.write_text(boot)
    wanted = a.only.split(",") if a.only else list(LAYOUT)
    launch = [n for n in wanted if n not in st["hosts"] or st["hosts"][n].get("state") == "terminated"]
    for n in launch:
        role, itype = LAYOUT[n]
        tags = f"{{Key=quaxar-tps,Value={a.run}}},{{Key=Name,Value=quaxar-tps-{a.run}-{n}}}"
        args = ["ec2", "run-instances", "--image-id", ami, "--instance-type", itype, "--key-name", KEY_NAME,
                "--security-group-ids", sg, "--subnet-id", subnet, "--count", "1",
                "--user-data", f"file://{boot_file}", "--instance-initiated-shutdown-behavior", "terminate",
                "--block-device-mappings",
                json.dumps([{"DeviceName": "/dev/sda1", "Ebs": {"VolumeSize": 30, "VolumeType": "gp3",
                                                                 "DeleteOnTermination": True}}]),
                "--tag-specifications", f"ResourceType=instance,Tags=[{tags}]", f"ResourceType=volume,Tags=[{tags}]"]
        market = "on-demand" if a.on_demand else "spot"
        if market == "spot":
            args += ["--instance-market-options",
                     "MarketType=spot,SpotOptions={SpotInstanceType=one-time,InstanceInterruptionBehavior=terminate}"]
        out, p = aws(a.region, *args, check=False)
        if p.returncode != 0 and market == "spot" and a.fallback:
            print(f"{n}: spot failed ({p.stderr.strip()[:120]}); launching on demand")
            args = args[:-2]
            out, p = aws(a.region, *args, check=False)
            market = "on-demand"
        if p.returncode != 0:
            save(a.run, st)
            sys.exit(f"{n}: launch failed: {p.stderr.strip()}")
        inst = out["Instances"][0]
        st["hosts"][n] = {"role": role, "type": itype, "id": inst["InstanceId"], "market": market,
                          "private_ip": inst["PrivateIpAddress"]}
        save(a.run, st)
        print(f"{n}: {inst['InstanceId']} {itype} {market}")
    ids = [st["hosts"][n]["id"] for n in launch]
    if ids:
        aws(a.region, "ec2", "wait", "instance-running", "--instance-ids", *ids)
    refresh(st)
    save(a.run, st)
    wait_boot(st, launch)
    print(json.dumps({n: (h.get("public_ip"), h.get("private_ip")) for n, h in st["hosts"].items()}, indent=1))


def refresh(st):
    ids = [h["id"] for h in st["hosts"].values()]
    if not ids:
        return
    res = aws(st["region"], "ec2", "describe-instances", "--instance-ids", *ids)[0]
    by_id = {i["InstanceId"]: i for r in res["Reservations"] for i in r["Instances"]}
    for h in st["hosts"].values():
        i = by_id.get(h["id"], {})
        h["state"] = i.get("State", {}).get("Name", "unknown")
        h["public_ip"] = i.get("PublicIpAddress")


def wait_boot(st, which):
    def one(n):
        for _ in range(60):
            try:
                if ssh(st, n, "test -f /var/lib/tps-boot-done && echo ok", check=False).strip() == "ok":
                    return "ready"
            except Exception:  # noqa: BLE001 - still booting
                pass
            time.sleep(5)
        return "timeout"
    print(parallel(which, one))


def cmd_bundle(a):
    """Copy the bundle to the sync host, then from it to every other host
    over the private network."""
    st = load(a.run)
    b = Path(a.bin_dir)
    tar = run_dir(a.run) / "bundle.tgz"
    files = ["quaxar", "tps-loadgen", "tps-fork"]
    for f in files:
        if not (b / f).exists():
            sys.exit(f"missing {b / f}")
    subprocess.run(["tar", "-czf", str(tar), "-C", str(b), *files, "-C", str(HERE), "sample.sh"], check=True)
    hub = a.hub
    scp_to(st, hub, tar, "/tmp/bundle.tgz")
    scp_to(st, hub, run_dir(a.run) / "run_key", "/home/ubuntu/.ssh/run_key")
    ssh(st, hub, "chmod 600 ~/.ssh/run_key && sudo mkdir -p /opt/tps && sudo tar -xzf /tmp/bundle.tgz -C /opt/tps")
    others = [n for n in st["hosts"] if n != hub]
    hub_ip = st["hosts"][hub]["private_ip"]
    for n in others:
        scp_to(st, n, run_dir(a.run) / "run_key", "/home/ubuntu/.ssh/run_key")

    def pull(n):
        ssh(st, n, f"chmod 600 ~/.ssh/run_key && scp -q -o StrictHostKeyChecking=no -i ~/.ssh/run_key "
                   f"ubuntu@{hub_ip}:/tmp/bundle.tgz /tmp/bundle.tgz && sudo mkdir -p /opt/tps && "
                   f"sudo tar -xzf /tmp/bundle.tgz -C /opt/tps && /opt/tps/quaxar --version")
        return "ok"
    print(parallel(others, pull))


UNIT = """[Unit]
Description=quaxar throughput test node
After=network-online.target
[Service]
User=ubuntu
EnvironmentFile=/data/mode.env
ExecStart=/opt/tps/quaxar --conf /data/quaxar.cfg $MODE_ARGS
LimitNOFILE=1048576
TimeoutStopSec=120
Restart=no
[Install]
WantedBy=multi-user.target
"""


def network_nodes(st):
    """Config entries of the validators and P2P nodes (private IPs)."""
    nodes = []
    for n in names(st, NETWORK_ROLES):
        h = st["hosts"][n]
        e = {"name": n, "role": h["role"], "peer_ip": h["private_ip"], "peer_port": 51235,
             "rpc_port": 5005, "ws_port": 6006}
        if h["role"] == "validator":
            e.update(st["validator_keys"][n])
        nodes.append(e)
    return nodes


def install_unit(st, n):
    ssh(st, n, f"echo {shlex.quote(UNIT)} | sudo tee /etc/systemd/system/quaxar-tps.service >/dev/null && "
               "sudo systemctl daemon-reload && mkdir -p /data/db && test -f /data/mode.env || echo 'MODE_ARGS=' > /data/mode.env")


def cmd_keys(a):
    """Validator keys for the run, generated locally with quaxar."""
    import tps
    st = load(a.run)
    if st.get("validator_keys"):
        print("validator keys exist")
        return
    keys = tps.generate_validator_keys(a.quaxar, 5, run_dir(a.run))
    st["validator_keys"] = {f"v{i}": k for i, k in enumerate(keys)}
    save(a.run, st)
    print("generated 5 validator keys")


def cmd_config(a):
    """Write the private-network config and the service unit to every node."""
    import tps
    st = load(a.run)
    nodes = network_nodes(st)

    def one(n):
        node = next(x for x in nodes if x["name"] == n)
        cfg = tps.render_config(node, nodes, "/data", "0.0.0.0", log_level=a.log_level, setup_txq=a.setup_txq)
        install_unit(st, n)
        p = subprocess.run(["ssh", *SSH_OPTS, f"ubuntu@{st['hosts'][n]['public_ip']}",
                            "cat > /data/quaxar.cfg && chmod 600 /data/quaxar.cfg"], input=cfg, text=True)
        return "ok" if p.returncode == 0 else f"exit {p.returncode}"
    print(parallel(names(st, NETWORK_ROLES), one))


def sync_config(data="/data"):
    return f"""[server]
port_rpc
port_ws
port_peer

[port_rpc]
ip = 127.0.0.1
port = 5005
protocol = http
admin = 127.0.0.1

[port_ws]
ip = 127.0.0.1
port = 6006
protocol = ws
admin = 127.0.0.1

[port_peer]
ip = 0.0.0.0
port = 51235
protocol = peer

[network_id]
main

[node_size]
huge

[node_db]
type = fjall
path = {data}/db/fjall
online_delete = 512
advisory_delete = 0
reserve_mb = 1024
compaction_base_mb = 2048

[database_path]
{data}/db

[ledger_history]
256

[ips]
{MAINNET_IPS}

[validator_list_sites]
https://vl.ripple.com
https://unl.xrplf.org

[validator_list_keys]
ED2677ABFFD1B33AC6FBC3062B71F1E8397C1505E1C42C64D11AD1B28FF73F4734
ED42AEC58B701EEBB77356FFFEC26F83C1F0407263530F068C7C73D392C7E06FD1

[validator_list_threshold]
0

[debug_logfile]
{data}/debug.log

[rpc_startup]
{{ "command": "log_level", "severity": "warning" }}

[ssl_verify]
1
"""


def cmd_sync(a):
    """Sync the sync host from mainnet (non-validating) and wait until it has
    a complete validated ledger."""
    st = load(a.run)
    n = SYNC_HOST
    install_unit(st, n)
    p = subprocess.run(["ssh", *SSH_OPTS, f"ubuntu@{st['hosts'][n]['public_ip']}",
                        "cat > /data/quaxar.cfg && echo 'MODE_ARGS=' > /data/mode.env && sudo systemctl restart quaxar-tps"],
                       input=sync_config(), text=True)
    if p.returncode:
        sys.exit("sync start failed")
    t0 = time.time()
    while True:
        out = ssh(st, n, "curl -s -m 5 -d '{\"method\":\"server_info\"}' http://127.0.0.1:5005/ || true", check=False)
        try:
            info = json.loads(out)["result"]["info"]
            state, v, cl = info.get("server_state"), info.get("validated_ledger", {}).get("seq"), info.get("complete_ledgers")
        except Exception:  # noqa: BLE001 - still starting
            state, v, cl = "starting", None, None
        print(f"{time.time() - t0:6.0f}s {state} validated {v} complete {cl}", flush=True)
        if state in ("full", "proposing") and cl and cl != "empty" and "-" in cl:
            lo, hi = (int(x) for x in cl.split(",")[-1].split("-"))
            if hi - lo >= a.min_ledgers:
                break
        time.sleep(30)
    st["sync_seconds"] = time.time() - t0
    save(a.run, st)
    ssh(st, n, "sudo systemctl stop quaxar-tps")
    print(f"synced in {st['sync_seconds']:.0f}s; stopped")


def cmd_fork(a):
    st = load(a.run)
    out = ssh(st, SYNC_HOST, f"/opt/tps/tps-fork /data/db/fjall /data/db {shlex.quote(a.account)} {shlex.quote(a.regular_key)}")
    print(out)
    st["funder"] = a.account
    save(a.run, st)


def cmd_distribute(a):
    """Every other network node pulls the forked store from the sync host."""
    st = load(a.run)
    src = st["hosts"][SYNC_HOST]["private_ip"]
    targets = [n for n in names(st, NETWORK_ROLES) if n != SYNC_HOST]

    def pull(n):
        t = time.time()
        ssh(st, n, "sudo systemctl stop quaxar-tps 2>/dev/null; rm -rf /data/db && "
                   f"ssh -o StrictHostKeyChecking=no -i ~/.ssh/run_key ubuntu@{src} "
                   "'tar -C /data --exclude=*.db-shm --exclude=*.db-wal --exclude=peerfinder.db -cf - db' | tar -C /data -xf -")
        return f"{time.time() - t:.0f}s"
    ssh(st, SYNC_HOST, "rm -f /data/db/peerfinder.db /data/db/*.db-shm /data/db/*.db-wal; du -sh /data/db")
    print(parallel(targets, pull))


def cmd_start(a):
    st = load(a.run)
    mode = {"load": "--load --valid", "start": "--start --valid", "none": "", "net": "--net"}[a.mode]
    if a.quorum:
        mode += f" --quorum {a.quorum}"

    def one(n):
        ssh(st, n, f"echo 'MODE_ARGS={mode}' > /data/mode.env && sudo systemctl restart quaxar-tps")
        return "started"
    if not a.sequential:
        print(parallel(names(st, NETWORK_ROLES), one))
        return
    # The first validator starts alone; the others start once it is closing
    # ledgers, and must join its chain. Nodes that finish loading at
    # different moments and each close the next ledger on their own can
    # otherwise split across ledgers that never reach quorum.
    order = names(st, ["validator"]) + names(st, ["p2p"])
    leader = order[0]
    t0 = time.time()
    one(leader)
    while closed(st, leader) is None:
        time.sleep(5)
    lead_start = closed(st, leader)[0]
    print(f"{leader}: up at {lead_start} after {time.time() - t0:.0f}s", flush=True)
    rest = order[1:]
    parallel(rest, one)
    pending = set(rest)
    while pending:
        time.sleep(10)
        lead = closed(st, leader)
        for n in sorted(pending):
            info = closed(st, n)
            if info and lead and info[0] > lead_start and abs(info[0] - lead[0]) <= 1:
                print(f"{n}: joined at {info[0]} after {time.time() - t0:.0f}s", flush=True)
                pending.discard(n)
        if time.time() - t0 > 900:
            sys.exit(f"not joined after 15 minutes: {sorted(pending)}")


def closed(st, n):
    """(seq, hash) of the node's last closed ledger, or None."""
    out = ssh(st, n, "curl -s -m 5 -d '{\"method\":\"ledger_closed\"}' http://127.0.0.1:5005/ || true", check=False)
    try:
        r = json.loads(out)["result"]
        return int(r["ledger_index"]), r["ledger_hash"]
    except Exception:  # noqa: BLE001 - not answering yet
        return None


def cmd_stop(a):
    st = load(a.run)
    print(parallel(names(st, NETWORK_ROLES),
                   lambda n: ssh(st, n, "sudo timeout 170 systemctl stop quaxar-tps; systemctl is-active quaxar-tps || true",
                                 timeout=200).strip()))


def cmd_status(a):
    st = load(a.run)
    refresh(st)
    save(a.run, st)

    def one(n):
        h = st["hosts"][n]
        if h["state"] != "running" or h["role"] == "load":
            return f"{h['state']}"
        out = ssh(st, n, "curl -s -m 5 -d '{\"method\":\"server_info\"}' http://127.0.0.1:5005/ || true", check=False)
        try:
            i = json.loads(out)["result"]["info"]
            v = i.get("validated_ledger", {})
            return f"{i.get('server_state')} validated {v.get('seq')} {str(v.get('hash'))[:12]} peers {i.get('peers')}"
        except Exception:  # noqa: BLE001
            return "quaxar not answering"
    for n, s in parallel(list(st["hosts"]), one).items():
        print(f"{n:3} {st['hosts'][n]['type']:12} {st['hosts'][n].get('public_ip')} {s}")


def cmd_baseline(a):
    st = load(a.run)
    if a.action == "save":
        cmd = "sudo systemctl stop quaxar-tps; rm -rf /data/baseline && cp -a /data/db /data/baseline && du -sh /data/baseline"
    else:
        cmd = "sudo systemctl stop quaxar-tps; test -d /data/baseline && rm -rf /data/db && cp -a /data/baseline /data/db && echo restored"
    print(parallel(names(st, NETWORK_ROLES), lambda n: ssh(st, n, cmd).strip()))


def ws(st, n):
    return f"ws://{st['hosts'][n]['private_ip']}:6006"


def cmd_setup(a):
    st = load(a.run)
    p2p = ",".join(ws(st, n) for n in names(st, ["p2p"]))
    cmd = (f"/opt/tps/tps-loadgen setup --tag {shlex.quote(a.tag)} --funder {st['funder']} --nodes {p2p} "
           f"--tests {a.tests} --iou-group {a.iou_group} --rate {a.rate} 2>&1 | tee /tmp/setup.log")
    ssh(st, "l0", cmd, capture=False)


def cmd_run(a):
    """One measured run: ledger watcher on l0, load from l0..l3 (each to its
    own P2P node), samplers on every host; results copied back."""
    st = load(a.run)
    import tps_steps
    steps = a.steps
    total = tps_steps.total_seconds(steps)
    label = a.label
    start_at = time.time() + 25
    out = run_dir(a.run) / "results" / label
    out.mkdir(parents=True, exist_ok=True)
    validators = ",".join(ws(st, n) for n in names(st, ["validator"]))
    loads = names(st, ["load"])
    p2ps = names(st, ["p2p"])
    secs = int(total + 60)
    rd = f"/tmp/run-{label}"

    def sampler(n):
        ssh(st, n, f"mkdir -p {rd} && nohup bash /opt/tps/sample.sh {rd}/sample-{n}.csv 10 {secs} >/dev/null 2>&1 &")
        return "ok"
    parallel(list(st["hosts"]), sampler)
    ssh(st, "l0", f"mkdir -p {rd} && nohup /opt/tps/tps-loadgen watch --validators {validators} --seconds {secs} "
                  f"--out {rd}/ledgers.jsonl > {rd}/watch.log 2>&1 &")

    def load_host(i_n):
        i, n = i_n
        p = p2ps[i % len(p2ps)]
        cmd = (f"mkdir -p {rd} && /opt/tps/tps-loadgen run --tag {shlex.quote(a.tag)} --nodes {ws(st, p)} "
               f"--host {i} --hosts {len(loads)} --steps {steps} --mix {a.mix} --start-at {start_at:.0f} "
               f"--tests {a.tests} --iou-group {a.iou_group} --fee-mult {a.fee_mult} --max-fee {a.max_fee} "
               f"--out {rd}/load-{n}.jsonl > {rd}/load-{n}.log 2>&1; tail -3 {rd}/load-{n}.log")
        return ssh(st, n, cmd, check=False, timeout=secs + 600)
    print(f"run {label}: {steps}, starting at {time.strftime('%H:%M:%S', time.gmtime(start_at))} UTC, {total:.0f}s")
    with ThreadPoolExecutor(max_workers=len(loads)) as ex:
        for n, r in zip(loads, ex.map(load_host, enumerate(loads))):
            print(n, r.strip())
    time.sleep(70)

    def fetch(n):
        try:
            scp_from(st, n, f"{rd}/*", out)
            return "ok"
        except subprocess.CalledProcessError as e:
            return f"copy failed {e.returncode}"
    print(parallel(list(st["hosts"]), fetch))
    (out / "params.json").write_text(json.dumps(vars(a), default=str, indent=1))
    print(f"results in {out}")


def cmd_isolate(a):
    """Close outbound traffic to anything outside the security group."""
    st = load(a.run)
    aws(st["region"], "ec2", "revoke-security-group-egress", "--group-id", st["sg"], "--ip-permissions",
        json.dumps([{"IpProtocol": "-1", "IpRanges": [{"CidrIp": "0.0.0.0/0"}]}]), check=False)
    aws(st["region"], "ec2", "authorize-security-group-egress", "--group-id", st["sg"], "--ip-permissions",
        json.dumps([{"IpProtocol": "-1", "UserIdGroupPairs": [{"GroupId": st["sg"]}]}]), check=False)
    print("egress limited to the security group")


def cmd_down(a):
    st = load(a.run)
    region = st.get("region", a.region)
    res = aws(region, "ec2", "describe-instances", "--filters", f"Name=tag:quaxar-tps,Values={a.run}",
              "Name=instance-state-name,Values=pending,running,stopping,stopped")[0]
    ids = [i["InstanceId"] for r in res["Reservations"] for i in r["Instances"]]
    if ids:
        aws(region, "ec2", "terminate-instances", "--instance-ids", *ids)
        aws(region, "ec2", "wait", "instance-terminated", "--instance-ids", *ids)
    print(f"terminated {len(ids)}")
    if st.get("sg"):
        for _ in range(12):
            _, p = aws(region, "ec2", "delete-security-group", "--group-id", st["sg"], check=False)
            if p.returncode == 0 or "InvalidGroup.NotFound" in p.stderr:
                break
            time.sleep(10)
        st["sg"] = None
    for h in st.get("hosts", {}).values():
        h["state"] = "terminated"
    save(a.run, st)
    left = aws(region, "ec2", "describe-instances", "--filters", f"Name=tag:quaxar-tps,Values={a.run}",
               "Name=instance-state-name,Values=pending,running,stopping,stopped")[0]["Reservations"]
    vols = aws(region, "ec2", "describe-volumes", "--filters", f"Name=tag:quaxar-tps,Values={a.run}")[0]["Volumes"]
    sgs = aws(region, "ec2", "describe-security-groups", "--filters", f"Name=tag:quaxar-tps,Values={a.run}")[0]["SecurityGroups"]
    print(f"left: {len(left)} instances, {len(vols)} volumes, {len(sgs)} security groups")


def register(sub):
    def add(name, fn, *args):
        s = sub.add_parser(name)
        s.add_argument("--run", required=True)
        for spec in args:
            flag, kw = spec
            s.add_argument(flag, **kw)
        s.set_defaults(fn=fn)
        return s
    s = sub.add_parser("aws-check")
    s.add_argument("--region", default="us-east-2")
    s.add_argument("--az", default="us-east-2b")
    s.set_defaults(fn=cmd_check)
    add("aws-up", cmd_up, ("--region", {"default": "us-east-2"}), ("--az", {"default": "us-east-2b"}),
        ("--only", {"default": ""}), ("--on-demand", {"action": "store_true"}),
        ("--fallback", {"action": "store_true"}), ("--lifetime", {"type": int, "default": LIFETIME_MIN}))
    add("aws-allow-ssh", cmd_allow_ssh)
    add("aws-keys", cmd_keys, ("--quaxar", {"required": True}))
    add("aws-bundle", cmd_bundle, ("--bin-dir", {"required": True}), ("--hub", {"default": SYNC_HOST}))
    add("aws-config", cmd_config, ("--log-level", {"default": "warning"}), ("--setup-txq", {"action": "store_true"}))
    add("aws-sync", cmd_sync, ("--min-ledgers", {"type": int, "default": 20}))
    add("aws-fork", cmd_fork, ("--account", {"required": True}), ("--regular-key", {"required": True}))
    add("aws-distribute", cmd_distribute)
    add("aws-start", cmd_start, ("--mode", {"choices": ["load", "start", "none", "net"], "required": True}),
        ("--sequential", {"action": "store_true"}), ("--quorum", {"type": int, "default": 0}))
    add("aws-stop", cmd_stop)
    add("aws-status", cmd_status)
    add("aws-baseline", cmd_baseline, ("--action", {"choices": ["save", "restore"], "required": True}))
    common = [("--tag", {"required": True}), ("--tests", {"type": int, "default": 250_000}),
              ("--iou-group", {"type": int, "default": 20_000})]
    add("aws-setup", cmd_setup, *common, ("--rate", {"type": int, "default": 1000}))
    add("aws-run", cmd_run, *common, ("--steps", {"required": True}), ("--label", {"required": True}),
        ("--mix", {"default": "xrp=20,iou=20,p1=20,p3=20,p6=20"}), ("--fee-mult", {"default": "1.2"}),
        ("--max-fee", {"default": "1000000"}))
    add("aws-isolate", cmd_isolate)
    add("aws-down", cmd_down, ("--region", {"default": "us-east-2"}))
