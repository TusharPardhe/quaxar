//! Regression reproduction for mainnet ledger 107366839, transaction index 104.
//!
//! The network applied the explicit BITX -> SOLO -> CSC self-payment through
//! both SOLO RippleState hops (seven affected nodes). The current node reports
//! tesSUCCESS while leaving those two hops unchanged, producing five nodes and
//! a divergent state root.

use super::fixtures::*;
use super::pipeline::full_apply;
use ledger::ReadView;
use protocol::{
    AccountID, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry, STPath,
    STPathElement, STPathSet, STTx, Ter, TxType, get_field_by_symbol, sf_generic,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn iou_frac(issuer: AccountID, currency: Currency, mantissa: i64, exponent: i32) -> STAmount {
    STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(mantissa, exponent).expect("valid fractional IOU"),
        Issue::new(currency, issuer),
    )
}

/// Creates a RippleState using its balance from the (already ordered) low
/// account perspective. This is necessary because the parent-ledger flags are
/// also stored in low/high orientation.
fn ripple_state(
    low: AccountID,
    high: AccountID,
    currency: Currency,
    balance_mantissa: i64,
    balance_exponent: i32,
    low_limit: (i64, i32),
    high_limit: (i64, i32),
    flags: u32,
) -> STLedgerEntry {
    assert!(low < high, "test accounts must preserve RippleState ordering");
    let keylet = protocol::line(low, high, currency);
    let amount = |issuer, mantissa, exponent| {
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(mantissa, exponent).expect("valid trust-line amount"),
            Issue::new(currency, issuer),
        )
    };
    let mut line = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, keylet.key);
    line.set_field_amount(
        sf("sfBalance"),
        amount(low, balance_mantissa, balance_exponent),
    );
    line.set_field_amount(sf("sfLowLimit"), amount(low, low_limit.0, low_limit.1));
    line.set_field_amount(
        sf("sfHighLimit"),
        amount(high, high_limit.0, high_limit.1),
    );
    line.set_field_u32(sf("sfFlags"), flags);
    line
}

fn line_balance(view: &impl ReadView, low: AccountID, high: AccountID, currency: Currency) -> STAmount {
    view.read(protocol::line(low, high, currency))
        .expect("read trust line")
        .expect("parent trust line must exist")
        .get_field_amount(sf("sfBalance"))
}

