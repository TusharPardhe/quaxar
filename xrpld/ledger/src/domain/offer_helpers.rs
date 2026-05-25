//! the reference implementation parity — offer deletion from the DEX.

use crate::views::apply_view::ApplyView;
use crate::views::read_view::ViewError;
use crate::{adjust_owner_count, dir_remove};
use basics::base_uint::Uint160;
use protocol::{
    STLedgerEntry, Ter, account_keylet, directory_node_keylet, get_field_by_symbol, lsfHybrid,
    owner_dir_keylet,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_uint160(account: protocol::AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

/// Removes an offer from the ledger, cleaning up owner directory, book
/// directory, and any additional book directories for hybrid offers.
///
pub fn offer_delete(view: &mut dyn ApplyView, sle: Arc<STLedgerEntry>) -> Result<Ter, ViewError> {
    let offer_index = *sle.key();
    let owner = sle.get_account_id(sf("sfAccount"));

    let book_directory = sle.get_field_h256(sf("sfBookDirectory"));

    // Remove from owner directory
    let owner_node = sle.get_field_u64(sf("sfOwnerNode"));
    if !dir_remove(
        view,
        &owner_dir_keylet(to_uint160(owner)),
        owner_node,
        offer_index,
        false,
    )? {
        return Ok(Ter::TEF_BAD_LEDGER);
    }

    // Remove from book directory
    let book_node = sle.get_field_u64(sf("sfBookNode"));
    if !dir_remove(
        view,
        &directory_node_keylet(book_directory),
        book_node,
        offer_index,
        false,
    )? {
        return Ok(Ter::TEF_BAD_LEDGER);
    }

    // Handle hybrid offers with additional book directories
    if sle.is_field_present(sf("sfAdditionalBooks")) {
        debug_assert!(
            sle.is_flag(lsfHybrid) && sle.is_field_present(sf("sfDomainID")),
            "xrpl::offerDelete : should be a hybrid domain offer"
        );

        let additional_books = sle.get_field_array(sf("sfAdditionalBooks"));
        for book_dir_entry in additional_books.iter() {
            let dir_index = book_dir_entry.get_field_h256(sf("sfBookDirectory"));
            let dir_node = book_dir_entry.get_field_u64(sf("sfBookNode"));
            if !dir_remove(
                view,
                &directory_node_keylet(dir_index),
                dir_node,
                offer_index,
                false,
            )? {
                return Ok(Ter::TEF_BAD_LEDGER);
            }
        }
    }

    // Adjust owner count
    if let Some(account_sle) = view.peek(account_keylet(to_uint160(owner)))? {
        adjust_owner_count(view, &account_sle, -1)?;
    }

    // Erase the offer
    view.erase(sle)?;

    Ok(Ter::TES_SUCCESS)
}
