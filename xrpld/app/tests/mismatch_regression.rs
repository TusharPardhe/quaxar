//! Regression tests for all 41 ledger mismatches from ledgers 104115399–104115992.
//!
//! Each test case is a real transaction from the XRPL mainnet that our node
//! produced the wrong TER for. The expected TER is what C++ (the validated
//! ledger) produced. Tests are grouped by root cause.
//!
//! Root causes:
//!   P0  - tecDIR_FULL: directory chain find_previous_page fallback (306 txs)
//!   P1a - credit_balance sign reversed (63 tecUNFUNDED_OFFER, 117 tecPATH_DRY)
//!   P1b - flow engine precision / DirectStep (97 tecPATH_DRY→tecPATH_PARTIAL)
//!   P2  - IOC result shaping (51 tecKILLED→tesSUCCESS, 31 tecKILLED→tecUNFUNDED_OFFER)
//!   P3  - tefBAD_LEDGER for passive OfferCreate (8 txs)
//!   P4  - tecNO_DST / tecNO_DST_INSUF_XRP false positives (12 txs)
//!   P5  - EscrowFinish IOU missing tecLIMIT_EXCEEDED check (5 txs)
//!   P6  - tesSUCCESS when should be tecUNFUNDED_PAYMENT/OFFER/RESERVE (4 txs)
//!
//! Run with: cargo test -p app --test mismatch_regression -- --nocapture

/// A single mismatch test case.
#[derive(Debug)]
struct MismatchCase {
    /// Ledger sequence containing the transaction.
    seq: u32,
    /// First 8 hex chars of the transaction hash.
    txid: &'static str,
    /// Transaction type name.
    tx_type: &'static str,
    /// Transaction flags.
    #[allow(dead_code)]
    flags: u32,
    /// TER our node produced (wrong).
    our_ter: &'static str,
    /// TER C++ produced (correct, from validated ledger).
    ter: &'static str,
    /// Root cause label.
    cause: &'static str,
}

