//! Full-execution reproduction of the 2026-10-01 mainnet ledger-hash
//! divergence: a tfPartialPayment swapping USDC->XRP delivered partial XRP
//! through the XRP/USDC AMM on mainnet (tesSUCCESS, 19309377 drops) but Quaxar
//! returned tecPATH_DRY because the reverse book step produced zero output.
//!
//! This test builds the real AMM pool state in a view and runs the Book step's
//! Reverse pass (ledger::ripple_calc::book_step::execute_book_step_with_options)
//! for the USDC(in)->XRP(out) book. It must consume AMM liquidity and produce
//! positive XRP output for a requested XRP amount.

use basics::base_uint::{Uint160, Uint256};
use ledger::ripple_calc::book_step::{
    execute_book_step_with_options, Book, BookStepOptions, BookStepPass,
};
use ledger::{ApplyViewImpl, Ledger, LedgerHeader};
use ledger::flow_engine::AmmContext;
use protocol::{
    AccountID, ApplyFlags, Asset, IOUAmount, Issue, LedgerEntryType, STAmount, STIssue,
    STLedgerEntry, Ter, XRPAmount, account_keylet, amm as amm_keylet, currency_from_string,
    get_field_by_symbol, line, sf_generic, xrp_account,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn uint160(a: AccountID) -> Uint160 {
    Uint160::from_slice(a.data()).expect("account width")
}

fn usdc_issue(issuer: AccountID) -> Issue {
    Issue::new(currency_from_string("USD"), issuer)
}

fn iou(value_mantissa: i64, exponent: i32, issue: Issue) -> STAmount {
    STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(value_mantissa, exponent).expect("iou"),
        issue,
    )
}

fn build_view_ledger(items: Vec<(Uint256, Vec<u8>)>) -> Ledger {
    let header = LedgerHeader {
        seq: 100,
        close_time: 1_000,
        close_time_resolution: ledger::LEDGER_DEFAULT_TIME_RESOLUTION,
        drops: ledger::INITIAL_XRP_DROPS,
        ..LedgerHeader::default()
    };
    let mut tree = MutableTree::new(100);
    for (key, payload) in &items {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state item");
    }
    Ledger::from_maps(
        header,
        SyncTree::from_root_with_type(tree.root(), SHAMapType::State, false, 100, SyncState::Immutable),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 100),
    )
}

fn account_root(account: AccountID, drops: u64) -> (Uint256, Vec<u8>) {
    let key = account_keylet(uint160(account)).key;
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, key);
    e.set_account_id(sf("sfAccount"), account);
    e.set_field_amount(sf("sfBalance"), STAmount::new_native(drops, false));
    e.set_field_u32(sf("sfSequence"), 1);
    e.set_field_u32(sf("sfOwnerCount"), 1);
    (key, e.get_serializer().data().to_vec())
}

fn amm_trustline(amm: AccountID, issuer: AccountID, usdc_value_mantissa: i64, exponent: i32) -> (Uint256, Vec<u8>) {
    // RippleState for the AMM's USDC holding. Balance sign is from the low
    // account's perspective. Pick limits large enough to not bind.
    let currency = currency_from_string("USD");
    let (low, high) = if amm < issuer { (amm, issuer) } else { (issuer, amm) };
    let key = line(low, high, currency).key;
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, key);
    // Balance stored from low perspective; if amm is high, amm holding USDC
    // means the balance (low owes amm) is positive from low... set so that
    // amm_account_holds returns +usdc for the amm.
    let bal_from_low = if amm < issuer {
        // amm is low: amm holding +usdc means low balance is +usdc
        iou(usdc_value_mantissa, exponent, Issue::new(currency, low))
    } else {
        // amm is high: amm holding +usdc means low (issuer) balance is -usdc
        iou(-usdc_value_mantissa, exponent, Issue::new(currency, low))
    };
    e.set_field_amount(sf("sfBalance"), bal_from_low);
    e.set_field_amount(sf("sfLowLimit"), iou(1_000_000_000, 0, Issue::new(currency, low)));
    e.set_field_amount(sf("sfHighLimit"), iou(1_000_000_000, 0, Issue::new(currency, high)));
    (key, e.get_serializer().data().to_vec())
}

