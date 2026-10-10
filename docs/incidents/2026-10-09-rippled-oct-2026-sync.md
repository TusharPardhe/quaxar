# rippled develop sync — October 2026

**Date:** 2026-10-09
**Branch:** `sync/rippled-oct-2026` (off `main` at v0.9.0)
**Upstream range:** `XRPLF/rippled` `develop` `a9027bb9..ede8af8191` (36 commits).

This records the port of the behavioural changes merged into rippled `develop`
since the previous sync (`sync/rippled-aug-sept-2026`), plus an amendment-flag
correction found while auditing mainnet amendment coverage.

## Classification of the 36 commits

| Type | Count | Action |
|---|---|---|
| feat / fix (behavioural) | 9 | Port (8 done, 1 deferred — see below) |
| refactor (C++ internal: `*Entry everywhere`, rpc-spec migration, `std::format` throws, binary grouping) | 11 | Skip — no behaviour change |
| build (LLVM/Clang 23, conan, nix, sanitizer defs) | 5 | Skip — C++ toolchain only |
| chore (images, hooks, ignore-revs, test splitting) | 5 | Skip |
| test (gtest migration, extra permission/SLE tests) | 3 | Skip — upstream test infra |
| **Total** | **36** | |

## Behavioural commits

| rippled | Title | quaxar status | Notes |
|---|---|---|---|
| #7583 `c2a4bc3aa1` | Validate `vetoed` param type in feature RPC | **Ported** | `feature` RPC now returns `invalidParams` for a non-boolean `vetoed`; removed the coercing `json_value_as_bool`. |
| #7582 `40f61f828a` | String type validation for `channel_id`/`signature` | **Ported** | `channel_verify` returns `invalidParams` for non-string fields; added `parse_required_string_field`. |
| #8244 `63c97e719f` | Allow lending transactions in Batch | **Ported** | SAV/Lending inner-type rejection now gated on `LendingProtocolV1_2`; threaded the flag through the rules-aware preflight path. |
| #8152 `b8d8738f81` | Enforce MPT issuance flags never cleared | **Ported** | Accumulate cleared issuance flags (excluding `lsfMPTLocked`); fail `ValidMPTIssuance` under `fixCleanup3_5_0`. |
| #8209 `f1744cb76e` | Don't block MPToken deletion on unrelated confidential balances | **Ported** | Split public-balance from ciphertext in the confidential gate; public-balance erase moves to `ValidMPTIssuance` under `fixCleanup3_5_0`; delete flags are set-only. |
| #8330 `bcbaa4df07` | Enable large Number mantissa with MPTokensV2 | **Ported** | `MPTokensV2` joins the large-mantissa and rules-guard predicates. |
| #8302 `60195e6d37` | Fix MPT partial payment overflow | **Ported** | Overflow-safe integral `mul_ratio` MPT transfer math under `fixCleanup3_5_0` (cost rounded up, delivery rounded down, overflow = exceeds SendMax); legacy path kept pre-amendment. |
| #6319 `3ac26f23c7` | Full `ledger_entry` object support | **Ported** | Structured selectors for Check `{account,seq}`, NFTokenOffer `{owner,seq}`, PayChannel `{account,destination,seq}`, SignerList `{account}`; raw hex ids still accepted. |
| #8266 `c45363fd8b` | Confidential MPT holder key update | **Deferred** | See below. |

All ported changes keep their rippled amendment gate, so behaviour is identical
on a ledger where the amendment is not enabled.

## Deferred: #8266 Confidential MPT holder key update

This is a **new feature**, not a parity fix, and is deliberately not shipped in
this sync. It requires:

- A new transaction type (`ttCONFIDENTIAL_MPT_HOLDER_KEY_UPDATE` = 93,
  `ConfidentialMPTHolderKeyUpdate`).
- A new serialized field `sfRecoveryKey` (VL, code 48) and new tx flags.
- A new amendment `ConfidentialMPTKeyRotation`, which quaxar does not register.
- A full transactor (preflight / preclaim / apply with key-rotation, recovery
  and cancel-recovery modes) and `ValidMPTIssuance` integration.
- The `ConfidentialTransfer` base feature, which quaxar currently holds
  `supported = false` pending full confidential-subsystem certification.

Shipping a partial confidential transaction type on top of an unsupported
confidential base would add consensus-relevant surface that cannot be certified
in this change. It is tracked as the single remaining item from this range.

## Amendment-coverage correction

A mainnet amendment audit (95 enabled, 20 voting as of 2026-10-08) found quaxar
supported every enabled amendment except **`PermissionDelegationV1_1`**, which
was registered `supported = false` despite the DelegateSet lifecycle,
delegated signing, fee payment and permission checks all being implemented. It
is now `supported = true`.

The other apparent gaps were phantom and were **not** registered: `Batch`,
`PermissionDelegation` and `fixDelegateV1_1` are superseded by the `V1_1`
variants quaxar already carries, and `OwnerPaysFee` is not a current rippled
amendment.

## Testing

- `cargo check --workspace --tests` compiles clean.
- New tests: feature `vetoed` type rejection, channel_verify non-string
  rejection, Batch lending-inner gate, MPT issuance-flag-cleared invariant,
  and ledger_entry structured selectors.
- Pre-existing failures confirmed identical on `origin/main` (not regressions):
  `rpc rpc_handler_registry_table_plus_expected_aliases`, `tx
  utility::parity::forced_validity_flags_match_current_cpp_forcevalidity_promotion`,
  `tx utility::parity::merge_forced_validity_matches_hash_router_cache_promotion_rule`,
  and `tx batch_sttx_policy::...rule_aware_inner_delegate_validation`.

## Pre-merge audit (2026-10-10)

The testnet node running the pre-sync binary was amendment blocked and held
at `connected` (rippled caps a blocked server at `connected`). Two enabled
testnet amendments were unsupported: `PermissionDelegationV1_1` (flag fixed
above) and **`fixBatchV1_2`**, which shipped in rippled 3.4.1 after this
sync's upstream range. The audit added:

- **`fixBatchV1_2`** (`19c94c73f4`): registered supported/DefaultYes; a Batch
  inner not wrapped in a `RawTransaction` object is `temMALFORMED`, checked
  first in the inner loop.
- **#8302 completion**: the first port changed only the direct Payment
  quote. The transit debit (`directSendNoLimitMPT`) now uses the exact
  round-up `mulRatio` cost under `fixCleanup3_5_0`, so quote and debit agree,
  and MPT `divRound` takes the Number path under `fixCleanup3_5_0` with
  rippled's no-rules default of enabled. The multi-receiver send needs no
  change: both lending callers waive the transfer fee.

Still outstanding from rippled 3.4.1: `578224f2e6` (ungated integer-overflow
hardening in StrandFlow, Steps accumulators and TokenHelpers send loops).
It changes results only on arithmetic overflow and is tracked as the next
sync item.
