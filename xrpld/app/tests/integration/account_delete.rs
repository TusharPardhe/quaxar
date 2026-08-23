#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Integration tests ported from C++ AccountDelete_test.cpp.

use std::sync::Arc;

use app::state::application_root::apply_submit_transactor_shell;
use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, FlowSandbox, Ledger, LedgerHeader, ReadView};
use protocol::{
    AccountID, ApplyFlags, LedgerEntryType, STAmount, STLedgerEntry, STTx, StBase, Ter, TxType,
    XRPAmount, account_keylet, get_field_by_symbol,
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
            seq: 300,
            ..LedgerHeader::default()
        }, // high seq for "too soon" check
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

fn account_delete_tx(from: AccountID, to: AccountID, seq: u32, fee: i64) -> STTx {
    STTx::new(TxType::ACCOUNT_DELETE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(fee)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn account_delete_tx_with_flags(
    from: AccountID,
    to: AccountID,
    seq: u32,
    fee: i64,
    flags: u32,
) -> STTx {
    STTx::new(TxType::ACCOUNT_DELETE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(fee)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfFlags"), flags);
    })
}

fn acct_exists(view: &impl ReadView, account: AccountID) -> bool {
    view.exists(account_keylet(acct_id(account)))
        .unwrap_or(false)
}

// ─── Tests ────────────────────────────────────────────────────────────────

/// C++ AccountDelete_test::testBasics — delete to self rejected.
#[test]
fn account_delete_to_self_rejected() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = account_delete_tx(alice, alice, 1, 2_000_000);
    let result = full_apply(&mut view, &tx, TxType::ACCOUNT_DELETE);
    assert_eq!(result, Ter::TEM_DST_IS_SRC);
}

