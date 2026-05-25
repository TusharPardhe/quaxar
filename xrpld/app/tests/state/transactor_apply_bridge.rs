use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyViewImpl, Ledger, LedgerHeader, ReadView};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    STTx, account_keylet, currency_from_string, get_field_by_symbol, line, lsfHighNoRipple,
    lsfHighReserve, lsfLowAuth, lsfLowFreeze, lsfLowNoRipple, lsfLowReserve, owner_dir_keylet,
    page_keylet, sf_generic, tfSetFreeze, tfSetNoRipple, tfSetfAuth,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

use app::state::trust_set::do_trust_set;

fn sample_account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn sample_uint256(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn iou_amount(value: i64) -> IOUAmount {
    IOUAmount::from_parts(value, 0).expect("canonical IOU amount")
}

fn account_root(account: AccountID, owner_count: u32, flags: u32) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    entry.set_account_id(get_field_by_symbol("sfAccount"), account);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(1_000_000, false),
    );
    entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), owner_count);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xA1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    if flags != 0 {
        entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    }
    entry
}

fn empty_ledger(entries: Vec<STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insert should succeed");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq: 1,
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
    )
}

fn trust_limit(currency: Currency, issuer: AccountID, value: i64) -> STAmount {
    STAmount::from_iou_amount(
        sf_generic(),
        iou_amount(value),
        Issue::new(currency, issuer),
    )
}

fn trust_line_entry(
    low: AccountID,
    high: AccountID,
    currency: Currency,
    low_limit: i64,
    high_limit: i64,
    flags: u32,
) -> STLedgerEntry {
    let keylet = line(low, high, currency);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, keylet.key);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_uint256(0xB1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 2);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::from_iou_amount(
            sf_generic(),
            iou_amount(0),
            Issue::new(currency, protocol::no_account()),
        ),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfLowLimit"),
        trust_limit(currency, low, low_limit),
    );
    entry.set_field_amount(
        get_field_by_symbol("sfHighLimit"),
        trust_limit(currency, high, high_limit),
    );
    entry.set_field_u32(get_field_by_symbol("sfFlags"), flags);
    entry
}

fn owner_dir_root(page_owner: AccountID, child: Uint256) -> STLedgerEntry {
    let root = owner_dir_keylet(raw_account_id(page_owner));
    let mut entry = STLedgerEntry::new(root);
    entry.set_field_h256(get_field_by_symbol("sfRootIndex"), root.key);
    entry.set_field_v256(
        get_field_by_symbol("sfIndexes"),
        protocol::STVector256::from_values(get_field_by_symbol("sfIndexes"), vec![child]),
    );
    entry
}

fn trust_set_tx(
    source: AccountID,
    limit_amount: STAmount,
    tx_flags: u32,
    quality_in: Option<u32>,
    quality_out: Option<u32>,
) -> STTx {
    STTx::new(protocol::TxType::TRUST_SET, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_field_amount(get_field_by_symbol("sfLimitAmount"), limit_amount.clone());
        tx.set_field_u32(get_field_by_symbol("sfFlags"), tx_flags);
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 1);
        if let Some(quality_in) = quality_in {
            tx.set_field_u32(get_field_by_symbol("sfQualityIn"), quality_in);
        }
        if let Some(quality_out) = quality_out {
            tx.set_field_u32(get_field_by_symbol("sfQualityOut"), quality_out);
        }
    })
}

