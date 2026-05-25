#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Shared test fixtures for integration tests requiring IOU infrastructure.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyViewImpl, Ledger, LedgerHeader};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    XRPAmount, account_keylet, get_field_by_symbol, line, owner_dir_keylet, sf_generic,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

pub fn acct(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}
pub fn acct_id(a: AccountID) -> Uint160 {
    Uint160::from_slice(a.data()).expect("w")
}
pub fn xrp(drops: i64) -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(drops))
}

pub fn usd_currency() -> Currency {
    protocol::currency_from_string("USD")
}
pub fn eur_currency() -> Currency {
    protocol::currency_from_string("EUR")
}

pub fn iou(issuer: AccountID, currency: Currency, value: i64) -> STAmount {
    let issue = Issue::new(currency, issuer);
    STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(value, 0).expect("a"),
        issue,
    )
}

pub fn account_root(account: AccountID, balance: i64, owners: u32, flags: u32) -> STLedgerEntry {
    let k = account_keylet(acct_id(account));
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, k.key);
    e.set_account_id(sf("sfAccount"), account);
    e.set_field_u32(sf("sfSequence"), 1);
    e.set_field_amount(sf("sfBalance"), xrp(balance));
    e.set_field_u32(sf("sfOwnerCount"), owners);
    e.set_field_u32(sf("sfFlags"), flags);
    e.set_field_h256(sf("sfPreviousTxnID"), Uint256::from_array([0xA1; 32]));
    e.set_field_u32(sf("sfPreviousTxnLgrSeq"), 1);
    e
}

/// Create a trust line (RippleState) between two accounts.
/// `low` must be < `high` lexicographically (by account ID bytes).
/// `balance` is from low's perspective (positive = low holds tokens).
pub fn trust_line(
    low: AccountID,
    high: AccountID,
    currency: Currency,
    balance: i64,
    low_limit: i64,
    high_limit: i64,
) -> STLedgerEntry {
    let keylet = line(low, high, currency);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, keylet.key);
    sle.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(balance, 0).expect("b"),
            Issue::new(currency, low),
        ),
    );
    sle.set_field_amount(
        sf("sfLowLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(low_limit, 0).expect("l"),
            Issue::new(currency, low),
        ),
    );
    sle.set_field_amount(
        sf("sfHighLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(high_limit, 0).expect("h"),
            Issue::new(currency, high),
        ),
    );
    sle.set_field_u32(sf("sfFlags"), 0);
    sle
}

/// Create an offer SLE directly in the ledger.
pub fn offer_sle(
    account: AccountID,
    sequence: u32,
    taker_pays: STAmount,
    taker_gets: STAmount,
) -> STLedgerEntry {
    let keylet = protocol::offer_keylet(acct_id(account), sequence);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, keylet.key);
    sle.set_account_id(sf("sfAccount"), account);
    sle.set_field_u32(sf("sfSequence"), sequence);
    sle.set_field_amount(sf("sfTakerPays"), taker_pays);
    sle.set_field_amount(sf("sfTakerGets"), taker_gets);
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    sle.set_field_h256(sf("sfBookDirectory"), Uint256::from_array([0x01; 32]));
    sle.set_field_u64(sf("sfBookNode"), 0);
    sle
}

/// Build a ledger from entries with standard test config.
pub fn build_ledger(entries: Vec<STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for e in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*e.key(), e.get_serializer().data().to_vec()),
        )
        .expect("insert");
    }
    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 3,
            close_time: 1000,
            parent_close_time: 1000,
            parent_hash: basics::sha_map_hash::SHAMapHash::new(
                basics::base_uint::Uint256::from_array([0x01; 32]),
            ),
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            1,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    );
    ledger.set_fees(ledger::Fees {
        base: 10,
        reserve: 200_000,
        increment: 50_000,
    });
    ledger
}

/// Build a ledger with amendments enabled.
pub fn build_ledger_with_features(entries: Vec<STLedgerEntry>, features: Vec<&str>) -> Ledger {
    let mut ledger = build_ledger(entries);
    let feature_ids: Vec<_> = features.iter().map(|f| protocol::feature_id(f)).collect();
    ledger.set_rules(protocol::Rules::new(feature_ids.into_iter()));
    ledger
}

pub fn new_view(ledger: Ledger) -> ApplyViewImpl<Ledger> {
    ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE)
}
