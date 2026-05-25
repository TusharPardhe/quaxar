use basics::base_uint::{Uint160, Uint256};
use ledger::{
    Ledger, LedgerHeader, credit_balance, credit_limit, is_deep_frozen, is_frozen,
    is_individual_frozen,
};
use protocol::{
    AccountID, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    account_keylet, currency_from_string, get_field_by_symbol, line, line_from_issue,
    lsfGlobalFreeze, lsfHighDeepFreeze, lsfHighFreeze, lsfLowDeepFreeze, lsfLowFreeze, sf_generic,
    xrp_currency,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn sample_account(fill: u8) -> Uint160 {
    Uint160::from_array([fill; 20])
}

fn to_account_id(account: Uint160) -> AccountID {
    AccountID::from_slice(account.data()).expect("account width should match")
}

fn trustline_key(low: Uint160, high: Uint160, currency: Currency) -> protocol::Keylet {
    line(to_account_id(low), to_account_id(high), currency)
}

fn iou_amount(mantissa: i64) -> IOUAmount {
    IOUAmount::from_parts(mantissa, 0).expect("expected IOU amount to be canonical")
}

fn make_trustline_entry(
    low: Uint160,
    high: Uint160,
    currency: Currency,
    balance: i64,
    low_limit: i64,
    high_limit: i64,
    flags: u32,
) -> Vec<u8> {
    let keylet = trustline_key(low, high, currency);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, keylet.key);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x41));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 314);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::from_iou_amount(
            sf_generic(),
            iou_amount(balance),
            Issue::new(currency, to_account_id(low)),
        ),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfLowLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            iou_amount(low_limit),
            Issue::new(currency, to_account_id(low)),
        ),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfHighLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            iou_amount(high_limit),
            Issue::new(currency, to_account_id(high)),
        ),
    );
    if flags != 0 {
        assert!(entry.set_flag(flags));
    }

    entry.get_serializer().data().to_vec()
}

fn make_account_root_entry(account: Uint160, flags: u32) -> Vec<u8> {
    let mut entry =
        STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, account_keylet(account).key);
    entry.set_account_id(get_field_by_symbol("sfAccount"), to_account_id(account));
    entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(0, false),
    );
    entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), 0);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0x52));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 2718);
    if flags != 0 {
        assert!(entry.set_flag(flags));
    }

    entry.get_serializer().data().to_vec()
}

fn build_ledger(entries: &[(Uint256, Vec<u8>)], seq: u32) -> Ledger {
    let mut tree = MutableTree::new(seq);
    for (key, payload) in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state tree insertion should succeed");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
    )
}

#[test]
fn trustline_keylet_canonicalizes_account_order_and_issue_form() {
    let low = sample_account(0x11);
    let high = sample_account(0x22);
    let currency = currency_from_string("USD");

    assert_eq!(
        trustline_key(low, high, currency),
        trustline_key(high, low, currency)
    );
    assert_eq!(
        line_from_issue(
            to_account_id(low),
            Issue::new(currency, to_account_id(high))
        ),
        trustline_key(low, high, currency)
    );
}

#[test]
fn credit_helpers_match_current_cpp_limit_and_balance_rules() {
    let low = sample_account(0x11);
    let high = sample_account(0x22);
    let currency = currency_from_string("USD");
    let ledger = build_ledger(
        &[(
            trustline_key(low, high, currency).key,
            make_trustline_entry(low, high, currency, 77, 100, 250, 0),
        )],
        42,
    );

    let low_limit = credit_limit(&ledger, low, high, currency)
        .expect("credit limit lookup should succeed")
        .iou();
    let high_limit = credit_limit(&ledger, high, low, currency)
        .expect("credit limit lookup should succeed")
        .iou();
    let low_balance = credit_balance(&ledger, low, high, currency)
        .expect("credit balance lookup should succeed")
        .iou();
    let high_balance = credit_balance(&ledger, high, low, currency)
        .expect("credit balance lookup should succeed")
        .iou();

    assert_eq!(low_limit, iou_amount(100));
    assert_eq!(high_limit, iou_amount(250));
    assert_eq!(low_balance, iou_amount(-77));
    assert_eq!(high_balance, iou_amount(77));
}

#[test]
fn credit_helpers_default_to_zero_when_line_is_missing() {
    let account = sample_account(0x11);
    let issuer = sample_account(0x22);
    let currency = currency_from_string("USD");
    let ledger = build_ledger(&[], 43);

    let limit = credit_limit(&ledger, account, issuer, currency)
        .expect("missing line lookup should succeed");
    let balance = credit_balance(&ledger, account, issuer, currency)
        .expect("missing line lookup should succeed");

    assert_eq!(limit.iou(), IOUAmount::new());
    assert_eq!(balance.iou(), IOUAmount::new());
    assert_eq!(limit.issue(), Issue::new(currency, to_account_id(account)));
    assert_eq!(
        balance.issue(),
        Issue::new(currency, to_account_id(account))
    );
}

#[test]
fn freeze_helpers_match_currenting_and_flag_rules() {
    let low = sample_account(0x11);
    let high = sample_account(0x22);
    let currency = currency_from_string("USD");
    let ledger = build_ledger(
        &[
            (
                trustline_key(low, high, currency).key,
                make_trustline_entry(
                    low,
                    high,
                    currency,
                    77,
                    100,
                    250,
                    lsfLowFreeze | lsfHighFreeze | lsfLowDeepFreeze | lsfHighDeepFreeze,
                ),
            ),
            (
                account_keylet(high).key,
                make_account_root_entry(high, lsfGlobalFreeze),
            ),
        ],
        44,
    );

    assert!(
        is_individual_frozen(&ledger, low, currency, high)
            .expect("individual freeze check should succeed")
    );
    assert!(
        is_individual_frozen(&ledger, high, currency, low)
            .expect("individual freeze check should succeed")
    );
    assert!(
        !is_individual_frozen(&ledger, low, xrp_currency(), high)
            .expect("xrp should never be individually frozen")
    );
    assert!(
        !is_individual_frozen(&ledger, low, currency, low)
            .expect("self-issued lines should not be individually frozen")
    );

    assert!(is_frozen(&ledger, low, currency, high).expect("global freeze should short-circuit"));
    assert!(
        is_frozen(&ledger, high, currency, low)
            .expect("line freeze should still apply when issuer is not globally frozen")
    );
    assert!(
        !is_frozen(&ledger, low, currency, low)
            .expect("self-issued lines should ignore trustline freeze checks")
    );
    assert!(!is_frozen(&ledger, low, xrp_currency(), high).expect("xrp should never be frozen"));

    assert!(
        is_deep_frozen(&ledger, low, currency, high)
            .expect("deep freeze should read the trustline flags")
    );
    assert!(
        is_deep_frozen(&ledger, high, currency, low)
            .expect("deep freeze should read both trustline flag sides")
    );
    assert!(
        !is_deep_frozen(&ledger, low, currency, low)
            .expect("self-issued lines should not be deep frozen")
    );
    assert!(
        !is_deep_frozen(&ledger, low, xrp_currency(), high)
            .expect("xrp should never be deep frozen")
    );
}
