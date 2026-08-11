# AWS NuDB and Quaxar artifact reset plan

**Status: prepared only. This document authorizes no action.**

No service has been stopped, no database data has been removed, no binary has been replaced, and no AWS resource has been modified. A destructive reset remains blocked until the operator explicitly confirms the exact outage and data-loss scope in the confirmation template below.

## Read-only inventory

| Item | Verified value |
| --- | --- |
| AWS region | `eu-west-2` |
| Instance | `i-0b7eadfd52c8cdbdc` |
| AMI | `ami-01029182857b20417` |
| Host | `ubuntu@35.179.201.81` |
| Service | `quaxar.service`, active |
| Service user | `xrpld:xrpld` |
| Running command | `/usr/local/bin/quaxar --conf /etc/quaxar/xrpld.cfg` |
| NodeStore root | `/var/lib/xrpld/db/nudb` |
| Relational database root | `/var/lib/xrpld/db` |
| Installed binary | `/usr/local/bin/quaxar` |
| Rollback binary | `/usr/local/lib/quaxar-rollbacks/quaxar-before-2594236` |
| OCI images | No Docker or Podman images found |

## Data and artifact inventory

`/var/lib/xrpld` is approximately 30 GiB. NuDB is approximately 28 GiB.

- `/var/lib/xrpld/db/nudb/xrpldb.0000/nudb.dat`: 27,330,175,992 bytes
- `/var/lib/xrpld/db/nudb/xrpldb.0000/nudb.key`: 1,274,212,352 bytes
- `/var/lib/xrpld/db/nudb/xrpldb.0000/nudb.log`: 693,111,218 bytes
- `/var/lib/xrpld/db/transaction.db`: approximately 1.95 GiB
- `ledger.db`, `ledger_headers.db`, WAL/SHM files, `peerfinder.db`, and `state.db` are present under `/var/lib/xrpld/db`.

Recorded binary hashes:

```text
/usr/local/bin/quaxar
b05b0fcbbcfcb8e8becac1a993f382bba3731c4a347f55db58d4933d838f0226

/usr/local/lib/quaxar-rollbacks/quaxar-before-2594236
2699cc3a38c3efaf3de4f039ef31b055874166d97a8a1695de4d53a697b68dd8
```

## Destructive impact

Removing NuDB destroys locally retained ledger and SHAMap node data. Removing the relational files may destroy transaction, ledger-header, peerfinder, and state metadata. Replacing the installed binary changes the executable used by the active systemd service. These operations are reversible only to the extent that explicitly retained backups and rollback artifacts are valid.

The phrase **“Quaxar image” is ambiguous**. Inventory found no OCI image. It could mean the installed binary, a local build artifact, an OCI image, an AMI, an EBS snapshot, or another artifact. The operator must identify the exact artifact before any removal or replacement.

## Required explicit confirmation

Before a reset, provide all of the following:

1. Approved outage window and permission to stop and restart `quaxar.service`.
2. Exact NuDB paths to remove, if any.
3. Exact relational database paths to remove, if any.
4. Whether `/etc/quaxar/xrpld.cfg`, logs, and the rollback binary must remain.
5. Exact meaning and location of any “Quaxar image” to remove or replace.
6. Explicit acknowledgement that the named data will be permanently lost.
7. Whether a pre-reset archive/snapshot is required and its destination.

Use this template:

```text
I approve an outage on i-0b7eadfd52c8cdbdc during <window>.
You may stop and restart quaxar.service.
Remove exactly: <absolute paths>.
Preserve exactly: <config/log/rollback/archive paths>.
“Quaxar image” means: <exact artifact and path/identifier>.
I acknowledge permanent data loss for the named paths.
Create a pre-reset archive: <yes/no and destination>.
```

## Safe execution order after approval

1. Reconfirm the instance identity, active service, configured paths, free disk space, and backup destination.
2. Capture final service status, binary hashes, configuration checksum, and database inventory.
3. Create the approved archive or snapshot and verify it before deletion.
4. Stop `quaxar.service` only inside the approved outage window.
5. Remove only the confirmed paths; do not use broad deletion patterns.
6. Install or rebuild only the confirmed binary/artifact while retaining the specified rollback binary.
7. Recheck ownership and configuration, then start `quaxar.service`.
8. Verify service health, logs, listener state, sync state, storage recreation, and rollback readiness.

Until the confirmation template is complete, this plan is documentation only.
