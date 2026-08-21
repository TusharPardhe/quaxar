# Quaxar AWS testnet node

`provision-testnet.sh` creates one persistent Quaxar public testnet node in
`us-east-1`. It deliberately uses the normal (non-spot) `m7i.xlarge` profile:
4 vCPUs and 16 GiB RAM are a suitable non-burstable host for Quaxar's
`[node_size] medium` profile. Acquisition execution is independently bounded
by its worker pool and ready scheduler; session count is not a four-target
configuration limit.

## Design

- **Network:** default VPC `vpc-04f8eca17bff44359`, public subnet
  `subnet-0b98157ee9b7d691a` (`us-east-1a`), with an allocated Elastic IP.
  The EIP is written to `[overlay] public_ip`, so the peer endpoint remains
  stable across a stop/start cycle.
- **Ingress:** TCP/51235 is public for XRPL peer overlay traffic. TCP/22 is
  limited to the explicit `SSH_CIDR`; RPC ports 5005 and 6006 bind only to
  `127.0.0.1` and must be reached over SSH forwarding. No private key is
  copied to the instance or committed.
- **Compute and data:** `m7i.xlarge`, Ubuntu 24.04 amd64, plus a 200 GiB
  encrypted gp3 root volume (3,000 IOPS/125 MB/s). The volume is tagged for
  NuDB and has `DeleteOnTermination=false`, preserving the database if the
  instance is terminated deliberately.
- **Ledger:** testnet network ID `1`, NuDB at `/var/lib/quaxar/db/nudb`,
  `ledger_history = 256`, `online_delete = 256`, and `node_size = medium`.
- **Lifecycle:** `systemd` owns `quaxar.service`; bootstrap output is in
  `/var/log/quaxar-bootstrap.log`. The build records its checked-out commit in
  `/etc/quaxar/build-info`.

The provisioned node is non-validating unless the operator separately installs
a protected `[validation_seed]` and restarts the service. The provisioner does
not generate, copy, or print validator secrets.

AWS resource discovery is idempotent for the named security group and Elastic
IP. It refuses to move an EIP attached to an unknown instance. The bootstrap
script is a fresh/canonical-host installer, not a transactional general-purpose
upgrader; do not rerun it manually on a custom deployment without a rollback
plan.
It owns only fresh hosts and the canonical `/var/lib/quaxar` layout. If it
detects a legacy service, data/log directory, or config, it refuses before
changing service state or ownership; follow the reviewed manual cutover in
[RUNNING.md](../../docs/RUNNING.md) instead. Bootstrap enables the new service
only after config validation and a successful local `server_info` response.
The readiness wait defaults to 180 seconds and can be changed, up to 900
seconds, with `RPC_READY_TIMEOUT_SECONDS` in the rendered bootstrap environment.

## Provision

The EC2 key pair `quaxar-testnet-2026` must already exist in `us-east-1` and
match the local `~/.ssh/quaxar-testnet-2026.pem`. Select the SSH source CIDR
rather than exposing port 22 globally:

```bash
cd infra/aws
SSH_CIDR="$(curl -fsS https://checkip.amazonaws.com | tr -d '\n')/32" \
QUAXAR_REF=pr/preferred-lcl-mode-promotion \
./provision-testnet.sh
```

The script returns the instance ID, EIP allocation ID, static public IP, and
an SSH command. It uses the verified Canonical Ubuntu 24.04 AMI set in the
script; override `AMI_ID`, `SUBNET_ID`, `VOLUME_SIZE_GIB`, or `INSTANCE_TYPE`
only when intentionally changing the deployment design.

## Verify and operate

The initial Rust build can take several minutes. Do not treat an EC2
`instance-status-ok` result as proof that application bootstrap has finished.
The bootstrap RPC check proves local service reachability, not completion of
ledger synchronization. Before enabling validator operation, repeatedly
confirm local closed/current, validated, published, and coordinator phase
identities advance together without recurring mode transitions.
Initial RSS growth reflects reusable shared SHAMap/cache population, not
readiness by itself; a small bootstrap ledger can advance locally until the
current network LCL is complete and installed.

```bash
ssh -i ~/.ssh/quaxar-testnet-2026.pem ubuntu@<elastic-ip> \
  'sudo tail -f /var/log/quaxar-bootstrap.log'

ssh -i ~/.ssh/quaxar-testnet-2026.pem ubuntu@<elastic-ip> \
  'sudo systemctl status quaxar --no-pager; sudo cat /etc/quaxar/build-info'

ssh -i ~/.ssh/quaxar-testnet-2026.pem -L 5005:127.0.0.1:5005 ubuntu@<elastic-ip>
curl -sS http://127.0.0.1:5005 -d '{"method":"server_info","params":[{}]}'
curl -sS http://127.0.0.1:5005 -d '{"method":"ledger_closed","params":[{}]}'
curl -sS http://127.0.0.1:5005 -d '{"method":"fetch_info","params":[{}]}'
curl -sS http://127.0.0.1:5005 -d '{"method":"get_counts","params":[{}]}'
```

For deployment evidence, also record the staged binary checksum, build commit,
unit/config paths, and recent mode-transition journal entries. Redact validator
seeds and all other credentials.

To stop compute charges without releasing the static endpoint, stop the EC2
instance. The EIP and retained 200 GiB volume continue to incur AWS charges.
To fully retire the node, terminate the instance, then deliberately delete the
retained tagged EBS volume, release the EIP, and delete the tagged security
group after confirming no other resource uses it.