fn amm_sle(amm_key: Uint256, amm_account: AccountID, issuer: AccountID, fee: u16) -> (Uint256, Vec<u8>) {
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::AMM, amm_key);
    e.set_account_id(sf("sfAccount"), amm_account);
    e.set_field_u16(sf("sfTradingFee"), fee);
    e.set_field_amount(
        sf("sfLPTokenBalance"),
        iou(1_000_000, 0, Issue::new(currency_from_string("LPT"), amm_account)),
    );
    let xrp_asset = Asset::Issue(Issue::new(currency_from_string("XRP"), xrp_account()));
    let usd_asset = Asset::Issue(usdc_issue(issuer));
    e.set_field_issue(sf("sfAsset"), STIssue::new_with_asset(sf("sfAsset"), xrp_asset));
    e.set_field_issue(sf("sfAsset2"), STIssue::new_with_asset(sf("sfAsset2"), usd_asset));
    (amm_key, e.get_serializer().data().to_vec())
}

#[test]
fn reverse_book_step_usdc_to_xrp_consumes_amm_liquidity() {
    let issuer = AccountID::from_array([0x11; 20]);
    let amm_account = AccountID::from_array([0x22; 20]);
    let taker = AccountID::from_array([0x33; 20]);

    let xrp_asset = Asset::Issue(Issue::new(currency_from_string("XRP"), xrp_account()));
    let usd_asset = Asset::Issue(usdc_issue(issuer));
    let amm_key = amm_keylet(usd_asset, xrp_asset).key;

    // AMM pool: ~38763 XRP / ~57500 USDC, fee 54 (the mainnet XRP/USDC pool shape).
    let pool_xrp_drops: u64 = 38_763_602_643;
    let items = vec![
        account_root(amm_account, pool_xrp_drops),
        account_root(taker, 1_000_000_000),
        account_root(issuer, 1_000_000_000),
        amm_trustline(amm_account, issuer, 5_750_075_485_900_167, -11), // 57500.75485900167 USDC
        amm_sle(amm_key, amm_account, issuer, 54),
    ];
    let base = Arc::new(build_view_ledger(items));
    let mut view = ApplyViewImpl::new(base, ApplyFlags::NONE);

    let book = Book {
        r#in: usd_asset,
        out: xrp_asset,
        domain: None,
    };
    // Request a modest XRP output (well within the pool): 19.309377 XRP.
    let requested_out = STAmount::from_xrp_amount(XRPAmount::from_drops(19_309_377));
    // Unlimited USDC input (reverse book step is output-driven).
    let unlimited_in = iou(9_999_999_999_999_999, 80, usdc_issue(issuer));

    let amm_context = AmmContext::new(taker, false);
    let result = execute_book_step_with_options(
        &mut view,
        &book,
        &unlimited_in,
        &requested_out,
        BookStepOptions {
            pass: BookStepPass::Reverse,
            reverse_input: None,
            owner_pays_transfer_fee: false,
            taker: Some(&taker),
            quality_threshold: None,
            remove_self_crossing: false,
            self_cross_cancellation: None,
            amm_context: Some(amm_context),
            previous_redeems: false,
            has_previous_step: false,
            previous_step_is_book: false,
            strand_dst: Some(&taker),
            strand_deliver: Some(xrp_asset),
            enforce_quality_threshold: false,
        },
    );

    assert_eq!(result.ter, Ter::TES_SUCCESS, "reverse book step must succeed");
    assert!(
        result.amount_out.signum() > 0,
        "reverse book step must produce positive XRP output from AMM liquidity; zero output is \
         the tecPATH_DRY divergence (amount_out={:?}, amount_in={:?})",
        result.amount_out,
        result.amount_in
    );
    assert!(result.amount_in.signum() > 0, "reverse book step must consume USDC input");
}
