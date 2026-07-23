use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{
    ApplyView, ApplyViewImpl, Ledger, LedgerHeader, ReadView,
    credential_helpers::{delete_sle, verify_deposit_preauth},
};
use protocol::{
    AccountID, ApplyFlags, LedgerEntryType, Rules, STLedgerEntry, STTx, STVector256, Ter, TxType,
    XRPAmount, account_keylet, credential_keylet, get_field_by_symbol, lsfAccepted,
    owner_dir_keylet,
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
fn expired_credential_deletion_failure_tracks_fix_cleanup_3_1_3() {
    let issuer = account(0x31);
    let subject = account(0x32);
    let destination = account(0x33);
    let credential_type = b"kyc";
    let mut credential = credential_entry(subject, issuer, credential_type, false);
    credential.set_field_u32(sf("sfExpiration"), 499);
    let credential_key = *credential.key();
    let credential_ids = STVector256::from_values(sf("sfCredentialIDs"), vec![credential_key]);
    let tx = STTx::new(TxType::PAYMENT, |object| {
        object.set_account_id(sf("sfAccount"), subject);
        object.set_account_id(sf("sfDestination"), destination);
        object.set_field_v256(sf("sfCredentialIDs"), credential_ids);
    });

    for amendment_enabled in [false, true] {
        // Deliberately omit the issuer AccountRoot. The expired record can be
        // found, but removeExpired cannot finish the issuer owner-count update.
        let mut ledger = ledger_with([
            account_entry(subject, 0),
            account_entry(destination, 0),
            owner_dir_entry(issuer, credential_key),
            owner_dir_entry(subject, credential_key),
            credential.clone(),
        ]);
        if amendment_enabled {
            ledger.set_rules(Rules::new([protocol::fix_cleanup_3_1_3()]));
        }
        let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

        let result = verify_deposit_preauth(&tx, &mut view, &subject, &destination, None)
            .expect("credential verification should return a TER");
        assert_eq!(
            result,
            if amendment_enabled {
                Ter::TEC_INTERNAL
            } else {
                Ter::TEC_EXPIRED
            },
            "fixCleanup3_1_3 must make expired-credential deletion failures observable"
        );
        let issuer_dir = view
            .read(owner_dir_keylet(account_raw(issuer)))
            .expect("issuer directory read")
            .expect("issuer directory exists");
        assert!(
            issuer_dir
                .get_field_v256(sf("sfIndexes"))
                .value()
                .contains(&credential_key)
        );
        assert!(
            view.read(credential_keylet(
                account_raw(subject),
                account_raw(issuer),
                credential_type,
            ))
            .expect("credential read")
            .is_some(),
            "a failed cleanup must not erase the credential"
        );
    }
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

    assert_eq!(delete_sle(&mut view, credential), Ok(Ter::TEC_INTERNAL));
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

    assert_eq!(delete_sle(&mut view, credential), Ok(Ter::TEC_INTERNAL));
    assert!(
        view.read(owner_dir_keylet(account_raw(issuer)))
            .expect("issuer directory read")
            .is_none_or(|directory| !directory
                .get_field_v256(sf("sfIndexes"))
                .value()
                .contains(&keylet.key)),
        "rippled removes the issuer directory entry before it discovers the missing accepted subject"
    );
}

#[test]
fn verify_valid_domain_rejects_missing_issuer_root_when_expired_cleanup_fails() {
    use ledger::credential_helpers::verify_valid_domain;
    let domain_id = Uint256::from_array([0xD1; 32]);
    let subject = account(0x13);
    let issuer = account(0x14);
    let credential_type = b"domain_access";
    let credential_key =
        credential_keylet(account_raw(subject), account_raw(issuer), credential_type).key;

    // Create an expired credential
    let mut credential = credential_entry(subject, issuer, credential_type, false);
    credential.set_field_u32(sf("sfExpiration"), 499);

    let mut sle_pd = STLedgerEntry::from_type_and_key(
        LedgerEntryType::PermissionedDomain,
        protocol::permissioned_domain_keylet_from_id(domain_id).key,
    );
    let mut obj = protocol::STObject::new(sf("sfCredential"));
    obj.set_account_id(sf("sfIssuer"), issuer);
    obj.set_field_vl(sf("sfCredentialType"), credential_type);

    let mut arr = protocol::STArray::new(sf("sfAcceptedCredentials"));
    arr.push_back(obj);
    sle_pd.set_field_array(sf("sfAcceptedCredentials"), arr);

    for amendment_enabled in [false, true] {
        let mut ledger = ledger_with([
            account_entry(subject, 0),
            owner_dir_entry(issuer, credential_key),
            owner_dir_entry(subject, credential_key),
            credential.clone(),
            sle_pd.clone(),
        ]);
        if amendment_enabled {
            ledger.set_rules(Rules::new([protocol::fix_cleanup_3_1_3()]));
        }
        let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

        let result = verify_valid_domain(&mut view, &subject, domain_id)
            .expect("credential verification should return a TER");
        assert_eq!(
            result,
            if amendment_enabled {
                Ter::TEC_INTERNAL
            } else {
                Ter::TEC_EXPIRED
            },
            "fixCleanup3_1_3 must make expired-credential deletion failures observable in verify_valid_domain"
        );
    }
}
