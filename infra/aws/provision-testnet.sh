#!/usr/bin/env bash
# Provision a single durable Quaxar public testnet node.
# Prerequisite: AWS CLI credentials for the target account and the named EC2 key pair.
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
AWS_REGION="${AWS_REGION:-us-east-1}"
VPC_ID="${VPC_ID:-vpc-04f8eca17bff44359}"
SUBNET_ID="${SUBNET_ID:-subnet-0b98157ee9b7d691a}" # us-east-1a
AMI_ID="${AMI_ID:-ami-052355af2a014bd2c}" # Canonical Ubuntu 24.04 amd64, verified 2026-08-10
INSTANCE_TYPE="${INSTANCE_TYPE:-m7i.xlarge}" # 4 vCPU, 16 GiB; non-burstable medium-node host
KEY_NAME="${KEY_NAME:-quaxar-testnet-2026}"
SSH_CIDR="${SSH_CIDR:?Set SSH_CIDR to the operator IPv4 /32, for example 203.0.113.10/32}"
QUAXAR_REF="${QUAXAR_REF:-pr/preferred-lcl-mode-promotion}"
NODE_NAME="${NODE_NAME:-quaxar-testnet-medium}"
SECURITY_GROUP_NAME="${SECURITY_GROUP_NAME:-${NODE_NAME}-sg}"
EIP_NAME="${EIP_NAME:-${NODE_NAME}-eip}"
VOLUME_SIZE_GIB="${VOLUME_SIZE_GIB:-200}"

for command in aws sed mktemp; do
  command -v "$command" >/dev/null || { echo "Missing required command: $command" >&2; exit 127; }
done
if [[ ! "$SSH_CIDR" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}/(3[0-2]|[12]?[0-9])$ ]]; then
  echo "SSH_CIDR must be an IPv4 CIDR, not an unrestricted range" >&2
  exit 64
fi
if [[ ! "$QUAXAR_REF" =~ ^[A-Za-z0-9._/-]+$ ]]; then
  echo "QUAXAR_REF may only contain letters, digits, dot, underscore, slash, and hyphen" >&2
  exit 64
fi

aws_cmd() {
  aws --region "$AWS_REGION" "$@"
}

SG_ID="$(aws_cmd ec2 describe-security-groups \
  --filters "Name=vpc-id,Values=$VPC_ID" "Name=group-name,Values=$SECURITY_GROUP_NAME" \
  --query 'SecurityGroups[0].GroupId' --output text)"
if [[ "$SG_ID" == "None" || -z "$SG_ID" ]]; then
  SG_ID="$(aws_cmd ec2 create-security-group \
    --group-name "$SECURITY_GROUP_NAME" \
    --description 'Quaxar testnet: SSH restricted, XRPL peer public' \
    --vpc-id "$VPC_ID" --query GroupId --output text)"
  aws_cmd ec2 create-tags --resources "$SG_ID" --tags \
    "Key=Name,Value=$SECURITY_GROUP_NAME" \
    'Key=ManagedBy,Value=quaxar-infra'
fi