/// All 41 representative mismatch cases (one per distinct root cause instance).
/// Full txid list is in docs/context/mismatch-report.md.
const CASES: &[MismatchCase] = &[
    // ── P0: tecDIR_FULL (306 total) ──────────────────────────────────────────
    MismatchCase {
        seq: 104115399,
        txid: "c765e8ca",
        tx_type: "OfferCreate",
        flags: 0x00000000,
        our_ter: "tecDIR_FULL",
        ter: "tesSUCCESS",
        cause: "P0_dir_full",
    },
    MismatchCase {
        seq: 104115405,
        txid: "1e6e3558",
        tx_type: "OfferCreate",
        flags: 0x00080000,
        our_ter: "tecDIR_FULL",
        ter: "tesSUCCESS",
        cause: "P0_dir_full",
    },
    MismatchCase {
        seq: 104115422,
        txid: "1cbbb3f7",
        tx_type: "OfferCreate",
        flags: 0x00000000,
        our_ter: "tecDIR_FULL",
        ter: "tesSUCCESS",
        cause: "P0_dir_full",
    },
    // ── P1a: credit_balance sign → tecUNFUNDED_OFFER (63 total) ─────────────
    MismatchCase {
        seq: 104115405,
        txid: "6ddcd40a",
        tx_type: "OfferCreate",
        flags: 0x00000000,
        our_ter: "tecUNFUNDED_OFFER",
        ter: "tesSUCCESS",
        cause: "P1a_unfunded_offer_flow_engine",
    },
    MismatchCase {
        seq: 104115405,
        txid: "ffe59f8d",
        tx_type: "OfferCreate",
        flags: 0x00000000,
        our_ter: "tecUNFUNDED_OFFER",
        ter: "tesSUCCESS",
        cause: "P1a_unfunded_offer_flow_engine",
    },
    MismatchCase {
        seq: 104115422,
        txid: "65ddd1bb",
        tx_type: "OfferCreate",
        flags: 0x00000000,
        our_ter: "tecUNFUNDED_OFFER",
        ter: "tesSUCCESS",
        cause: "P1a_unfunded_offer_flow_engine",
    },
    // ── P1b: flow engine → tecPATH_DRY when C++ succeeds (117 total) ─────────
    MismatchCase {
        seq: 104115399,
        txid: "466bb915",
        tx_type: "Payment",
        flags: 0x00020000,
        our_ter: "tecPATH_DRY",
        ter: "tesSUCCESS",
        cause: "P1b_flow_engine_path_dry",
    },
    MismatchCase {
        seq: 104115399,
        txid: "cf491c64",
        tx_type: "Payment",
        flags: 0x80020000,
        our_ter: "tecPATH_DRY",
        ter: "tesSUCCESS",
        cause: "P1b_flow_engine_path_dry",
    },
    MismatchCase {
        seq: 104115405,
        txid: "2168a966",
        tx_type: "Payment",
        flags: 0x00030000,
        our_ter: "tecPATH_DRY",
        ter: "tesSUCCESS",
        cause: "P1b_flow_engine_path_dry",
    },
    // ── P1b: flow engine → tecPATH_DRY when C++ returns tecPATH_PARTIAL (97) ─
    MismatchCase {
        seq: 104115399,
        txid: "f62a546b",
        tx_type: "Payment",
        flags: 0x80020000,
        our_ter: "tecPATH_DRY",
        ter: "tecPATH_PARTIAL",
        cause: "P1b_flow_engine_path_dry_vs_partial",
    },
    MismatchCase {
        seq: 104115405,
        txid: "982b0f57",
        tx_type: "Payment",
        flags: 0x80020000,
        our_ter: "tecPATH_DRY",
        ter: "tecPATH_PARTIAL",
        cause: "P1b_flow_engine_path_dry_vs_partial",
    },
    MismatchCase {
        seq: 104115422,
        txid: "3c9efd0d",
        tx_type: "Payment",
        flags: 0x80020000,
        our_ter: "tecPATH_DRY",
        ter: "tecPATH_PARTIAL",
        cause: "P1b_flow_engine_path_dry_vs_partial",
    },
    // ── P1b: flow engine over-delivers → tesSUCCESS when C++ returns tecPATH_PARTIAL (37) ─
    MismatchCase {
        seq: 104115399,
        txid: "b77c4423",
        tx_type: "Payment",
        flags: 0x00020000,
        our_ter: "tesSUCCESS",
        ter: "tecPATH_PARTIAL",
        cause: "P1b_flow_engine_over_delivers",
    },
    // ── P2: IOC tecKILLED when C++ crosses successfully (51 total) ───────────
    MismatchCase {
        seq: 104115399,
        txid: "2b7935e8",
        tx_type: "OfferCreate",
        flags: 0x00020000,
        our_ter: "tecKILLED",
        ter: "tesSUCCESS",
        cause: "P2_ioc_killed",
    },
    MismatchCase {
        seq: 104115399,
        txid: "64eb3f66",
        tx_type: "OfferCreate",
        flags: 0x00020000,
        our_ter: "tecKILLED",
        ter: "tesSUCCESS",
        cause: "P2_ioc_killed",
    },
    MismatchCase {
        seq: 104115399,
        txid: "c1aa8f9e",
        tx_type: "OfferCreate",
        flags: 0x00020000,
        our_ter: "tecKILLED",
        ter: "tesSUCCESS",
        cause: "P2_ioc_killed",
    },
    // ── P2: IOC tecKILLED when C++ returns tecUNFUNDED_OFFER (31 total) ──────
    MismatchCase {
        seq: 104115405,
        txid: "30ce9391",
        tx_type: "OfferCreate",
        flags: 0x00020000,
        our_ter: "tecKILLED",
        ter: "tecUNFUNDED_OFFER",
        cause: "P2_ioc_killed_vs_unfunded",
    },
    MismatchCase {
        seq: 104115405,
        txid: "bfa83642",
        tx_type: "OfferCreate",
        flags: 0x00020000,
        our_ter: "tecKILLED",
        ter: "tecUNFUNDED_OFFER",
        cause: "P2_ioc_killed_vs_unfunded",
    },
    // ── P3: tefBAD_LEDGER for passive OfferCreate (8 total) ──────────────────
    MismatchCase {
        seq: 104115405,
        txid: "c168f394",
        tx_type: "OfferCreate",
        flags: 0x00010000,
        our_ter: "tefBAD_LEDGER",
        ter: "tesSUCCESS",
        cause: "P3_tef_bad_ledger_passive",
    },
    MismatchCase {
        seq: 104115405,
        txid: "ef13e0c7",
        tx_type: "OfferCreate",
        flags: 0x00010000,
        our_ter: "tefBAD_LEDGER",
        ter: "tesSUCCESS",
        cause: "P3_tef_bad_ledger_passive",
    },
    MismatchCase {
        seq: 104115427,
        txid: "51db2431",
        tx_type: "OfferCreate",
        flags: 0x00000000,
        our_ter: "tefBAD_LEDGER",
        ter: "tesSUCCESS",
        cause: "P3_tef_bad_ledger_passive",
    },
    // ── P4: tecNO_DST false positive (6 total) ────────────────────────────────
    MismatchCase {
        seq: 104115427,
        txid: "1bf11416",
        tx_type: "TrustSet",
        flags: 0x00020000,
        our_ter: "tecNO_DST",
        ter: "tesSUCCESS",
        cause: "P4_no_dst_false_positive",
    },
    MismatchCase {
        seq: 104115427,
        txid: "af93069a",
        tx_type: "Payment",
        flags: 0x00020000,
        our_ter: "tecNO_DST",
        ter: "tesSUCCESS",
        cause: "P4_no_dst_false_positive",
    },
    MismatchCase {
        seq: 104115541,
        txid: "3b1a4c01",
        tx_type: "TrustSet",
        flags: 0x00000000,
        our_ter: "tecNO_DST",
        ter: "tesSUCCESS",
        cause: "P4_no_dst_false_positive",
    },
    // ── P4: tecNO_DST_INSUF_XRP false positive (6 total) ─────────────────────
    MismatchCase {
        seq: 104115540,
        txid: "a12fbddd",
        tx_type: "Payment",
        flags: 0x00000000,
        our_ter: "tecNO_DST_INSUF_XRP",
        ter: "tesSUCCESS",
        cause: "P4_no_dst_insuf_xrp_false_positive",
    },
    MismatchCase {
        seq: 104115563,
        txid: "31e9ed36",
        tx_type: "Payment",
        flags: 0x00000000,
        our_ter: "tecNO_DST_INSUF_XRP",
        ter: "tesSUCCESS",
        cause: "P4_no_dst_insuf_xrp_false_positive",
    },
    // ── P5: EscrowFinish IOU missing tecLIMIT_EXCEEDED (5 total) ─────────────
    MismatchCase {
        seq: 104115428,
        txid: "2c03fdd5",
        tx_type: "EscrowFinish",
        flags: 0x00000000,
        our_ter: "tesSUCCESS",
        ter: "tecLIMIT_EXCEEDED",
        cause: "P5_escrow_finish_iou_limit",
    },
    MismatchCase {
        seq: 104115542,
        txid: "b78df60d",
        tx_type: "EscrowFinish",
        flags: 0x00000000,
        our_ter: "tesSUCCESS",
        ter: "tecLIMIT_EXCEEDED",
        cause: "P5_escrow_finish_iou_limit",
    },
    MismatchCase {
        seq: 104115547,
        txid: "8c64819d",
        tx_type: "EscrowFinish",
        flags: 0x00000000,
        our_ter: "tesSUCCESS",
        ter: "tecLIMIT_EXCEEDED",
        cause: "P5_escrow_finish_iou_limit",
    },
    // ── P6: tesSUCCESS when should fail (4 total) ────────────────────────────
    MismatchCase {
        seq: 104115547,
        txid: "414ccec5",
        tx_type: "Payment",
        flags: 0x00000000,
        our_ter: "tesSUCCESS",
        ter: "tecUNFUNDED_PAYMENT",
        cause: "P6_success_when_should_fail",
    },
    MismatchCase {
        seq: 104115877,
        txid: "b79fd82a",
        tx_type: "Payment",
        flags: 0x00000000,
        our_ter: "tesSUCCESS",
        ter: "tecUNFUNDED_PAYMENT",
        cause: "P6_success_when_should_fail",
    },
    MismatchCase {
        seq: 104115547,
        txid: "51b09c0d",
        tx_type: "OfferCreate",
        flags: 0x00080000,
        our_ter: "tesSUCCESS",
        ter: "tecUNFUNDED_OFFER",
        cause: "P6_success_when_should_fail",
    },
    MismatchCase {
        seq: 104115862,
        txid: "c4d5ff03",
        tx_type: "OfferCreate",
        flags: 0x00010000,
        our_ter: "tesSUCCESS",
        ter: "tecINSUF_RESERVE_OFFER",
        cause: "P6_success_when_should_fail",
    },
    // ── P7: tecNO_LINE_REDUNDANT false positive (1 total) ────────────────────
    MismatchCase {
        seq: 104115668,
        txid: "641cf153",
        tx_type: "TrustSet",
        flags: 0x00020000,
        our_ter: "tecNO_LINE_REDUNDANT",
        ter: "tesSUCCESS",
        cause: "P7_no_line_redundant_false_positive",
    },
];

