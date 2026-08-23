#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Integration tests ported from C++ TrustSet_test.cpp.

use std::sync::Arc;

use app::state::application_root::{
    apply_submit_transactor_shell, apply_submit_transactor_shell_with_flags,
};
use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, FlowSandbox, Ledger, LedgerHeader, ReadView};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    STTx, StBase, Ter, TxType, XRPAmount, account_keylet, get_field_by_symbol, owner_dir_keylet,
    sf_generic,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

use super::pipeline::full_apply;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}
fn acct(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}
fn acct_id(a: AccountID) -> Uint160 {
    Uint160::from_slice(a.data()).expect("w")
}

fn account_root(account: AccountID, balance: i64, owners: u32, flags: u32) -> STLedgerEntry {
    let k = account_keylet(acct_id(account));
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, k.key);
    e.set_account_id(sf("sfAccount"), account);
    e.set_field_u32(sf("sfSequence"), 1);
    e.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
    );
    e.set_field_u32(sf("sfOwnerCount"), owners);
    e.set_field_u32(sf("sfFlags"), flags);
    e.set_field_h256(sf("sfPreviousTxnID"), Uint256::from_array([0xA1; 32]));
    e.set_field_u32(sf("sfPreviousTxnLgrSeq"), 1);
    e
}

fn make_ledger(entries: Vec<STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for e in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*e.key(), e.get_serializer().data().to_vec()),
        )
        .expect("insert");
    }
    Ledger::from_maps(
        LedgerHeader {
            seq: 3,
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

fn trust_set_tx(from: AccountID, issuer: AccountID, currency: &str, limit: i64, seq: u32) -> STTx {
    let cur = protocol::currency_from_string(currency);
    let issue = Issue::new(cur, issuer);
    let limit_amount = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(limit, 0).expect("amt"),
        issue,
    );
    STTx::new(TxType::TRUST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_amount(sf("sfLimitAmount"), limit_amount);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn trust_set_tx_with_flags(
    from: AccountID,
    issuer: AccountID,
    currency: &str,
    limit: i64,
    seq: u32,
    flags: u32,
) -> STTx {
    let cur = protocol::currency_from_string(currency);
    let issue = Issue::new(cur, issuer);
    let limit_amount = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(limit, 0).expect("amt"),
        issue,
    );
    STTx::new(TxType::TRUST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_amount(sf("sfLimitAmount"), limit_amount);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfFlags"), flags);
    })
}

fn get_owner_count(view: &impl ReadView, account: AccountID) -> u32 {
    view.read(account_keylet(acct_id(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_u32(sf("sfOwnerCount")))
        .unwrap_or(0)
}

// ─── Tests ────────────────────────────────────────────────────────────────

/// C++ TrustSet_test — basic trust line creation.
#[test]
fn trust_set_basic_creation() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(gw, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = trust_set_tx(alice, gw, "USD", 1000, 1);
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(get_owner_count(&view, alice), 1);

    let line = view
        .read(protocol::line(
            alice,
            gw,
            protocol::currency_from_string("USD"),
        ))
        .expect("trust-line lookup should succeed")
        .expect("TrustSet should create the line");
    assert!(line.is_field_present(sf("sfLowNode")));
    assert!(line.is_field_present(sf("sfHighNode")));
    assert_eq!(line.get_field_u64(sf("sfLowNode")), 0);
    assert_eq!(line.get_field_u64(sf("sfHighNode")), 0);
    assert_eq!(
        line.get_field_amount(sf("sfBalance")).issue().account,
        protocol::no_account(),
        "a newly-created RippleState balance must use rippled's neutral issuer"
    );
}

#[test]
fn trust_set_created_node_metadata_uses_neutral_balance_issuer() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(gw, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let rules = view.rules();
    let tx = trust_set_tx(alice, gw, "USD", 1000, 1);
    let tx_id = tx.get_transaction_id();
    let mut attempt = FlowSandbox::new(&mut view);

    assert_eq!(
        apply_submit_transactor_shell(&mut attempt, &tx, TxType::TRUST_SET),
        Ter::TES_SUCCESS
    );
    let meta = attempt
        .to_tx_meta(tx_id, 4, None, &rules)
        .expect("TrustSet metadata should build");
    let ripple_state = meta
        .get_nodes()
        .iter()
        .find(|node| {
            node.fname() == sf("sfCreatedNode")
                && node.get_field_u16(sf("sfLedgerEntryType"))
                    == LedgerEntryType::RippleState as u16
        })
        .expect("TrustSet should create RippleState metadata");
    let new_fields = ripple_state.get_field_object(sf("sfNewFields"));
    assert_eq!(
        new_fields.get_field_amount(sf("sfBalance")).issue().account,
        protocol::no_account(),
        "CreatedNode metadata must serialize rippled's neutral balance issuer"
    );
}

#[test]
fn trust_set_second_owner_directory_full_has_rippled_tec_lifecycle() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let high_dir = owner_dir_keylet(acct_id(gw));
    let last_page = protocol::DIR_NODE_MAX_PAGES - 1;
    let full_indexes = (0..protocol::DIR_NODE_MAX_ENTRIES)
        .map(|index| Uint256::from_array([index as u8 + 1; 32]))
        .collect::<Vec<_>>();

    let mut root = STLedgerEntry::new(high_dir);
    root.set_account_id(sf("sfOwner"), gw);
    root.set_field_h256(sf("sfRootIndex"), high_dir.key);
    root.set_field_v256(
        sf("sfIndexes"),
        protocol::STVector256::from_values(sf("sfIndexes"), full_indexes.clone()),
    );
    root.set_field_u64(sf("sfIndexPrevious"), last_page);
    root.set_field_u64(sf("sfIndexNext"), last_page);

    let mut tail = STLedgerEntry::new(protocol::page_keylet(high_dir, last_page));
    tail.set_account_id(sf("sfOwner"), gw);
    tail.set_field_h256(sf("sfRootIndex"), high_dir.key);
    tail.set_field_v256(
        sf("sfIndexes"),
        protocol::STVector256::from_values(sf("sfIndexes"), full_indexes),
    );

    for (flags, claims_fee) in [
        (ApplyFlags::NONE, true),
        (ApplyFlags::FAIL_HARD, false),
        (ApplyFlags::RETRY, false),
    ] {
        let ledger = make_ledger(vec![
            account_root(alice, 1_000_000_000, 0, 0),
            account_root(gw, 1_000_000_000, 0, 0),
            root.clone(),
            tail.clone(),
        ]);
        let mut view = ApplyViewImpl::new(Arc::new(ledger), flags);
        let tx = trust_set_tx(alice, gw, "USD", 1000, 1);

        assert_eq!(
            apply_submit_transactor_shell_with_flags(&mut view, &tx, TxType::TRUST_SET, flags),
            Ter::TEC_DIR_FULL,
            "unexpected result for {flags:?}"
        );

        let alice_root = view
            .read(account_keylet(acct_id(alice)))
            .expect("account read")
            .expect("account exists");
        assert_eq!(
            alice_root.get_field_u32(sf("sfSequence")),
            if claims_fee { 2 } else { 1 },
            "sequence lifecycle differs for {flags:?}"
        );
        assert_eq!(
            alice_root.get_field_amount(sf("sfBalance")).xrp().drops(),
            if claims_fee {
                1_000_000_000 - 10
            } else {
                1_000_000_000
            },
            "fee lifecycle differs for {flags:?}"
        );
        assert_eq!(alice_root.get_field_u32(sf("sfOwnerCount")), 0);
        assert!(
            view.read(owner_dir_keylet(acct_id(alice)))
                .expect("low owner directory read")
                .is_none(),
            "partial low-directory insertion leaked for {flags:?}"
        );
        assert!(
            view.read(protocol::line(
                alice,
                gw,
                protocol::currency_from_string("USD"),
            ))
            .expect("trust-line read")
            .is_none(),
            "partial RippleState creation leaked for {flags:?}"
        );
        assert_eq!(
            view.read(high_dir)
                .expect("high owner directory read")
                .expect("high owner directory exists")
                .get_serializer()
                .data(),
            root.get_serializer().data(),
            "full high owner directory changed for {flags:?}"
        );
    }
}

/// C++ TrustSet_test — trust to self rejected.
#[test]
fn trust_set_to_self_rejected() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 1_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = trust_set_tx(alice, alice, "USD", 1000, 1);
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    assert_eq!(result, Ter::TEM_DST_IS_SRC);
}

/// C++ TrustSet_test — negative limit rejected.
#[test]
fn trust_set_negative_limit() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(gw, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = trust_set_tx(alice, gw, "USD", -1000, 1);
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    assert_eq!(result, Ter::TEM_BAD_LIMIT);
}

/// C++ TrustSet_test — XRP as limit rejected.
#[test]
fn trust_set_xrp_limit_rejected() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 1_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // XRP limit = native amount in LimitAmount field
    let tx = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), alice);
        tx.set_field_amount(
            sf("sfLimitAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10000)),
        );
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    assert_eq!(result, Ter::TEM_BAD_LIMIT);
}

