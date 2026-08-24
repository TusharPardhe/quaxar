use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{
    ApplyView, ApplyViewImpl, Ledger, LedgerHeader, ReadView,
    credential_helpers::{authorized_deposit_preauth, delete_sle, verify_deposit_preauth},
};
use protocol::{
    AccountID, ApplyFlags, LedgerEntryType, Rules, STLedgerEntry, STTx, STVector256, Ter, TxType,
    XRPAmount, account_keylet, credential_keylet, deposit_preauth_credentials_keylet,
    get_field_by_symbol, lsfAccepted, lsfDepositAuth, owner_dir_keylet, sha512_half_slices,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

#[test]
fn credential_deposit_preauth_sorts_pairs_before_hashing() {
    let subject = account(0x20);
    let destination = account(0x30);
    let first_issuer = account(0x01);
    let second_issuer = account(0x03);
    let first_type = b"a";
    let second_type = b"b";
    let first = credential_entry(subject, first_issuer, first_type, true);
    let second = credential_entry(subject, second_issuer, second_type, true);

    // Pair order is first_issuer then second_issuer, while these particular
    // SHA-512Half values sort in the opposite order. This catches the exact
    // consensus bug seen in the live PaymentChannelClaim transactions.
    let hashes = vec![
        sha512_half_slices(&[first_issuer.data(), first_type]),
        sha512_half_slices(&[second_issuer.data(), second_type]),
    ];
    assert!(hashes[0] > hashes[1]);

    let preauth_keylet = deposit_preauth_credentials_keylet(account_raw(destination), &hashes);
    let mut preauth = STLedgerEntry::new(preauth_keylet);
    preauth.set_account_id(sf("sfAccount"), destination);
    let mut destination_entry = account_entry(destination, 0);
    destination_entry.set_field_u32(sf("sfFlags"), lsfDepositAuth);

    let ids = STVector256::from_values(sf("sfCredentialIDs"), vec![*second.key(), *first.key()]);
    let tx = STTx::new(TxType::PAYCHAN_CLAIM, |object| {
        object.set_account_id(sf("sfAccount"), subject);
        object.set_field_v256(sf("sfCredentialIDs"), ids);
    });
    let ledger = ledger_with([
        account_entry(subject, 0),
        destination_entry,
        first,
        second,
        preauth,
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let destination_sle = view
        .read(account_keylet(account_raw(destination)))
        .expect("destination read")
        .expect("destination exists");

    assert_eq!(
        verify_deposit_preauth(
            &tx,
            &mut view,
            &subject,
            &destination,
            Some(destination_sle.as_ref()),
        ),
        Ok(Ter::TES_SUCCESS)
    );
}

#[test]
fn credential_deposit_preauth_rejects_zero_ids_after_cleanup_3_4() {
    let destination = account(0x30);
    let mut ledger = ledger_with([account_entry(destination, 0)]);
    ledger.set_rules(Rules::new([protocol::feature_id("fixCleanup3_4_0")]));
    let view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let ids = STVector256::from_values(sf("sfCredentialIDs"), vec![Uint256::zero()]);

    assert_eq!(
        authorized_deposit_preauth(&view, &ids, &destination),
        Ok(Ter::TEF_INTERNAL)
    );
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
fn credential_read_view_preclaims_preserve_ter_order_and_defaulting() {
    use ledger::credential_helpers::{
        credential_accept_preclaim, credential_create_preclaim, credential_delete_preclaim,
    };

    let issuer = account(0x41);
    let subject = account(0x42);
    let credential_type = b"kyc";
    let credential = credential_entry(subject, issuer, credential_type, false);

    let missing_subject = ledger_with([credential.clone()]);
    assert_eq!(
        credential_create_preclaim(&missing_subject, subject, issuer, credential_type),
        Ok(Ter::TEC_NO_TARGET),
        "CredentialCreate returns tecNO_TARGET before considering a duplicate"
    );

    let missing_issuer = ledger_with([account_entry(subject, 0), credential.clone()]);
    assert_eq!(
        credential_accept_preclaim(&missing_issuer, subject, issuer, credential_type),
        Ok(Ter::TEC_NO_ISSUER),
        "CredentialAccept returns tecNO_ISSUER before considering its credential"
    );

    let no_credential = ledger_with([account_entry(subject, 0), account_entry(issuer, 0)]);
    assert_eq!(
        credential_create_preclaim(&no_credential, subject, issuer, credential_type),
        Ok(Ter::TES_SUCCESS)
    );
    assert_eq!(
        credential_accept_preclaim(&no_credential, subject, issuer, credential_type),
        Ok(Ter::TEC_NO_ENTRY)
    );
    assert_eq!(
        credential_delete_preclaim(&no_credential, issuer, Some(subject), None, credential_type,),
        Ok(Ter::TEC_NO_ENTRY),
        "CredentialDelete defaults an omitted issuer to the transaction account"
    );

    let accepted = ledger_with([
        account_entry(subject, 0),
        account_entry(issuer, 0),
        credential.clone(),
    ]);
    assert_eq!(
        credential_create_preclaim(&accepted, subject, issuer, credential_type),
        Ok(Ter::TEC_DUPLICATE)
    );
    assert_eq!(
        credential_accept_preclaim(&accepted, subject, issuer, credential_type),
        Ok(Ter::TES_SUCCESS)
    );
    assert_eq!(
        credential_delete_preclaim(&accepted, issuer, Some(subject), None, credential_type,),
        Ok(Ter::TES_SUCCESS)
    );

    let accepted_credential = credential_entry(subject, issuer, credential_type, true);
    let already_accepted = ledger_with([
        account_entry(subject, 0),
        account_entry(issuer, 0),
        accepted_credential,
    ]);
    assert_eq!(
        credential_accept_preclaim(&already_accepted, subject, issuer, credential_type),
        Ok(Ter::TEC_DUPLICATE)
    );
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