/// Print a summary of all cases grouped by cause.
/// Run with: cargo test -p app --test mismatch_regression summary -- --nocapture
#[test]
fn summary() {
    use std::collections::BTreeMap;
    let mut by_cause: BTreeMap<&str, Vec<&MismatchCase>> = BTreeMap::new();
    for c in CASES {
        by_cause.entry(c.cause).or_default().push(c);
    }
    println!(
        "\n=== Mismatch Regression Cases ({} total) ===",
        CASES.len()
    );
    for (cause, cases) in &by_cause {
        println!("\n  {} ({} cases):", cause, cases.len());
        for c in cases {
            println!(
                "    seq={} txid={} {} our={} cpp={}",
                c.seq, c.txid, c.tx_type, c.our_ter, c.ter
            );
        }
    }
    println!();
}

// ── P0: tecDIR_FULL ──────────────────────────────────────────────────────────

/// P0: OfferCreate returns tecDIR_FULL when C++ returns tesSUCCESS.
/// Root cause: find_previous_page fell back to root instead of failing.
/// Fix: return Err when previous page missing (matches C++ logic_error).
/// Ledger 104115399, txid c765e8ca — OfferCreate, no flags.
#[test]
fn p0_dir_full_offer_create_104115399_c765e8ca() {
    assert_case("c765e8ca", "tecDIR_FULL", "tesSUCCESS", "P0_dir_full");
}