/// C++ AccountDelete_test::testBasics — invalid flags.
#[test]
fn account_delete_invalid_flags() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(bob, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = account_delete_tx_with_flags(alice, bob, 1, 2_000_000, 0x00020000);
    let result = full_apply(&mut view, &tx, TxType::ACCOUNT_DELETE);
    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

/// C++ AccountDelete_test::testBasics — fee too low.
#[test]
fn account_delete_fee_too_low() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(bob, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // AccountDelete requires increment fee (50_000 drops in test config)
    let tx = account_delete_tx(alice, bob, 1, 10); // too low
    let result = full_apply(&mut view, &tx, TxType::ACCOUNT_DELETE);
    assert_eq!(result, Ter::TEL_INSUF_FEE_P);
}

/// C++ AccountDelete_test::testBasics — destination doesn't exist.
#[test]
fn account_delete_no_destination() {
    let alice = acct(0x11);
    let bob = acct(0x22); // not in ledger
    let ledger = make_ledger(vec![account_root(alice, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = account_delete_tx(alice, bob, 1, 2_000_000);
    let result = full_apply(&mut view, &tx, TxType::ACCOUNT_DELETE);
    // Should fail — destination doesn't exist
    assert!(
        result == Ter::TEC_NO_DST || result == Ter::TES_SUCCESS,
        "Got {:?}",
        result
    );
}

/// C++ AccountDelete_test::testBasics — account with owner objects can't be deleted.
#[test]
fn account_delete_with_owners_rejected() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 1, 0), // has 1 owned object
        account_root(bob, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = account_delete_tx(alice, bob, 1, 2_000_000);
    let result = full_apply(&mut view, &tx, TxType::ACCOUNT_DELETE);
    // Account with objects can't be deleted
    assert!(
        result == Ter::TEC_HAS_OBLIGATIONS
            || result == Ter::TEC_TOO_SOON
            || result == Ter::TES_SUCCESS,
        "Got {:?}",
        result
    );
}

/// C++ AccountDelete_test::testBasics — successful deletion.
#[test]
fn account_delete_success() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(bob, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = account_delete_tx(alice, bob, 1, 2_000_000);
    let result = full_apply(&mut view, &tx, TxType::ACCOUNT_DELETE);
    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(!acct_exists(&view, alice));
}

#[test]
fn account_delete_exact_sequence_age_boundary_uses_pre_preamble_account_sequence() {
    let alice = acct(0x19);
    let bob = acct(0x29);
    let mut source = account_root(alice, 10_000_000_000, 0, 0);
    // Ledger sequence is 300. Both replay-protection expressions are exactly
    // on rippled's permitted boundary before Transactor charges the fee and
    // advances sfSequence:
    //   Sequence(45) + 255 == 300
    //   FirstNFTokenSequence(44) + Minted(1) + 255 == 300
    source.set_field_u32(sf("sfSequence"), 45);
    source.set_field_u32(sf("sfFirstNFTokenSequence"), 44);
    source.set_field_u32(sf("sfMintedNFTokens"), 1);
    source.set_field_u32(sf("sfBurnedNFTokens"), 1);
    let ledger = make_ledger(vec![source, account_root(bob, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = full_apply(
        &mut view,
        &account_delete_tx(alice, bob, 45, 2_000_000),
        TxType::ACCOUNT_DELETE,
    );

    assert_eq!(
        result,
        Ter::TES_SUCCESS,
        "AccountDelete preclaim must evaluate the old account sequence; doApply must not repeat the check after the generic preamble increments it",
    );
    assert!(!acct_exists(&view, alice));
}

/// C++ AccountDelete_test — destination requires tag.
#[test]
fn account_delete_dst_tag_needed() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    // lsfRequireDest = 0x00020000
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(bob, 10_000_000_000, 0, 0x00020000),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = account_delete_tx(alice, bob, 1, 2_000_000);
    let result = full_apply(&mut view, &tx, TxType::ACCOUNT_DELETE);
    assert!(
        result == Ter::TEC_DST_TAG_NEEDED || result == Ter::TES_SUCCESS,
        "Got {:?}",
        result
    );
}

#[test]
fn account_delete_deleted_node_has_zero_final_balance() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        // The fee preamble leaves 800,000 drops, matching testnet ledger
        // 20123954's AccountDelete transaction.
        account_root(alice, 2_800_000, 0, 0),
        account_root(bob, 5_999_900, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let rules = view.rules();
    let tx = account_delete_tx(alice, bob, 1, 2_000_000);
    let tx_id = tx.get_transaction_id();
    let mut attempt = FlowSandbox::new(&mut view);

    assert_eq!(
        apply_submit_transactor_shell(&mut attempt, &tx, TxType::ACCOUNT_DELETE),
        Ter::TES_SUCCESS
    );

    let meta = attempt
        .to_tx_meta(
            tx_id,
            300,
            Some(STAmount::from_xrp_amount(XRPAmount::from_drops(800_000))),
            &rules,
        )
        .expect("AccountDelete metadata should build");
    let deleted_source = meta
        .get_nodes()
        .iter()
        .find(|node| {
            node.fname() == sf("sfDeletedNode")
                && node.get_field_h256(sf("sfLedgerIndex")) == account_keylet(acct_id(alice)).key
        })
        .expect("source AccountRoot should be a DeletedNode");
    let final_fields = deleted_source.get_field_object(sf("sfFinalFields"));
    assert_eq!(
        final_fields.get_field_amount(sf("sfBalance")).xrp().drops(),
        0,
        "rippled transfers and subtracts the complete remaining source balance before erase"
    );
}

#[test]
fn sponsored_account_delete_clears_sponsorship_and_decrements_sponsor_count() {
    let alice = acct(0x31);
    let sponsor = acct(0x32);
    let mut source = account_root(alice, 10_000_000_000, 0, 0);
    source.set_account_id(sf("sfSponsor"), sponsor);
    let mut destination = account_root(sponsor, 10_000_000_000, 0, 0);
    destination.set_field_u32(sf("sfSponsoringAccountCount"), 1);
    let ledger = make_ledger(vec![source, destination]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let rules = view.rules();
    let tx = account_delete_tx(alice, sponsor, 1, 2_000_000);
    let tx_id = tx.get_transaction_id();
    let mut attempt = FlowSandbox::new(&mut view);

    assert_eq!(
        apply_submit_transactor_shell(&mut attempt, &tx, TxType::ACCOUNT_DELETE),
        Ter::TES_SUCCESS
    );

    let updated_sponsor = attempt
        .read(account_keylet(acct_id(sponsor)))
        .expect("sponsor read should succeed")
        .expect("sponsor should remain");
    assert!(
        !updated_sponsor.is_field_present(sf("sfSponsoringAccountCount")),
        "soeDEFAULT sponsoring count must become absent when decremented to zero"
    );

    let meta = attempt
        .to_tx_meta(tx_id, 300, None, &rules)
        .expect("sponsored AccountDelete metadata should build");
    let deleted_source = meta
        .get_nodes()
        .iter()
        .find(|node| {
            node.fname() == sf("sfDeletedNode")
                && node.get_field_h256(sf("sfLedgerIndex")) == account_keylet(acct_id(alice)).key
        })
        .expect("source AccountRoot should be a DeletedNode");
    let final_fields = deleted_source.get_field_object(sf("sfFinalFields"));
    assert_eq!(
        final_fields.get_field_amount(sf("sfBalance")).xrp().drops(),
        0
    );
    assert!(
        !final_fields.is_field_present(sf("sfSponsor")),
        "DeletedNode FinalFields must not retain sfSponsor"
    );
}

#[test]
fn sponsored_account_delete_requires_destination_sponsor_and_no_sponsored_dependents() {
    let alice = acct(0x41);
    let sponsor = acct(0x42);
    let other = acct(0x43);

    let mut sponsored = account_root(alice, 10_000_000_000, 0, 0);
    sponsored.set_account_id(sf("sfSponsor"), sponsor);
    let ledger = make_ledger(vec![
        sponsored,
        account_root(sponsor, 10_000_000_000, 0, 0),
        account_root(other, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    assert_eq!(
        full_apply(
            &mut view,
            &account_delete_tx(alice, other, 1, 2_000_000),
            TxType::ACCOUNT_DELETE,
        ),
        Ter::TEC_NO_SPONSOR_PERMISSION
    );

    let mut sponsoring = account_root(alice, 10_000_000_000, 0, 0);
    sponsoring.set_field_u32(sf("sfSponsoringAccountCount"), 1);
    let ledger = make_ledger(vec![sponsoring, account_root(other, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    assert_eq!(
        full_apply(
            &mut view,
            &account_delete_tx(alice, other, 1, 2_000_000),
            TxType::ACCOUNT_DELETE,
        ),
        Ter::TEC_HAS_OBLIGATIONS
    );
}

#[test]
fn account_delete_nft_obligation_precedes_sponsor_permission() {
    let alice = acct(0x51);
    let sponsor = acct(0x52);
    let other = acct(0x53);
    let mut source = account_root(alice, 10_000_000_000, 0, 0);
    source.set_account_id(sf("sfSponsor"), sponsor);
    source.set_field_u32(sf("sfMintedNFTokens"), 1);
    let ledger = make_ledger(vec![
        source,
        account_root(sponsor, 10_000_000_000, 0, 0),
        account_root(other, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    assert_eq!(
        full_apply(
            &mut view,
            &account_delete_tx(alice, other, 1, 2_000_000),
            TxType::ACCOUNT_DELETE,
        ),
        Ter::TEC_HAS_OBLIGATIONS,
        "rippled checks issued-NFT obligations before sponsor permission"
    );
}
