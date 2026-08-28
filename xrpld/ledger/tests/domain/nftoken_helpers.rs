use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{
    ApplyViewImpl, Ledger, LedgerHeader, ReadView, nftoken_helpers::repair_nftoken_directory_links,
};
use protocol::{
    AccountID, ApplyFlags, STArray, STLedgerEntry, get_field_by_symbol, nft_page_keylet,
    nft_page_max_keylet, nft_page_min_keylet,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn owner() -> AccountID {
    AccountID::from_array([0x44; 20])
}

fn owner_raw() -> Uint160 {
    Uint160::from_slice(owner().data()).expect("account width")
}

fn page(keylet: protocol::Keylet, token: u64) -> STLedgerEntry {
    let mut token_object = protocol::STObject::make_inner_object(sf("sfNFToken"));
    token_object.set_field_h256(sf("sfNFTokenID"), Uint256::from_u64(token));
    let mut tokens = STArray::new(sf("sfNFTokens"));
    tokens.push_back(token_object);
    let mut page = STLedgerEntry::new(keylet);
    page.set_field_array(sf("sfNFTokens"), tokens);
    page
}

fn ledger_with(entries: impl IntoIterator<Item = STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insertion");
    }
    Ledger::from_maps(
        LedgerHeader::default(),
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

fn read_page(view: &ApplyViewImpl<Ledger>, keylet: protocol::Keylet) -> Arc<STLedgerEntry> {
    view.read(keylet).expect("page read").expect("page exists")
}

#[test]
fn repairs_every_adjacent_page_and_clears_terminal_forward_link() {
    let min = nft_page_min_keylet(owner_raw());
    let first_key = nft_page_keylet(min, Uint256::from_u64(1));
    // repairNFTokenDirectoryLinks asks succ(current.next()), so leave a gap
    // just as real NFToken page boundaries normally do.
    let middle_key = nft_page_keylet(min, Uint256::from_u64(3));
    let last_key = nft_page_max_keylet(owner_raw());
    let mut first = page(first_key, 1);
    let middle = page(middle_key, 2);
    let mut last = page(last_key, 3);
    first.set_field_h256(sf("sfPreviousPageMin"), Uint256::from_u64(9));
    last.set_field_h256(sf("sfNextPageMin"), Uint256::from_u64(9));
    let ledger = ledger_with([first, middle, last]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    assert_eq!(
        repair_nftoken_directory_links(&mut view, &owner()),
        Ok(true)
    );

    let first = read_page(&view, first_key);
    let middle = read_page(&view, middle_key);
    let last = read_page(&view, last_key);
    assert!(!first.is_field_present(sf("sfPreviousPageMin")));
    assert_eq!(first.get_field_h256(sf("sfNextPageMin")), middle_key.key);
    assert_eq!(
        middle.get_field_h256(sf("sfPreviousPageMin")),
        first_key.key
    );
    assert_eq!(middle.get_field_h256(sf("sfNextPageMin")), last_key.key);
    assert_eq!(last.get_field_h256(sf("sfPreviousPageMin")), middle_key.key);
    assert!(!last.is_field_present(sf("sfNextPageMin")));
}

#[test]
fn relocates_a_noncanonical_terminal_page_to_the_max_key() {
    let min = nft_page_min_keylet(owner_raw());
    let first_key = nft_page_keylet(min, Uint256::from_u64(1));
    let bad_last_key = nft_page_keylet(min, Uint256::from_u64(3));
    let canonical_last_key = nft_page_max_keylet(owner_raw());
    let first = page(first_key, 1);
    let bad_last = page(bad_last_key, 2);
    let ledger = ledger_with([first, bad_last]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    assert_eq!(
        repair_nftoken_directory_links(&mut view, &owner()),
        Ok(true)
    );

    assert!(view.read(bad_last_key).expect("old page read").is_none());
    let first = read_page(&view, first_key);
    let last = read_page(&view, canonical_last_key);
    assert_eq!(
        first.get_field_h256(sf("sfNextPageMin")),
        canonical_last_key.key
    );
    assert_eq!(last.get_field_h256(sf("sfPreviousPageMin")), first_key.key);
    assert_eq!(
        last.get_field_array(sf("sfNFTokens"))
            .get(0)
            .expect("terminal token")
            .get_field_h256(sf("sfNFTokenID")),
        Uint256::from_u64(2)
    );
}