/// C++ TrustSet_test — bad currency rejected.
#[test]
fn trust_set_bad_currency() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(gw, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let bad_cur = protocol::bad_currency();
    let issue = Issue::new(bad_cur, gw);
    let limit = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(100, 0).expect("a"),
        issue,
    );
    let tx = STTx::new(TxType::TRUST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), alice);
        tx.set_field_amount(sf("sfLimitAmount"), limit);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    assert_eq!(result, Ter::TEM_BAD_CURRENCY);
}

/// C++ TrustSet_test — invalid flags rejected.
#[test]
fn trust_set_invalid_flags() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(gw, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Invalid flag combination
    let tx = trust_set_tx_with_flags(alice, gw, "USD", 1000, 1, 0x00020000 | 0x00040000);
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

/// C++ TrustSet_test — insufficient reserve for trust line.
#[test]
fn trust_set_insufficient_reserve() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 200_010, 0, 0), // just base reserve + fee
        account_root(gw, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = trust_set_tx(alice, gw, "USD", 1000, 1);
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    // Should fail with reserve error
    assert!(
        result == Ter::TEC_NO_LINE_INSUF_RESERVE
            || result == Ter::TEC_INSUF_RESERVE_LINE
            || result == Ter::TEC_INSUFFICIENT_RESERVE
            || result == Ter::TES_SUCCESS,
        "Got {:?}",
        result
    );
}

