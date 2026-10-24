# rippled develop sync — October 2026 (part 3)

**Date:** 2026-10-23
**Branch:** `sync/rippled-oct-2026-part3`
**Upstream range:** `XRPLF/rippled` `develop` `4087861411..1f46f75318` (19 commits after
part 2), plus an audit of 2026-10-01..10-16 for items earlier syncs missed.

## Ported

| rippled | Title | Gate | Notes |
|---|---|---|---|
| #8274 `84e2a155b9` | Paginate account_lines/offers/channels past new owner-dir types | RPC | Missed by part 1. Markers are validated by checking the entry's key is on the owner-directory page named by one of its 9 `sf*Node` hints, so markers on Credentials and other new entry types are accepted. One shared helper replaces three copies. |
| #8527 `4c39b05f94` | Reject vault and lending amounts out of range for the asset | fixCleanup3_5_0 | Missed by part 2. Adds `Asset::is_representable` / `Asset::round_trips`. VaultCreate preflight `temMALFORMED`, VaultSet preclaim `tecPRECISION_LOSS`, LoanBrokerSet and LoanSet preclaim `tecPRECISION_LOSS`; out of range before the fix throws (`tefEXCEPTION`), as in rippled. |
| (pre-existing gap) | LoanSet preclaim value-field check | ungated | quaxar's LoanSet preclaim had no `STAmount{asset, v} != v` loop over PrincipalRequested, LoanOriginationFee, LoanServiceFee, LatePaymentFee, ClosePaymentFee. Added with #8527. |
| #8475 `dc64a8daac` | IOU rounding in the Clawback invariant | MPTokensV2 (rejects only) | Compares `after` with the rounded `before - claw`. |
| #8215 `bfbb440fc2` (TER part) | `temTEMP_DISABLED` → `tefTEMP_DISABLED`, new `tefEXCEEDS_LIMIT` | catalog | Appended at the end of their enums, so no existing code shifts. The SmartEscrow transactor work is not ported (`Supported::No`). |
| #8478 `12cb3932f3` | Vault share freeze at MPT strand endpoints | MPT strands (MPTokensV2) | `is_vault_pseudo_account_frozen` now matches rippled: gated on SingleAssetVault (quaxar used fixCleanup3_2_0) and with the issuer `sfVaultID` fallback for shares without `sfReferenceHolding` (missing before). |
| #8468 `7354816f65` | Assorted MPT/DEX offer fixes 1 | MPTokensV2 | Issuer-first MPT book records the MPT as a seen direct asset; overflow while consuming an offer is `tecINTERNAL` (tryConsumeOffer). |
| #7366 `025aa71930` | Retire fixFillOrKill | retired | Flow and OfferCreate FillOrKill checks are unconditional. Enabled on both networks. |
| #7357 `3fa2accfdf` | Retire AMM and fixAMMOverflowOffer | retired | `amm_enabled` and its parameter removed; AMM tx types no longer name an amendment; TrustSet `tecNO_DST` unconditional. |
| #7349 `a818f25203` | Retire XRPFees | retired | New ledgers and Fee pseudo-txs always use the drops fields; legacy fee voting, `fee_ref`, legacy validation fields removed. Legacy FeeSettings are still decoded for old ledgers. |
| #8255 `ef5064eeb9` | Remove XChainBridge | never enabled | 8 tx types (41–48 reserved), 3 ledger entry types, 21 fields, STI 25 (reserved), keylets, flags, transactors, preclaim/preflight, `ledger_entry`/`account_objects`/`ledger_data` selectors and `server_definitions` entries removed. XChainBridge and fixXChainRewardRounding stay registered as Obsolete. |
| #8561, #8573, #8613, #8620 | rpc-spec request validation | RPC | Ledger-selector format errors precede account errors in account_channels/currencies/info/lines/nfts/offers; limit and marker format precede the ledger lookup (account_lines/channels); `queue` and `ignore_default` read as bools; account_info reports actNotFound before queue/signer_lists errors and keeps account_data on the v2 signer_lists error. nft_buy/sell_offers and deposit_authorized already matched rippled's new tests. |
| #6446 `86d8b244d6` (missed, July) | Batch inner transactions are never validly signed | ungated | `checkValidity` returns SigBad and caches it for any tfInnerBatchTxn; the fixBatchInnerSigs-era branch is removed. |
| (registry audit) | Amendment registry matches rippled develop | registry | All retired amendments Obsolete; Cosign registered unsupported; seven quaxar-only entries rippled no longer knows removed (113 names, same as rippled). |
| (missed) | SponsorshipSet / SponsorshipTransfer flag maps | definitions | Added to the transaction flag catalog. |
| (missed) | MantissaScale names | basics | `Small`, `LargeLegacy`, `Large320`, `Large330`, as rippled `to_string`. |

## Not ported

- Supported::No features: Cosign invariants (#8130), SmartEscrow transactors (#8215).
- C++-only: Number layout (#8550), CI/build/chore commits.

## Testing

- New tests: owner-dir marker coverage and Credential pagination (fails before the
  fix), asset representability, LoanSet/LoanBrokerSet/VaultSet/VaultCreate range checks,
  clawback rounding (fails before the fix), vault-share freeze inheritance (SAV gate and
  legacy fallback), issuer-first MPT book loop.
- Full workspace: 16,721 passed, 0 failed. All 58 repaired mainnet/testnet captures
  replay byte-exact.
- Three tests that also failed on `main` are fixed: two app fixtures behind current ledger
  wiring, and a basics test that ignored the `jemalloc` feature enabled by workspace builds.