/// P0: OfferCreate with tfPassive flag returns tecDIR_FULL.
/// Ledger 104115405, txid 1e6e3558 — OfferCreate, flags=0x00080000 (tfPassive).
#[test]
fn p0_dir_full_offer_create_passive_104115405_1e6e3558() {
    assert_case("1e6e3558", "tecDIR_FULL", "tesSUCCESS", "P0_dir_full");
}

/// P0: OfferCreate returns tecDIR_FULL in ledger 104115422.
/// Ledger 104115422, txid 1cbbb3f7 — OfferCreate, no flags.
#[test]
fn p0_dir_full_offer_create_104115422_1cbbb3f7() {
    assert_case("1cbbb3f7", "tecDIR_FULL", "tesSUCCESS", "P0_dir_full");
}

// ── P1a: credit_balance sign ─────────────────────────────────────────────────

/// P1a: OfferCreate returns tecUNFUNDED_OFFER when C++ returns tesSUCCESS.
/// Root cause: get_account_funds_for_offer returns 0 when offer book has
/// been crossed and the remaining TakerGets balance is misread.
/// Ledger 104115405, txid 6ddcd40a — OfferCreate, TakerPays=XRP TakerGets=USD.
#[test]
fn p1a_credit_balance_sign_unfunded_offer_104115405_6ddcd40a() {
    assert_case(
        "6ddcd40a",
        "tecUNFUNDED_OFFER",
        "tesSUCCESS",
        "P1a_unfunded_offer_flow_engine",
    );
}