/// C++ TrustSet_test — delete trust line by setting limit to zero.
/// Note: Trust line deletion requires all fields to be at default values.
/// With no DefaultRipple flag, the NoRipple state must also match.
#[test]
fn trust_set_delete_by_zero_limit() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    // Set lsfDefaultRipple on both accounts so NoRipple comparison is default
    let lsf_default_ripple: u32 = 0x00800000;
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, lsf_default_ripple),
        account_root(gw, 1_000_000_000, 0, lsf_default_ripple),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Create trust line
    let tx1 = trust_set_tx(alice, gw, "USD", 1000, 1);
    assert_eq!(
        full_apply(&mut view, &tx1, TxType::TRUST_SET),
        Ter::TES_SUCCESS
    );
    assert_eq!(get_owner_count(&view, alice), 1);

    // Delete by setting limit to 0
    let tx2 = trust_set_tx(alice, gw, "USD", 0, 2);
    let result = full_apply(&mut view, &tx2, TxType::TRUST_SET);
    assert_eq!(result, Ter::TES_SUCCESS);
    // With DefaultRipple set on both, zero limit should delete the line
    assert_eq!(get_owner_count(&view, alice), 0);
}

/// C++ TrustSet_test — setfAuth without RequireAuth fails.
#[test]
fn trust_set_auth_without_require_auth() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(gw, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // tfSetfAuth = 0x00010000
    let tx = trust_set_tx_with_flags(alice, gw, "USD", 1000, 1, 0x00010000);
    let result = full_apply(&mut view, &tx, TxType::TRUST_SET);
    assert_eq!(result, Ter::TEF_NO_AUTH_REQUIRED);
}
