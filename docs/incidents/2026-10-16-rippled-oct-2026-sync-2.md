# rippled develop sync — October 2026 (part 2)

**Date:** 2026-10-16
**Branch:** `fix/app-integration-tests`
**Upstream range:** `XRPLF/rippled` `develop` `ede8af8191..4087861411` (35 commits after the
previous sync, `2026-10-09-rippled-oct-2026-sync.md`).

## Ported

| rippled | Title | Gate | quaxar |
|---|---|---|---|
| #8533 `b398b88385` | Reject zero credential ID in `deposit_authorized` | RPC | `bea59a9b` |
| #8568 `59ff04b648` | WASM transactions in `simulate` admin-only | RPC | `3600ce6d` |
| #8480 `1ed0bccb07` | `getBookPage` funded-amount overflow; `Quality::ceilOut` native clamp | RPC / fixCleanup3_5_0 | `32f9f02c`, `13ef9b5e` |
| #8583 `f1ce9449f2` | No bootstrap / fixed-peer DNS in standalone | operational | `1f1c36bd` |
| #8561 `33289e882b` | rpc-spec (C): validation precedence and marker parsing | RPC | `eb96a3b9` |
| #8226 `b010d4a563` | Retire fixAMMv1_1 (always on, amendment obsolete) | retired | `54139f72` |
| #8453 `fcb3ee6425` | Checked int64 XRPAmount / MPTAmount | fixCleanup3_5_0 | `f8f466f9` |
| #8523 `c908ed7839` | Malformed EscrowFinish condition is `temMALFORMED` | fixCleanup3_5_0 | `327cdfc1` |
| #8181 `791c96faad` | Pseudo-account owner-directory contents invariant | fixCleanup3_5_0 | `31c9437b` |
| #8211 `77eacbb52c` | MPT/DEX: Number `divRound` for MPT operands; safe issuance-max lookup | fixCleanup3_5_0 | `62254b01`, `af472c1c` |
| #8267 `7237b3321c` | Transfer fee on third-party vault withdrawals (live path + invariant) | fixCleanup3_5_0 | `fff2fa4b`, `6de5662e`, `73a0903b` |
| #8470 `84d980d883` | VaultClawback with nonexistent holder is `terNO_ACCOUNT` | fixCleanup3_5_0 | `9bc75dd7` |
| #8204 `aa70b368b5` | LoanBrokerDelete authorization, reserve order, holding creation, MPT cap | fixCleanup3_5_0 | `541df33b`, `5a1459cf` |
| #8231 `1da55e3989` | Waive CanTransfer on loan default / broker delete | fixCleanup3_5_0 | `61642b71` |
| #8082-#8086, #8476 | SHAMap/acquisition invalid-data hardening, header account hash on both routes | ungated | `c9d3a326` |

## Not ported

- MPTokensV2-only parts of #8211 and #8480 (OfferStream, BookStep MaximumAmount cap, OfferCreate
  gross-up): rippled marks MPTokensV2 `Supported::No`.
- LendingProtocolV1_2 FixedPrecision stack (#8257, #8258, #8260, #8271, #8460): `Supported::No`.
- Cosign / TransactionProposalCancel (#8538), SmartEscrow fee voting (#8214),
  IssuerMirror/InitialIssuer encryption keys (#8325, ConfidentialMPTKeyRotation): `Supported::No`.
- C++-only refactors and test infrastructure: TxQ refactor (#8064), `*Entry everywhere` (#8364),
  LedgerHistory/TaggedCache (#5874), SHAMapAddNode counts (#8080), acquisition seams (#8081),
  peer harness (#8310).

None of the ported consensus changes is active on testnet or mainnet: fixCleanup3_5_0 is not
enabled anywhere, and fixAMMv1_1 is already enabled on both networks.