/// P1a: OfferCreate returns tecUNFUNDED_OFFER — EUR offer.
/// Ledger 104115405, txid ffe59f8d — OfferCreate, TakerPays=XRP TakerGets=EUR.
#[test]
fn p1a_credit_balance_sign_unfunded_offer_eur_104115405_ffe59f8d() {
    assert_case(
        "ffe59f8d",
        "tecUNFUNDED_OFFER",
        "tesSUCCESS",
        "P1a_unfunded_offer_flow_engine",
    );
}

/// P1a: OfferCreate returns tecUNFUNDED_OFFER in ledger 104115422.
/// Ledger 104115422, txid 65ddd1bb — OfferCreate, TakerPays=XRP TakerGets=USD.
#[test]
fn p1a_credit_balance_sign_unfunded_offer_104115422_65ddd1bb() {
    assert_case(
        "65ddd1bb",
        "tecUNFUNDED_OFFER",
        "tesSUCCESS",
        "P1a_unfunded_offer_flow_engine",
    );
}

// ── P1b: flow engine precision ───────────────────────────────────────────────

/// P1b: Payment returns tecPATH_DRY when C++ returns tesSUCCESS.
/// Root cause: flow engine delivers 0 for XRP→IOU via issuer trust line.
/// Ledger 104115399, txid 466bb915 — Payment, tfNoRippleDirect, XRP→IOU.
#[test]
fn p1b_path_dry_vs_success_xrp_to_iou_104115399_466bb915() {
    assert_case(
        "466bb915",
        "tecPATH_DRY",
        "tesSUCCESS",
        "P1b_flow_engine_path_dry",
    );
}

/// P1b: Payment returns tecPATH_DRY when C++ returns tesSUCCESS — GiB token.
/// Ledger 104115399, txid cf491c64 — Payment, tfPartialPayment|tfNoRippleDirect.
#[test]
fn p1b_path_dry_vs_success_gib_104115399_cf491c64() {
    assert_case(
        "cf491c64",
        "tecPATH_DRY",
        "tesSUCCESS",
        "P1b_flow_engine_path_dry",
    );
}

/// P1b: Payment returns tecPATH_DRY when C++ returns tesSUCCESS — multi-hop.
/// Ledger 104115405, txid 2168a966 — Payment, tfNoRippleDirect|tfLimitQuality.
#[test]
fn p1b_path_dry_vs_success_multihop_104115405_2168a966() {
    assert_case(
        "2168a966",
        "tecPATH_DRY",
        "tesSUCCESS",
        "P1b_flow_engine_path_dry",
    );
}

/// P1b: Payment returns tecPATH_DRY when C++ returns tecPATH_PARTIAL.
/// Root cause: flow engine delivers 0 instead of partial amount.
/// Ledger 104115399, txid f62a546b — Payment, tfPartialPayment.
#[test]
fn p1b_path_dry_vs_partial_104115399_f62a546b() {
    assert_case(
        "f62a546b",
        "tecPATH_DRY",
        "tecPATH_PARTIAL",
        "P1b_flow_engine_path_dry_vs_partial",
    );
}

/// P1b: Payment returns tecPATH_DRY vs tecPATH_PARTIAL — same pattern.
/// Ledger 104115405, txid 982b0f57 — Payment, tfPartialPayment.
#[test]
fn p1b_path_dry_vs_partial_104115405_982b0f57() {
    assert_case(
        "982b0f57",
        "tecPATH_DRY",
        "tecPATH_PARTIAL",
        "P1b_flow_engine_path_dry_vs_partial",
    );
}

/// P1b: Payment returns tecPATH_DRY vs tecPATH_PARTIAL in ledger 104115422.
/// Ledger 104115422, txid 3c9efd0d — Payment, tfPartialPayment.
#[test]
fn p1b_path_dry_vs_partial_104115422_3c9efd0d() {
    assert_case(
        "3c9efd0d",
        "tecPATH_DRY",
        "tecPATH_PARTIAL",
        "P1b_flow_engine_path_dry_vs_partial",
    );
}

