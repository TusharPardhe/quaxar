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

Do not enable validation merely because initial acquisition filled the expected
RAM or briefly reported `full`. Confirm several consecutive local closed hashes
belong to the network's validated chain and that publication continues without
mode churn.

## Generate an identity

Run this once in a private directory:

```bash
umask 077
quaxar validator-keys generate
```

On supported Unix platforms, the command creates `validator-keys.json` with
mode `0600` and refuses to overwrite an existing file. It contains:

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

Record the generated public `n...` identity separately if it is needed for
operator registration. Never copy the private key, seed, RFC-1751 words, or
complete JSON file into documentation or logs. Validate the protected config
before restarting:

```bash
quaxar --conf /etc/quaxar/quaxar.cfg config
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

## Domain verification

Explorers and the Validator History Service (VHS) show a domain as verified
only when two things agree (the same check as rippled's validator-keys tool):

| Where | What | Who signs |
| --- | --- | --- |
| Manifest (`[validator_token]`) | `sfDomain = <domain>` | master key |
| `https://<domain>/.well-known/xrp-ledger.toml` | `[[VALIDATORS]]` with `public_key` and `attestation` | master key |

The attestation is the master key's signature, as upper-case hex, over the
exact string `[domain-attestation-blob:<domain>:<master public key>]`.

IMPORTANT: a domain can only be carried by a manifest, so the validator must
run with `[validator_token]` instead of `[validation_seed]`. The master public
key (the validator identity) does not change. Each `set-domain` raises the
token sequence; peers keep the highest sequence they have seen.

```bash
cd <directory holding validator-keys.json>   # ideally an offline machine
quaxar validator-keys set-domain example.com
```

The command backs up the key file to `validator-keys.json.bak`, records
`domain` and `token_sequence`, and prints the attestation and the new token.
Then:

1. Publish the TOML over HTTPS with `Access-Control-Allow-Origin: *`
   (`Content-Type: application/toml` is recommended).
2. Back up the config, remove `[validation_seed]`, add `[validator_token]`,
   run `quaxar --conf <cfg> config`, and restart.
3. Check `quaxar validator-info` shows the domain and the new sequence.

Once the token is in place, the master seed no longer needs to be on the
validator. Keep `validator-keys.json` offline; it is needed for future token
rotation and for revocation.

## Key files and backups

Keep at least one encrypted offline backup of `validator-keys.json`. A lost
root seed cannot be recovered. Do not reuse an identity across simultaneously
running validators, because they can issue conflicting validations.

`validator-keys create-token` and `set-domain` build manifests the same way as
rippled's validator-keys tool (`createValidatorToken`). The `sign` and `revoke`
commands are still experimental developer tooling.
