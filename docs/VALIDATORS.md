# Running a Validator

A validator is a fully synchronized Quaxar server that signs and publishes
validations. Generating a key does not add the validator to another server's
trusted list, and appearing on a public explorer can lag network observation.

## Prerequisites

Before enabling validation, operate the node reliably in `full` state with a
stable public peer endpoint, accurate host time, sufficient storage, and
working validator-list publishers. Reliability means sustained canonical
closed-ledger and validated advancement, bounded lag, a coherent coordinator
phase, and no recurring Full/Syncing cycle. Keep RPC and WebSocket
administration on a private interface or SSH tunnel.

## Generate an identity

Run this once in a private directory:

```bash
umask 077
quaxar validator-keys generate
```

The command creates `validator-keys.json` with mode `0600` and refuses to
overwrite an existing file. It contains:

- `validation_public_key`: XRPL node-public key beginning with `n`
- `validation_private_key`: XRPL node-private key beginning with `p`
- `validation_seed`: family seed beginning with `s`
- `validation_key`: RFC-1751 words representing the seed material
- key type, format version, and creation time

The generated root identity uses secp256k1, matching the accepted XRPL
validator seed/public-key encoding. Treat the seed, private key, RFC-1751
words, and the JSON file as secrets. Never paste them into logs, issues, chat,
or source control.

## Configure validation

Store the generated `validation_seed` in the service config:

```ini
[validation_seed]
<secret-family-seed>
```

For the dedicated `User=quaxar` service described in [RUNNING.md](RUNNING.md),
ensure only the service account can read the file:

```bash
sudo chown root:quaxar /etc/quaxar/quaxar.cfg
sudo chmod 0640 /etc/quaxar/quaxar.cfg
sudo systemctl restart quaxar.service
```

The interactive Linux installer runs the service as the invoking user instead.
Confirm that account with `systemctl show -p User --value quaxar.service` and
retain config ownership/read permission for that user; do not blindly change
the group to `quaxar` if it does not exist.

Likewise, an upgraded host may still deliberately run `quaxar.service` under a
legacy account until ownership is migrated. Check the unit's actual `User` and
`Group`; never change the config or database owner while the service is
running.

Verify locally:

```bash
quaxar validator-info
quaxar server-info
```

When synchronized, a validating server normally reports `proposing` and
`server_info` exposes `pubkey_validator`. `full` is the healthy terminal state
for a server without an active validation identity. These labels are
point-in-time signals; confirm continued ledger advancement before treating the
validator as operational.

## Trust and explorer visibility

The public key identifies the validator; it is safe to publish. Other servers
only count its validations when their configured validator lists trust it.
Explorer visibility generally requires the explorer to observe validations and
may take several ledgers. UNL membership is separate and normally requires an
established operating history plus inclusion by a validator-list publisher.

Check:

```bash
quaxar peers
quaxar validator-info
quaxar unl-list
quaxar rpc server_info --raw
```

If `pubkey_validator` is absent, confirm the service is reading the intended
config and that `[validation_seed]` contains the family seed, not the public or
private key. If the server never reaches `proposing`, resolve synchronization,
quorum, validator-list, and clock issues before changing keys.

## Key files and backups

Keep at least one encrypted offline backup of `validator-keys.json`. A lost
root seed cannot be recovered. Do not reuse an identity across simultaneously
running validators, because they can issue conflicting validations.

The `validator-keys create-token`, `sign`, and `revoke` commands are currently
experimental developer tooling. Their token/manifest serialization and
rotation workflow are not documented as `rippled`-compatible for production.
Use `[validation_seed]` for the supported operational path until that
compatibility is independently audited.