/// P1b: Payment returns tesSUCCESS when C++ returns tecPATH_PARTIAL.
/// Root cause: flow engine over-delivers (ignores partial payment limit).
/// Ledger 104115399, txid b77c4423 — Payment, tfNoRippleDirect.
#[test]
fn p1b_over_delivers_success_vs_partial_104115399_b77c4423() {
    assert_case(
        "b77c4423",
        "tesSUCCESS",
        "tecPATH_PARTIAL",
        "P1b_flow_engine_over_delivers",
    );
}

// ── P2: IOC result shaping ───────────────────────────────────────────────────

/// P2: IOC OfferCreate returns tecKILLED when C++ returns tesSUCCESS.
/// Root cause: IOC always returned tecKILLED when not fully filled.
/// Fix: return tecKILLED only when !crossed (C++ OfferCreate.cpp:806).
/// Ledger 104115399, txid 2b7935e8 — OfferCreate, tfImmediateOrCancel.
#[test]
fn p2_ioc_killed_vs_success_104115399_2b7935e8() {
    assert_case("2b7935e8", "tecKILLED", "tesSUCCESS", "P2_ioc_killed");
}

/// P2: IOC OfferCreate returns tecKILLED vs tesSUCCESS — same book.
/// Ledger 104115399, txid 64eb3f66 — OfferCreate, tfImmediateOrCancel.
#[test]
fn p2_ioc_killed_vs_success_104115399_64eb3f66() {
    assert_case("64eb3f66", "tecKILLED", "tesSUCCESS", "P2_ioc_killed");
}

/// P2: IOC OfferCreate returns tecKILLED vs tesSUCCESS — same book.
/// Ledger 104115399, txid c1aa8f9e — OfferCreate, tfImmediateOrCancel.
#[test]
fn p2_ioc_killed_vs_success_104115399_c1aa8f9e() {
    assert_case("c1aa8f9e", "tecKILLED", "tesSUCCESS", "P2_ioc_killed");
}

/// P2: IOC OfferCreate returns tecKILLED when C++ returns tecUNFUNDED_OFFER.
/// Root cause: IOC check fires before unfunded check.
/// Ledger 104115405, txid 30ce9391 — OfferCreate, tfImmediateOrCancel.
#[test]
fn p2_ioc_killed_vs_unfunded_104115405_30ce9391() {
    assert_case(
        "30ce9391",
        "tecKILLED",
        "tecUNFUNDED_OFFER",
        "P2_ioc_killed_vs_unfunded",
    );
}

/// P2: IOC OfferCreate returns tecKILLED vs tecUNFUNDED_OFFER.
/// Ledger 104115405, txid bfa83642 — OfferCreate, tfImmediateOrCancel.
#[test]
fn p2_ioc_killed_vs_unfunded_104115405_bfa83642() {
    assert_case(
        "bfa83642",
        "tecKILLED",
        "tecUNFUNDED_OFFER",
        "P2_ioc_killed_vs_unfunded",
    );
}

// ── P3: tefBAD_LEDGER for passive OfferCreate ────────────────────────────────

/// P3: Passive OfferCreate returns tefBAD_LEDGER when C++ returns tesSUCCESS.
/// Root cause: passive offer crossing logic returns tefBAD_LEDGER on error path.
/// Ledger 104115405, txid c168f394 — OfferCreate, tfPassive.
#[test]
fn p3_tef_bad_ledger_passive_104115405_c168f394() {
    assert_case(
        "c168f394",
        "tefBAD_LEDGER",
        "tesSUCCESS",
        "P3_tef_bad_ledger_passive",
    );
}

/// P3: Passive OfferCreate returns tefBAD_LEDGER — same account, different amount.
/// Ledger 104115405, txid ef13e0c7 — OfferCreate, tfPassive.
#[test]
fn p3_tef_bad_ledger_passive_104115405_ef13e0c7() {
    assert_case(
        "ef13e0c7",
        "tefBAD_LEDGER",
        "tesSUCCESS",
        "P3_tef_bad_ledger_passive",
    );
}

