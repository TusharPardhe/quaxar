# Rippled Aug–Sept 2026 Sync Audit & Implementation

Branch: `sync/rippled-aug-sept-2026`. rippled ref develop @ a9027bb997 (2026-10-02).
Goal: check each consensus-relevant commit 1-by-1; implement the missing ones.

Status legend: PRESENT (already in our code) | MISSING (ported here) |
NA-feature (amendment/feature not implemented in quaxar; out of scope) |
NA-noncore (RPC/peer-metric/C++-isms with no consensus effect; noted).

## Verified before this branch (prior audit)
- #7429 MPT STIssue endianness: PRESENT
- #7430 AMM auction slot zero-cost: PRESENT
- #7704 AMM zero clawback MPT rounding: PRESENT
- #7940 verifyProofPath inner-at-leaf: PRESENT
- #7941 selectBranch depth clamp: PRESENT
- #7942 traversal node id from descended branch: PRESENT
- STPathElement hardened hash: PARTIAL (DoS-hardening, non-consensus)
- #8c594c7 CheckCash issuer limit-waiver skip: MISSING (to implement)

## 1-by-1 audit log
(appended as each commit is checked)

### Batch 1 (#1-#17)
- #1 8461ded Cap untrusted manifests per message: PRESENT (overlay_impl.rs:619-666 max_untrusted_manifests + "too many untrusted manifests").
- #5 54cfdda Increase manifest size cap + fix relay: PRESENT (max_manifests_message_size + "oversized manifest batch").
- #2 97f35add Null check account object reads: NA-noncore (RPC AccountObjects robustness; non-consensus).
- #4 b8451ff JSON iterators value_type: SKIP (C++ism, no Rust equivalent).
- #7913 abf5511 sign-check wording lending msgs: SKIP (message wording only).
- #37 c5dc408 Remove explicit from hash ctors: SKIP (C++ism).
- #11 b19c3c6 #7971 Add zero keylet check in credential: PRESENT (credential_helpers.rs:223,437 fix_cleanup_3_4_0 && h.is_zero -> TEC_INTERNAL).

### Implemented on this branch
- #8254 0219c01b SponsorshipTransfer zero sfObjectID -> TEM_MALFORMED (fixCleanup3_5_0). DONE.
- 00eeb0 decode_vl_length_1 reject first byte >192. DONE.
- #8111 f8fba07 VaultClawback pseudo-holder -> TEC_PSEUDO_ACCOUNT (fixCleanup3_4_0). DONE.

### Reclassified after close review
- #7796 636d2d4 AMMClawback IgnoreReserve: PRESENT. Our amm_clawback_ignores_recipient_reserve
  (transactor_dispatcher.rs:410) + amm_prepare_withdraw_holding reserve bypass (clawback_issuer
  present && fixCleanup3_4_0) is the ReserveHandling::IgnoreReserve equivalent. Audit grep
  false-negative (searched literal "IgnoreReserve").
- c8e767 PaymentBurn cross-zero reject (fixCleanup3_4_0): DONE. Granular
  PaymentBurn now requires dstAmount <= held (or mayIssue) so redeeming cannot
  cross zero into minting. payment.rs facts + invoke_preclaim builder.

## FINAL STATUS (all 90 audited)

### Implemented on branch sync/rippled-aug-sept-2026 (9 legit missing ports)
1. #8254  0219c01b SponsorshipTransfer zero sfObjectID -> TEM_MALFORMED (fixCleanup3_5_0)
2.        00eeb0a0 decode_vl_length_1 reject first byte > 192 (vl-prefix encoder/decoder parity)
3. #8111  f8fba079 VaultClawback pseudo-holder -> TEC_PSEUDO_ACCOUNT (fixCleanup3_4_0)
4.        c8e767af PaymentBurn cross-zero reject (fixCleanup3_4_0)
5.        796f2f8f Loan invariant: allow non-final zero-principal LoanPay
6.        a18839d9 MPT authorize-cap exception for LoanSet + VaultWithdraw (+test)
7. #8141  b3b38e44 ValidVault fee-payer XRP delta for sponsored VaultWithdraw (+test)
8.        53628b70/eae0a354 STPathElement seeded hardened dedup hash (non-consensus)
9. overlay: 6099940c TMGetLedger bound; 9aebb5eb TMTransactions cap; e302e4ee #8220 peer
   limit total; 7e82b066 #8309 queue tx to requesting peers; 54cfdda0 never truncate
   trusted manifests for relay. (each +regression test)

### SKIPPED per user steering (not legit Rust ports / keep 1:1 only where meaningful)
- NA-CPPISM: b8451ff JSON iterator value_type; c5dc408 remove `explicit`; 4f88195 assorted
  cleanup; ddbc5f1a IntrusivePointer leak; 0db7b766 protobuf DiscardUnknownFields (prost
  discards unknown fields automatically); abf5511 message wording.
- NA-RPC (non-consensus, RPC-handler-only robustness, already Option/Result-safe in Rust):
  97f35add account-object null; 798e889e oracle dedup; 4173f7e4 account_lines peer type;
  639943 nft buy/sell flag; d43e5ac gateway_balances type; 1a4a40eb noripple_check;
  8f4e9c25 CTID in ledger expanded txns; 04eca6d6 book_offers running-balance rounding.
- NA-NONCORE (telemetry): 768aef30 + 53246e5b cluster-traffic counting.
- NA-FEATURE (amendment/tx-type not implemented in quaxar): d5bfe94f/7f55dd39/8b1a2282
  ConfidentialMPT key rotation + MirrorUpdate; 028783 SmartEscrow .macro; 646d2ce6 Cosign v1
  TransactionProposalCreate. (feature.rs registers these as unsupported, matching intent.)

### Confirmed ALREADY-PRESENT (not reclassified above) : all remaining ~60 commits
Verified present with file:line evidence in the batch-A/B/C audit (SHAMap #7940/#7941/#7942,
MPT STIssue #7429, AMM #7430/#7704/#7373, all Vault/Lending consensus fixes #7863/#8004/#8013/
#8014/#8055/#8057/#8075/#8119/#8140/#8143/#8144/#8151/#8153/#8154, credential #7971/#6827,
escrow #8142, sig-prefixes #8162, calculateBaseFee #3e4e56, simulate dry-run #ea6226,
amendment registrations #8125/#8174/#8185, manifest caps #8461ded, etc.), plus #7977 vault
withdrawal destination checks and #7796 AMMClawback IgnoreReserve (reclassified PRESENT).
