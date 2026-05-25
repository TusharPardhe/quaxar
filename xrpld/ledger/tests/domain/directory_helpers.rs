use basics::base_uint::{Uint160, Uint256};
use ledger::{
    Dir, Ledger, LedgerHeader, dir_is_empty, for_each_item, for_each_item_after,
    for_each_owner_item, for_each_owner_item_after,
};
use protocol::{
    AccountID, Keylet, LedgerEntryType, STLedgerEntry, get_field_by_symbol, owner_dir_keylet,
    page_keylet,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sample_key(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn sample_owner(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn owner_root(owner: AccountID) -> Keylet {
    owner_dir_keylet(Uint160::from_slice(owner.data()).expect("account width"))
}

fn child_payload(key: Uint256) -> Vec<u8> {
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::FeeSettings, key);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), sample_key(0xA1));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    entry.set_field_u64(get_field_by_symbol("sfBaseFee"), 10);
    entry.get_serializer().data().to_vec()
}

fn directory_page_payload(
    root: Keylet,
    page: u64,
    indexes: &[Uint256],
    next: u64,
    previous: u64,
) -> Vec<u8> {
    let mut entry = STLedgerEntry::new(page_keylet(root, page));
    entry.set_field_v256(
        get_field_by_symbol("sfIndexes"),
        protocol::STVector256::from_values(get_field_by_symbol("sfIndexes"), indexes.to_vec()),
    );
    entry.set_field_u64(get_field_by_symbol("sfIndexNext"), next);
    entry.set_field_u64(get_field_by_symbol("sfIndexPrevious"), previous);
    entry.get_serializer().data().to_vec()
}

fn build_ledger(items: &[(Uint256, Vec<u8>)]) -> Ledger {
    let mut tree = MutableTree::new(1);
    for (key, payload) in items {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state map item insertion should succeed");
    }

    let state_map = SyncTree::from_root_with_type(
        tree.root(),
        SHAMapType::State,
        false,
        1,
        SyncState::Immutable,
    );

    Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            ..LedgerHeader::default()
        },
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    )
}

#[test]
fn for_each_item_walks_directory_pages_and_preserves_missing_children() {
    let owner = sample_owner(0x11);
    let root = owner_root(owner);
    let child_one = sample_key(0x21);
    let child_two = sample_key(0x22);
    let child_three = sample_key(0x23);
    let missing_child = sample_key(0x24);
    let page_one = page_keylet(root, 1);

    let ledger = build_ledger(&[
        (
            root.key,
            directory_page_payload(root, 0, &[child_one, missing_child], 1, 1),
        ),
        (
            page_one.key,
            directory_page_payload(root, 1, &[child_two, child_three], 0, 0),
        ),
        (child_one, child_payload(child_one)),
        (child_two, child_payload(child_two)),
        (child_three, child_payload(child_three)),
    ]);

    let mut seen = Vec::new();
    for_each_item(&ledger, root, |sle| {
        seen.push(sle.map(|entry| *entry.key()));
    })
    .expect("directory walk should succeed");

    assert_eq!(
        seen,
        vec![Some(child_one), None, Some(child_two), Some(child_three)]
    );
}

#[test]
fn for_each_owner_item_uses_owner_directory_root() {
    let owner = sample_owner(0x12);
    let root = owner_root(owner);
    let child = sample_key(0x31);
    let ledger = build_ledger(&[
        (root.key, directory_page_payload(root, 0, &[child], 0, 0)),
        (child, child_payload(child)),
    ]);

    let mut seen = Vec::new();
    for_each_owner_item(&ledger, owner, |sle| {
        seen.push(sle.map(|entry| *entry.key()));
    })
    .expect("owner directory walk should succeed");

    assert_eq!(seen, vec![Some(child)]);
}