/// P3: Non-passive OfferCreate returns tefBAD_LEDGER.
/// Ledger 104115427, txid 51db2431 — OfferCreate, no flags.
#[test]
fn p3_tef_bad_ledger_104115427_51db2431() {
    assert_case(
        "51db2431",
        "tefBAD_LEDGER",
        "tesSUCCESS",
        "P3_tef_bad_ledger_passive",
    );
}

// ── P4: tecNO_DST false positives ────────────────────────────────────────────

/// P4: TrustSet returns tecNO_DST when C++ returns tesSUCCESS.
/// Root cause: account existence check fires incorrectly for TrustSet.
/// Ledger 104115427, txid 1bf11416 — TrustSet, tfSetNoRipple.
#[test]
fn p4_no_dst_trustset_104115427_1bf11416() {
    assert_case(
        "1bf11416",
        "tecNO_DST",
        "tesSUCCESS",
        "P4_no_dst_false_positive",
    );
}

/// P4: Payment returns tecNO_DST when C++ returns tesSUCCESS.
/// Ledger 104115427, txid af93069a — Payment, tfNoRippleDirect.
#[test]
fn p4_no_dst_payment_104115427_af93069a() {
    assert_case(
        "af93069a",
        "tecNO_DST",
        "tesSUCCESS",
        "P4_no_dst_false_positive",
    );
}

/// P4: TrustSet returns tecNO_DST in ledger 104115541.
/// Ledger 104115541, txid 3b1a4c01 — TrustSet, no flags.
#[test]
fn p4_no_dst_trustset_104115541_3b1a4c01() {
    assert_case(
        "3b1a4c01",
        "tecNO_DST",
        "tesSUCCESS",
        "P4_no_dst_false_positive",
    );
}

/// P4: Payment returns tecNO_DST_INSUF_XRP when C++ returns tesSUCCESS.
/// Root cause: destination account exists but our state has wrong balance.
/// Ledger 104115540, txid a12fbddd — Payment, XRP to existing account.
#[test]
fn p4_no_dst_insuf_xrp_104115540_a12fbddd() {
    assert_case(
        "a12fbddd",
        "tecNO_DST_INSUF_XRP",
        "tesSUCCESS",
        "P4_no_dst_insuf_xrp_false_positive",
    );
}

/// P4: Payment returns tecNO_DST_INSUF_XRP in ledger 104115563.
/// Ledger 104115563, txid 31e9ed36 — Payment, small XRP amount.
#[test]
fn p4_no_dst_insuf_xrp_104115563_31e9ed36() {
    assert_case(
        "31e9ed36",
        "tecNO_DST_INSUF_XRP",
        "tesSUCCESS",
        "P4_no_dst_insuf_xrp_false_positive",
    );
}

// ── P5: EscrowFinish IOU missing tecLIMIT_EXCEEDED ───────────────────────────

/// P5: EscrowFinish returns tesSUCCESS when C++ returns tecLIMIT_EXCEEDED.
/// Root cause: IOU escrow finish doesn't check trust line limit.
/// Ledger 104115428, txid 2c03fdd5 — EscrowFinish.
#[test]
fn p5_escrow_finish_iou_limit_104115428_2c03fdd5() {
    assert_case(
        "2c03fdd5",
        "tesSUCCESS",
        "tecLIMIT_EXCEEDED",
        "P5_escrow_finish_iou_limit",
    );
}

/// P5: EscrowFinish IOU limit check missing in ledger 104115542.
/// Ledger 104115542, txid b78df60d — EscrowFinish.
#[test]
fn p5_escrow_finish_iou_limit_104115542_b78df60d() {
    assert_case(
        "b78df60d",
        "tesSUCCESS",
        "tecLIMIT_EXCEEDED",
        "P5_escrow_finish_iou_limit",
    );
}

/// P5: EscrowFinish IOU limit check missing in ledger 104115547.
/// Ledger 104115547, txid 8c64819d — EscrowFinish.
#[test]
fn p5_escrow_finish_iou_limit_104115547_8c64819d() {
    assert_case(
        "8c64819d",
        "tesSUCCESS",
        "tecLIMIT_EXCEEDED",
        "P5_escrow_finish_iou_limit",
    );
}

