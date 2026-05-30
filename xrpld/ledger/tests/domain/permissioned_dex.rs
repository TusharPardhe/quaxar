use basics::base_uint::{Uint160, Uint256};
use ledger::{Ledger, LedgerHeader, permissioned_dex_helpers::offer_in_domain};
use protocol::{
    AccountID, LedgerEntryType, Rules, STArray, STLedgerEntry, STObject, feature_id,
    get_field_by_symbol, lsfHybrid, offer_keylet, permissioned_domain_keylet_from_id,
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

fn ledger_with(entries: impl IntoIterator<Item = STLedgerEntry>, features: &[Uint256]) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insertion should succeed");
    }

    let mut ledger = Ledger::from_maps(
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
    );
    ledger.set_rules(Rules::new(features.iter().copied()));
    ledger
}

fn permissioned_domain(domain: Uint256, owner: AccountID) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::PermissionedDomain,
        permissioned_domain_keylet_from_id(domain).key,
    );
    entry.set_account_id(sf("sfOwner"), owner);
    entry.set_field_array(
        sf("sfAcceptedCredentials"),
        STArray::new(sf("sfAcceptedCredentials")),
    );
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn additional_books(count: usize) -> STArray {
    let mut array = STArray::new(sf("sfAdditionalBooks"));
    for n in 0..count {
        let mut book = STObject::make_inner_object(sf("sfBook"));
        book.set_field_h256(sf("sfBookDirectory"), Uint256::from_u64(100 + n as u64));
        book.set_field_u64(sf("sfBookNode"), n as u64);
        array.push_back(book);
    }
    array
}

fn domain_offer(
    owner: AccountID,
    sequence: u32,
    domain: Uint256,
    additional_books_len: Option<usize>,
) -> STLedgerEntry {
    let keylet = offer_keylet(account_raw(owner), sequence);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, keylet.key);
    entry.set_account_id(sf("sfAccount"), owner);
    entry.set_field_u32(sf("sfSequence"), sequence);
    entry.set_field_u32(sf("sfFlags"), lsfHybrid);
    entry.set_field_h256(sf("sfDomainID"), domain);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry.set_field_u64(sf("sfBookNode"), 0);
    entry.set_field_h256(sf("sfBookDirectory"), Uint256::from_u64(10));
    if let Some(len) = additional_books_len {
        entry.set_field_array(sf("sfAdditionalBooks"), additional_books(len));
    }
    entry
}

#[test]
fn offer_in_domain_requires_hybrid_additional_books_presence() {
    let owner = account(0x21);
    let domain = Uint256::from_u64(21);
    let ledger = ledger_with(
        [
            permissioned_domain(domain, owner),
            domain_offer(owner, 1, domain, None),
        ],
        &[],
    );

    assert!(
        !offer_in_domain(&ledger, &offer_keylet(account_raw(owner), 1).key, &domain)
            .expect("domain check should succeed")
    );
}

#[test]
fn offer_in_domain_legacy_allows_present_additional_books_with_any_size() {
    let owner = account(0x22);
    let domain = Uint256::from_u64(22);
    let ledger = ledger_with(
        [
            permissioned_domain(domain, owner),
            domain_offer(owner, 1, domain, Some(0)),
        ],
        &[],
    );

    assert!(
        offer_in_domain(&ledger, &offer_keylet(account_raw(owner), 1).key, &domain)
            .expect("domain check should succeed")
    );
}

#[test]
fn offer_in_domain_fix_cleanup_3_1_3_requires_exactly_one_additional_book() {
    let owner = account(0x23);
    let domain = Uint256::from_u64(23);

    for (sequence, additional_books_len, expected) in
        [(1, Some(0), false), (2, Some(1), true), (3, Some(2), false)]
    {
        let ledger = ledger_with(
            [
                permissioned_domain(domain, owner),
                domain_offer(owner, sequence, domain, additional_books_len),
            ],
            &[feature_id("fixCleanup3_1_3")],
        );

        assert_eq!(
            offer_in_domain(
                &ledger,
                &offer_keylet(account_raw(owner), sequence).key,
                &domain
            )
            .expect("domain check should succeed"),
            expected,
            "sequence {sequence} with {additional_books_len:?} additional books"
        );
    }
}