# A duplicate-rule error is safe on re-run; any other state remains visible through
# describe-security-groups and causes the later launch to fail rather than opening SSH.
aws_cmd ec2 authorize-security-group-ingress --group-id "$SG_ID" --ip-permissions "[
  {\"IpProtocol\":\"tcp\",\"FromPort\":22,\"ToPort\":22,\"IpRanges\":[{\"CidrIp\":\"$SSH_CIDR\",\"Description\":\"Operator SSH\"}]},
  {\"IpProtocol\":\"tcp\",\"FromPort\":51235,\"ToPort\":51235,\"IpRanges\":[{\"CidrIp\":\"0.0.0.0/0\",\"Description\":\"XRPL peer overlay\"}]}
]" 2>/dev/null || true

EIP_ALLOCATION_ID="$(aws_cmd ec2 describe-addresses \
  --filters "Name=tag:Name,Values=$EIP_NAME" \
  --query 'Addresses[0].AllocationId' --output text)"
if [[ "$EIP_ALLOCATION_ID" == "None" || -z "$EIP_ALLOCATION_ID" ]]; then
  EIP_ALLOCATION_ID="$(aws_cmd ec2 allocate-address --domain vpc --query AllocationId --output text)"
  aws_cmd ec2 create-tags --resources "$EIP_ALLOCATION_ID" --tags \
    "Key=Name,Value=$EIP_NAME" \
    'Key=ManagedBy,Value=quaxar-infra'
fi
PUBLIC_IP="$(aws_cmd ec2 describe-addresses --allocation-ids "$EIP_ALLOCATION_ID" \
  --query 'Addresses[0].PublicIp' --output text)"
ASSOCIATED_INSTANCE="$(aws_cmd ec2 describe-addresses --allocation-ids "$EIP_ALLOCATION_ID" \
  --query 'Addresses[0].InstanceId' --output text)"

INSTANCE_ID="$(aws_cmd ec2 describe-instances \
  --filters "Name=tag:Name,Values=$NODE_NAME" \
            'Name=instance-state-name,Values=pending,running,stopping,stopped' \
  --query 'Reservations[].Instances[].InstanceId | [0]' --output text)"
if [[ "$INSTANCE_ID" == "None" || -z "$INSTANCE_ID" ]]; then
  if [[ "$ASSOCIATED_INSTANCE" != "None" && -n "$ASSOCIATED_INSTANCE" ]]; then
    echo "EIP $PUBLIC_IP is associated with $ASSOCIATED_INSTANCE, but no managed node exists; refusing to move it." >&2
    exit 1
  fi

  USER_DATA="$(mktemp)"
  trap 'rm -f "$USER_DATA"' EXIT
  sed -e "s|__QUAXAR_REF__|$QUAXAR_REF|g" -e "s|__PUBLIC_IP__|$PUBLIC_IP|g" \
    "$SCRIPT_DIR/setup-testnet.sh" >"$USER_DATA"

  INSTANCE_ID="$(aws_cmd ec2 run-instances \
    --image-id "$AMI_ID" \
    --instance-type "$INSTANCE_TYPE" \
    --key-name "$KEY_NAME" \
    --network-interfaces "[{\"DeviceIndex\":0,\"AssociatePublicIpAddress\":false,\"SubnetId\":\"$SUBNET_ID\",\"Groups\":[\"$SG_ID\"]}]" \
    --block-device-mappings "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":$VOLUME_SIZE_GIB,\"VolumeType\":\"gp3\",\"DeleteOnTermination\":false,\"Encrypted\":true,\"Iops\":3000,\"Throughput\":125}}]" \
    --metadata-options 'HttpTokens=required,HttpEndpoint=enabled,HttpPutResponseHopLimit=1' \
    --user-data "file://$USER_DATA" \
    --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=$NODE_NAME},{Key=ManagedBy,Value=quaxar-infra},{Key=Network,Value=xrpl-testnet}]" \
                         "ResourceType=volume,Tags=[{Key=Name,Value=$NODE_NAME-nudb},{Key=ManagedBy,Value=quaxar-infra},{Key=Purpose,Value=nudb}]" \
    --query 'Instances[0].InstanceId' --output text)"
  aws_cmd ec2 wait instance-running --instance-ids "$INSTANCE_ID"
  aws_cmd ec2 associate-address --instance-id "$INSTANCE_ID" --allocation-id "$EIP_ALLOCATION_ID" >/dev/null
elif [[ "$ASSOCIATED_INSTANCE" != "$INSTANCE_ID" ]]; then
  echo "Existing managed instance $INSTANCE_ID does not own EIP $PUBLIC_IP; refusing to reassign it." >&2
  exit 1
fi

aws_cmd ec2 wait instance-status-ok --instance-ids "$INSTANCE_ID"
printf 'instance_id=%s\npublic_ip=%s\nsecurity_group=%s\neip_allocation_id=%s\n' \
  "$INSTANCE_ID" "$PUBLIC_IP" "$SG_ID" "$EIP_ALLOCATION_ID"
printf 'ssh_command=ssh -i ~/.ssh/quaxar-testnet-2026.pem ubuntu@%s\n' "$PUBLIC_IP"
printf 'bootstrap_log=/var/log/quaxar-bootstrap.log\n'
