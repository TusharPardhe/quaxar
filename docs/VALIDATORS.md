# Validator Setup

## What is a Validator

A validator on the XRP Ledger participates in the consensus process by proposing transaction sets and voting on ledger closings. Validators that are trusted by other nodes (included in their UNL) directly influence which transactions are confirmed.

Running a validator requires:
- A reliable, well-connected node that stays synced
- A validator key pair (master + ephemeral)
- Uptime commitment (validators that go offline lose trust)

## Key Generation

Generate a new validator master key pair:

```bash
xrpld validator-keys generate
```

Output:
```
Master Public Key:  nHUFE9prPXPrHcG3SkwP1UzAQbSphqyQkQK9ATXLZsfkezRda2Hm
Master Secret Key:  paQmjZ37pKKPMrgadBLsuf9ab7Y7EUNzh27LQrZqoexpAs31nJi

Store the master secret key securely. It cannot be recovered.
```

**Important:** Store the master secret key offline in a secure location. It is used only to create tokens and for key rotation.

## Creating a Token

Generate a validator token from the master key. The token contains an ephemeral key pair that the running node uses for signing:

```bash
xrpld validator-keys create-token
```

You will be prompted for the master secret key. Output:

```
[validator_token]
eyJ0eXAiOiJKV1QiLCJhbGciOiJFZERTQSJ9...
```

## Adding Token to Config

Add the token to your node's configuration file:

```ini
[validator_token]
eyJ0eXAiOiJKV1QiLCJhbGciOiJFZERTQSJ9...
```

Restart the node to activate. Verify with:

```bash
xrpld validator-keys show
```

## Key Rotation

Validators use a two-level key scheme:

- **Master key** — Long-lived identity. Used offline to sign manifests.
- **Ephemeral key** — Short-lived signing key embedded in the token. Used by the running node.

To rotate keys:
1. Generate a new token with `validator-keys create-token` (increments the manifest sequence)
2. Replace the `[validator_token]` in config
3. Restart the node
4. The new manifest propagates to peers automatically
5. The old ephemeral key is invalidated

The master public key remains your validator's identity across rotations.

## Revoking Keys

If your master key is compromised, revoke it permanently:

```bash
xrpld validator-keys revoke
```

This publishes a maximum-sequence manifest that permanently disables the validator identity. **This action is irreversible.** You will need to generate a new master key and establish trust from scratch.

## Getting on the UNL

The default UNL is published by the XRP Ledger Foundation. To be considered:

1. Run a reliable validator with 99%+ uptime for several months
2. Demonstrate operational competence and security practices
3. Apply through the XRPLF validator application process
4. Maintain geographic and jurisdictional diversity

Operators can also add your validator to their own `[validators]` section independently of the default UNL.

## Security Best Practices

- **Air-gapped key generation** — Generate master keys on an offline machine. Never expose the master secret to a networked system.
- **Separate signing server** — If possible, run the validator on a dedicated machine with no other services.
- **Token-only on server** — Only the token (ephemeral key) should exist on the running node. The master secret stays offline.
- **Regular rotation** — Rotate ephemeral keys periodically (every 3–6 months).
- **Monitor manifests** — Watch for unexpected manifest publications that could indicate compromise.
- **Firewall** — Restrict admin RPC port to localhost. Only expose the peer port (51235) publicly.
- **Revocation plan** — Have a documented procedure to revoke keys quickly if compromise is suspected.
