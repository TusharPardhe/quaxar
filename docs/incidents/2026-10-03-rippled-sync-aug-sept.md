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
