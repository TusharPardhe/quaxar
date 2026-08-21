# Quaxar AWS testnet node

`provision-testnet.sh` creates one persistent Quaxar public testnet node in
`us-east-1`. It deliberately uses the normal (non-spot) `m7i.xlarge` profile:
4 vCPUs and 16 GiB RAM are a suitable non-burstable host for Quaxar's
`[node_size] medium` profile, whose acquisition concurrency defaults to four.

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

The provisioner is intentionally idempotent for its named security group and
Elastic IP. It refuses to move an EIP attached to an unknown instance.

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

```bash
ssh -i ~/.ssh/quaxar-testnet-2026.pem ubuntu@<elastic-ip> \
  'sudo tail -f /var/log/quaxar-bootstrap.log'

ssh -i ~/.ssh/quaxar-testnet-2026.pem ubuntu@<elastic-ip> \
  'sudo systemctl status quaxar --no-pager; sudo cat /etc/quaxar/build-info'

ssh -i ~/.ssh/quaxar-testnet-2026.pem -L 5005:127.0.0.1:5005 ubuntu@<elastic-ip>
curl -sS http://127.0.0.1:5005 -d '{"method":"server_info","params":[{}]}'
```

To stop compute charges without releasing the static endpoint, stop the EC2
instance. The EIP and retained 200 GiB volume continue to incur AWS charges.
To fully retire the node, terminate the instance, then deliberately delete the
retained tagged EBS volume, release the EIP, and delete the tagged security
group after confirming no other resource uses it.
