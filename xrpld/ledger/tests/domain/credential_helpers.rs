use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, Ledger, LedgerHeader, credential_helpers::delete_sle};
use protocol::{
    AccountID, ApplyFlags, LedgerEntryType, STLedgerEntry, STVector256, Ter, XRPAmount,
    account_keylet, credential_keylet, get_field_by_symbol, lsfAccepted, owner_dir_keylet,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account(byte: u8) -> AccountID {
    AccountID::from_array([byte; 20])
}

fn account_raw(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn account_entry(account: AccountID, owner_count: u32) -> STLedgerEntry {
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_raw(account)).key,
    );
    sle.set_account_id(sf("sfAccount"), account);
    sle.set_field_amount(
        sf("sfBalance"),
        protocol::STAmount::from_xrp_amount(XRPAmount::from_drops(100_000_000)),
    );
    sle.set_field_u32(sf("sfSequence"), 1);
    sle.set_field_u32(sf("sfOwnerCount"), owner_count);
    sle
}

fn owner_dir_entry(owner: AccountID, index: Uint256) -> STLedgerEntry {
    let mut sle = STLedgerEntry::new(owner_dir_keylet(account_raw(owner)));
    sle.set_field_v256(
        sf("sfIndexes"),
        STVector256::from_values(sf("sfIndexes"), vec![index]),
    );
    sle.set_field_u64(sf("sfIndexNext"), 0);
    sle.set_field_u64(sf("sfIndexPrevious"), 0);
    sle
}

fn credential_entry(
    subject: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
    accepted: bool,
) -> STLedgerEntry {
    let keylet = credential_keylet(account_raw(subject), account_raw(issuer), credential_type);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Credential, keylet.key);
    sle.set_account_id(sf("sfSubject"), subject);
    sle.set_account_id(sf("sfIssuer"), issuer);
    sle.set_field_vl(sf("sfCredentialType"), credential_type);
    sle.set_field_u64(sf("sfIssuerNode"), 0);
    sle.set_field_u64(sf("sfSubjectNode"), 0);
    if accepted {
        sle.set_field_u32(sf("sfFlags"), lsfAccepted);
    }
    sle
}

fn ledger_with(entries: impl IntoIterator<Item = STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insertion should succeed");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            parent_close_time: 500,
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

#[test]
fn delete_sle_rejects_missing_issuer_root_when_issuer_owner_count_must_change() {
    let issuer = account(0x11);
    let subject = account(0x12);
    let credential = credential_entry(subject, issuer, b"kyc", false);
    let keylet = credential_keylet(account_raw(subject), account_raw(issuer), b"kyc");
    let ledger = ledger_with([
        account_entry(subject, 0),
        owner_dir_entry(issuer, keylet.key),
        owner_dir_entry(subject, keylet.key),
        credential,
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let credential = view
        .peek(keylet)
        .expect("credential read should succeed")
        .expect("credential should exist");

    assert_eq!(delete_sle(&mut view, credential), Ok(Ter::TEF_BAD_LEDGER));
}

#[test]
fn delete_sle_rejects_missing_subject_root_when_accepted_subject_owner_count_must_change() {
    let issuer = account(0x21);
    let subject = account(0x22);
    let credential = credential_entry(subject, issuer, b"kyc", true);
    let keylet = credential_keylet(account_raw(subject), account_raw(issuer), b"kyc");
    let ledger = ledger_with([
        account_entry(issuer, 0),
        owner_dir_entry(issuer, keylet.key),
        owner_dir_entry(subject, keylet.key),
        credential,
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let credential = view
        .peek(keylet)
        .expect("credential read should succeed")
        .expect("credential should exist");

    assert_eq!(delete_sle(&mut view, credential), Ok(Ter::TEF_BAD_LEDGER));
}