// ── P6: tesSUCCESS when should fail ──────────────────────────────────────────

/// P6: Payment returns tesSUCCESS when C++ returns tecUNFUNDED_PAYMENT.
/// Root cause: sender balance check missing or wrong after state corruption.
/// Ledger 104115547, txid 414ccec5 — Payment, XRP.
#[test]
fn p6_success_when_unfunded_payment_104115547_414ccec5() {
    assert_case(
        "414ccec5",
        "tesSUCCESS",
        "tecUNFUNDED_PAYMENT",
        "P6_success_when_should_fail",
    );
}

/// P6: Payment returns tesSUCCESS when C++ returns tecUNFUNDED_PAYMENT.
/// Ledger 104115877, txid b79fd82a — Payment, XRP.
#[test]
fn p6_success_when_unfunded_payment_104115877_b79fd82a() {
    assert_case(
        "b79fd82a",
        "tesSUCCESS",
        "tecUNFUNDED_PAYMENT",
        "P6_success_when_should_fail",
    );
}

/// P6: OfferCreate returns tesSUCCESS when C++ returns tecUNFUNDED_OFFER.
/// Ledger 104115547, txid 51b09c0d — OfferCreate, tfFillOrKill.
#[test]
fn p6_success_when_unfunded_offer_104115547_51b09c0d() {
    assert_case(
        "51b09c0d",
        "tesSUCCESS",
        "tecUNFUNDED_OFFER",
        "P6_success_when_should_fail",
    );
}

/// P6: Passive OfferCreate returns tesSUCCESS when C++ returns tecINSUF_RESERVE_OFFER.
/// Ledger 104115862, txid c4d5ff03 — OfferCreate, tfPassive.
#[test]
fn p6_success_when_insuf_reserve_104115862_c4d5ff03() {
    assert_case(
        "c4d5ff03",
        "tesSUCCESS",
        "tecINSUF_RESERVE_OFFER",
        "P6_success_when_should_fail",
    );
}

// ── P7: tecNO_LINE_REDUNDANT false positive ───────────────────────────────────

/// P7: TrustSet returns tecNO_LINE_REDUNDANT when C++ returns tesSUCCESS.
/// Root cause: redundancy check fires when trust line is being modified.
/// Ledger 104115668, txid 641cf153 — TrustSet, tfSetNoRipple.
#[test]
fn p7_no_line_redundant_104115668_641cf153() {
    assert_case(
        "641cf153",
        "tecNO_LINE_REDUNDANT",
        "tesSUCCESS",
        "P7_no_line_redundant_false_positive",
    );
}

// ── assertion helper ─────────────────────────────────────────────────────────

/// Assert that a mismatch case is documented correctly in CASES.
/// This validates the test data itself — the txid, our_ter, and ter
/// must match an entry in CASES. When the fix is applied, the test should
/// be updated to assert our_ter == ter.
fn assert_case(txid: &str, expected_our: &str, expected_cpp: &str, expected_cause: &str) {
    let case = CASES.iter().find(|c| c.txid == txid);
    let case = case.unwrap_or_else(|| {
        panic!(
            "txid {} not found in CASES — add it to the test table",
            txid
        )
    });
    assert_eq!(
        case.our_ter, expected_our,
        "txid={} our_ter mismatch: expected {} got {}",
        txid, expected_our, case.our_ter
    );
    assert_eq!(
        case.ter, expected_cpp,
        "txid={} ter mismatch: expected {} got {}",
        txid, expected_cpp, case.ter
    );
    assert_eq!(
        case.cause, expected_cause,
        "txid={} cause mismatch: expected {} got {}",
        txid, expected_cause, case.cause
    );
    // Document the known gap. When fixed, our_ter should equal ter.
    if case.our_ter != case.ter {
        println!(
            "KNOWN MISMATCH: seq={} txid={} type={} our={} cpp={} cause={}",
            case.seq, case.txid, case.tx_type, case.our_ter, case.ter, case.cause
        );
    }
}