#[test]
fn for_each_item_after_matches_current_cpp_found_and_limit_rules() {
    let owner = sample_owner(0x13);
    let root = owner_root(owner);
    let child_one = sample_key(0x41);
    let child_two = sample_key(0x42);
    let child_three = sample_key(0x43);
    let child_four = sample_key(0x44);

    let ledger = build_ledger(&[
        (
            root.key,
            directory_page_payload(root, 0, &[child_one, child_two], 1, 1),
        ),
        (
            page_keylet(root, 1).key,
            directory_page_payload(root, 1, &[child_three, child_four], 0, 0),
        ),
        (child_one, child_payload(child_one)),
        (child_two, child_payload(child_two)),
        (child_three, child_payload(child_three)),
        (child_four, child_payload(child_four)),
    ]);

    let mut seen = Vec::new();
    let found = for_each_item_after(&ledger, root, child_two, 0, 2, |sle| {
        seen.push(sle.map(|entry| *entry.key()));
        true
    })
    .expect("directory walk after should succeed");

    assert!(found);
    assert_eq!(seen, vec![Some(child_three), Some(child_four)]);

    let mut not_found_seen = Vec::new();
    let found = for_each_owner_item_after(&ledger, owner, sample_key(0x99), 0, 2, |sle| {
        not_found_seen.push(sle.map(|entry| *entry.key()));
        true
    })
    .expect("owner directory walk after should succeed");

    assert!(!found);
    assert!(not_found_seen.is_empty());
}

#[test]
fn for_each_item_after_missing_root_matches_current_cpp_return_shape() {
    let owner = sample_owner(0x14);
    let root = owner_root(owner);
    let ledger = build_ledger(&[]);

    let mut seen = Vec::new();
    let after_zero = for_each_item_after(&ledger, root, Uint256::zero(), 0, 1, |sle| {
        seen.push(sle.map(|entry| *entry.key()));
        true
    })
    .expect("missing root walk should not error");
    assert!(after_zero);
    assert!(seen.is_empty());

    let after_nonzero = for_each_item_after(&ledger, root, sample_key(0x55), 0, 1, |_| true)
        .expect("missing root walk should not error");
    assert!(!after_nonzero);
}

#[test]
fn dir_is_empty_matches_anchor_page_rules() {
    let owner = sample_owner(0x15);
    let root = owner_root(owner);
    let child = sample_key(0x61);

    let missing = build_ledger(&[]);
    assert!(dir_is_empty(&missing, root).expect("missing directory check should succeed"));

    let empty_root = build_ledger(&[(root.key, directory_page_payload(root, 0, &[], 0, 0))]);
    assert!(dir_is_empty(&empty_root, root).expect("empty root check should succeed"));

    let later_page = build_ledger(&[
        (root.key, directory_page_payload(root, 0, &[], 1, 1)),
        (
            page_keylet(root, 1).key,
            directory_page_payload(root, 1, &[child], 0, 0),
        ),
        (child, child_payload(child)),
    ]);
    assert!(!dir_is_empty(&later_page, root).expect("later page check should succeed"));

    let non_empty_root = build_ledger(&[
        (root.key, directory_page_payload(root, 0, &[child], 0, 0)),
        (child, child_payload(child)),
    ]);
    assert!(!dir_is_empty(&non_empty_root, root).expect("non-empty root check should succeed"));
}

#[test]
fn dir_iterator_walks_items_and_supports_page_jumps() {
    let owner = sample_owner(0x16);
    let root = owner_root(owner);
    let child_one = sample_key(0x71);
    let child_two = sample_key(0x72);
    let child_three = sample_key(0x73);

    let ledger = build_ledger(&[
        (
            root.key,
            directory_page_payload(root, 0, &[child_one, child_two], 1, 1),
        ),
        (
            page_keylet(root, 1).key,
            directory_page_payload(root, 1, &[child_three], 0, 0),
        ),
        (child_one, child_payload(child_one)),
        (child_two, child_payload(child_two)),
        (child_three, child_payload(child_three)),
    ]);

    let dir = Dir::new(&ledger, root).expect("directory should build");
    let end = dir.end();
    let mut it = dir.begin();

    assert_ne!(it, end);
    assert_eq!(it.page(), root);
    assert_eq!(it.page_size(), 2);
    assert_eq!(
        *it.current()
            .expect("current item lookup should succeed")
            .expect("first child should exist")
            .key(),
        child_one
    );

    it.advance().expect("advance within page should succeed");
    assert_eq!(
        *it.current()
            .expect("current item lookup should succeed")
            .expect("second child should exist")
            .key(),
        child_two
    );

    it.next_page().expect("next page should succeed");
    assert_eq!(it.page(), page_keylet(root, 1));
    assert_eq!(it.page_size(), 1);
    assert_eq!(it.index(), child_three);
    assert_eq!(
        *it.current()
            .expect("current item lookup should succeed")
            .expect("third child should exist")
            .key(),
        child_three
    );

    it.advance().expect("advance to end should succeed");
    assert!(it.is_end());
    assert_eq!(it, end);
}