#[test]
fn trust_set_create_line_populates_flags_limits_and_owner_dirs() {
    let source = sample_account(0x11);
    let destination = sample_account(0x22);
    let currency = currency_from_string("USD");
    let ledger = empty_ledger(vec![
        account_root(source, 0, 0),
        account_root(destination, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let limit_amount = trust_limit(currency, destination, 50);
    let tx = trust_set_tx(
        source,
        limit_amount.clone(),
        tfSetfAuth | tfSetNoRipple | tfSetFreeze,
        Some(7),
        Some(9),
    );

    let result = do_trust_set(&mut view, &tx, Some(1_000_000));

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    let line_sle = view
        .read(line(source, destination, currency))
        .expect("line read should succeed")
        .expect("line should exist");
    assert_eq!(
        line_sle.get_field_amount(get_field_by_symbol("sfLowLimit")),
        limit_amount
    );
    assert_eq!(
        line_sle.get_field_amount(get_field_by_symbol("sfHighLimit")),
        trust_limit(currency, destination, 0)
    );
    assert_eq!(
        line_sle.get_field_u32(get_field_by_symbol("sfLowQualityIn")),
        7
    );
    assert_eq!(
        line_sle.get_field_u32(get_field_by_symbol("sfLowQualityOut")),
        9
    );
    assert_eq!(
        line_sle.get_field_u32(get_field_by_symbol("sfFlags")),
        lsfLowReserve | lsfLowAuth | lsfLowNoRipple | lsfLowFreeze | lsfHighNoRipple
    );

    let source_root = view
        .read(account_keylet(raw_account_id(source)))
        .expect("source root read should succeed")
        .expect("source root should exist");
    assert_eq!(
        source_root.get_field_u32(get_field_by_symbol("sfOwnerCount")),
        1
    );

    let low_dir = owner_dir_keylet(raw_account_id(source));
    let high_dir = owner_dir_keylet(raw_account_id(destination));
    let low_root = view
        .read(low_dir)
        .expect("low dir read should succeed")
        .expect("low dir should exist");
    let high_root = view
        .read(high_dir)
        .expect("high dir read should succeed")
        .expect("high dir should exist");
    assert_eq!(
        low_root
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value(),
        &[line_sle.key().to_owned()]
    );
    assert_eq!(
        high_root
            .get_field_v256(get_field_by_symbol("sfIndexes"))
            .value(),
        &[line_sle.key().to_owned()]
    );
}

#[test]
fn trust_set_delete_existing_line_clears_owner_counts_and_owner_dirs() {
    let source = sample_account(0x33);
    let destination = sample_account(0x44);
    let currency = currency_from_string("EUR");
    let line_keylet = line(source, destination, currency);
    let mut line_entry = trust_line_entry(
        source,
        destination,
        currency,
        25,
        0,
        lsfLowReserve | lsfHighReserve | lsfLowNoRipple | lsfHighNoRipple,
    );
    line_entry.set_field_u64(get_field_by_symbol("sfLowNode"), 0);
    line_entry.set_field_u64(get_field_by_symbol("sfHighNode"), 0);

    let ledger = empty_ledger(vec![
        account_root(source, 1, 0),
        account_root(destination, 1, 0),
        line_entry,
        owner_dir_root(source, line_keylet.key),
        owner_dir_root(destination, line_keylet.key),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let limit_amount = trust_limit(currency, destination, 0);
    let tx = trust_set_tx(source, limit_amount, 0, None, None);

    let result = do_trust_set(&mut view, &tx, Some(1_000_000));

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    assert!(
        view.read(line(source, destination, currency))
            .expect("line read should succeed")
            .is_none()
    );
    assert_eq!(
        view.read(account_keylet(raw_account_id(source)))
            .expect("source root read should succeed")
            .expect("source root should exist")
            .get_field_u32(get_field_by_symbol("sfOwnerCount")),
        0
    );
    assert_eq!(
        view.read(account_keylet(raw_account_id(destination)))
            .expect("destination root read should succeed")
            .expect("destination root should exist")
            .get_field_u32(get_field_by_symbol("sfOwnerCount")),
        0
    );
    assert!(
        view.read(page_keylet(owner_dir_keylet(raw_account_id(source)), 0))
            .expect("source dir read should succeed")
            .is_none()
    );
    assert!(
        view.read(page_keylet(
            owner_dir_keylet(raw_account_id(destination)),
            0
        ))
        .expect("destination dir read should succeed")
        .is_none()
    );
}