#[test]
fn mainnet_107366839_self_payment_must_modify_both_solo_hops() {
    // Map the mainnet participants to distinct, ordered fixture accounts while
    // preserving the payment cycle:
    // rogue -> BITX issuer -> BITX holder -> SOLO issuer -> CSC holder -> CSC issuer -> rogue.
    let rogue = acct(0x11);
    let bitx_issuer = acct(0x22);
    let csc_issuer = acct(0x33);
    let solo_issuer = acct(0x44);
    let bitx_holder = acct(0x55);
    let csc_holder = acct(0x66);
    let bitx = Currency::from_hex("4249547800000000000000000000000000000000")
        .expect("BITX hex currency");
    let csc = protocol::currency_from_string("CSC");
    let solo = Currency::from_hex("534F4C4F00000000000000000000000000000000")
        .expect("SOLO hex currency");

    // AccountRoot Flags/TransferRate are from parent ledger 107366838. The
    // public account_info endpoint returned actMalformed for the two supplied
    // intermediary strings, so their roots use DefaultRipple, required by the
    // stated rippling topology; no transfer rate was provided for either.
    let mut bitx_root = account_root(bitx_issuer, 10_000_000_000, 0, 9_961_472);
    let mut csc_root = account_root(csc_issuer, 10_000_000_000, 0, 9_437_184);
    let mut solo_root = account_root(solo_issuer, 10_000_000_000, 0, 12_058_624);
    solo_root.set_field_u32(sf("sfTransferRate"), 1_000_100_000);
    // Explicitly retain the default transfer rate on BITX and CSC: their
    // canonical roots omit sfTransferRate. All accounts involved in rippling
    // must default-ripple, as do their mainnet AccountRoots.
    bitx_root.set_field_u32(sf("sfFlags"), 9_961_472);
    csc_root.set_field_u32(sf("sfFlags"), 9_437_184);

    let ledger = build_ledger_with_features(
        vec![
            account_root(rogue, 100_000_000_000, 2, protocol::lsfDefaultRipple),
            bitx_root,
            csc_root,
            solo_root,
            account_root(bitx_holder, 100_000_000_000, 2, protocol::lsfDefaultRipple),
            account_root(csc_holder, 100_000_000_000, 2, protocol::lsfDefaultRipple),
            // CSC rCSCMan... (low) -> rHnb... (high), balance -7614891.455957439.
            ripple_state(
                csc_issuer,
                csc_holder,
                csc,
                -7_614_891_455_957_439,
                -9,
                (0, 0),
                (0, 0),
                16_908_288,
            ),
            // BITX rogue (low) -> rBitcoi... (high), balance 0.
            ripple_state(
                rogue,
                bitx_issuer,
                bitx,
                0,
                0,
                (9_999_999_999_999_999, 80),
                (0, 0),
                1_114_112,
            ),
            // The original line is rfgw... (low) -> rBitcoi... (high). In
            // fixture order BITX issuer is low, so negate the stated balance.
            ripple_state(
                bitx_issuer,
                bitx_holder,
                bitx,
                -1_851_053_532_862_736,
                -16,
                (0, 0),
                (0, 0),
                16_842_752,
            ),
            // Original low rCSCMan... -> high rogue; fixture ordering reverses it.
            ripple_state(
                rogue,
                csc_issuer,
                csc,
                3_776_382_846_535_617,
                -9,
                (9_999_999_999_999_999, 79),
                (0, 0),
                2_228_224,
            ),
            // SOLO rsoLo... (low) -> rfgw... (high).
            ripple_state(
                solo_issuer,
                bitx_holder,
                solo,
                -1_741_232_622_763_737,
                -12,
                (0, 0),
                (0, 0),
                16_908_288,
            ),
            // SOLO rsoLo... (low) -> rHnb... (high).
            ripple_state(
                solo_issuer,
                csc_holder,
                solo,
                -5_036_600_303_715_244,
                -11,
                (0, 0),
                (0, 0),
                16_908_288,
            ),
        ],
        vec!["fixReducedOffersV2"],
    );
    let mut view = new_view(ledger);

    let solo_to_bitx_before = line_balance(&view, solo_issuer, bitx_holder, solo);
    let solo_to_csc_before = line_balance(&view, solo_issuer, csc_holder, solo);

    // Mainnet Paths:
    // [{currency SOLO, issuer rsoLo...}, {account rsoLo...},
    //  {currency CSC, issuer rCSCMan...}, {account rCSCMan...}].
    let mut path = STPath::new();
    path.push_back(STPathElement::from_optionals(None, Some(solo.into()), Some(solo_issuer)));
    path.push_back(STPathElement::from_optionals(Some(solo_issuer), None, None));
    path.push_back(STPathElement::from_optionals(None, Some(csc.into()), Some(csc_issuer)));
    path.push_back(STPathElement::from_optionals(Some(csc_issuer), None, None));
    let mut paths = STPathSet::new(sf("sfPaths"));
    paths.push_back(path);

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), rogue);
        tx.set_account_id(sf("sfDestination"), rogue);
        tx.set_field_amount(
            sf("sfAmount"),
            iou_frac(csc_issuer, csc, 1_051_839_906_607_096, -12),
        );
        tx.set_field_amount(
            sf("sfSendMax"),
            iou_frac(bitx_issuer, bitx, 7_503_332_193_915_335, -19),
        );
        tx.set_field_path_set(sf("sfPaths"), paths);
        tx.set_field_u32(sf("sfFlags"), protocol::tfPartialPayment);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });

    let result = full_apply(&mut view, &tx, TxType::PAYMENT);
    let solo_to_bitx_after = line_balance(&view, solo_issuer, bitx_holder, solo);
    let solo_to_csc_after = line_balance(&view, solo_issuer, csc_holder, solo);
    let affected_nodes = view
        .table()
        .to_tx_meta(tx.get_transaction_id(), 4, None)
        .get_nodes()
        .len();
    let solo_to_bitx_modified = solo_to_bitx_before != solo_to_bitx_after;
    let solo_to_csc_modified = solo_to_csc_before != solo_to_csc_after;
    eprintln!(
        "mainnet_107366839: node returned {result:?} ({result_token}); affected_nodes={affected_nodes}; \
         solo_to_bitx_modified={solo_to_bitx_modified} ({solo_to_bitx_before:?} -> {solo_to_bitx_after:?}); \
         solo_to_csc_modified={solo_to_csc_modified} ({solo_to_csc_before:?} -> {solo_to_csc_after:?})",
        result_token = protocol::trans_token(result),
    );

    assert_eq!(
        result,
        Ter::TES_SUCCESS,
        "network result is tesSUCCESS; node returned {result:?}; affected_nodes={affected_nodes}"
    );
    assert!(
        solo_to_bitx_modified && solo_to_csc_modified,
        "network modified both SOLO rippling lines (7 affected nodes); node returned \
         affected_nodes={affected_nodes}, solo_to_bitx_modified={solo_to_bitx_modified}, \
         solo_to_csc_modified={solo_to_csc_modified}"
    );
}
