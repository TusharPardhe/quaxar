//! the reference implementation parity — offer deletion from the DEX.

use crate::views::apply_view::ApplyView;
use crate::views::read_view::ViewError;
use crate::{decrease_owner_count_for_object, dir_remove};
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
    let trace_offer_sequence = std::env::var("XRPL_TRACE_OFFER_SEQUENCE")
        .map(|value| value == "1")
        .unwrap_or(false);
    let owner_removed = dir_remove(
        view,
        &owner_dir_keylet(to_uint160(owner)),
        owner_node,
        offer_index,
        false,
    )?;
    if trace_offer_sequence {
        eprintln!(
            "TRACE offer_delete: offer={} owner={:?} owner_node={} owner_removed={}",
            offer_index, owner, owner_node, owner_removed
        );
    }
    if !owner_removed {
        return Ok(Ter::TEF_BAD_LEDGER);
    }

    // Remove from book directory
    let book_node = sle.get_field_u64(sf("sfBookNode"));
    let book_removed = dir_remove(
        view,
        &directory_node_keylet(book_directory),
        book_node,
        offer_index,
        false,
    )?;
    if trace_offer_sequence {
        eprintln!(
            "TRACE offer_delete: offer={} book_dir={} book_node={} book_removed={}",
            offer_index, book_directory, book_node, book_removed
        );
    }
    if !book_removed {
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

    // Match rippled's decreaseOwnerCountForObject even for defensive legacy
    // state. Offers are not currently sponsor-eligible, but using the object
    // aware helper keeps sponsorship counters canonical if such an entry is
    // ever encountered.
    if let Some(account_sle) = view.peek(account_keylet(to_uint160(owner)))? {
        decrease_owner_count_for_object(view, &account_sle, &sle, 1)?;
    }

    // Erase the offer
    view.erase(sle)?;

    Ok(Ter::TES_SUCCESS)
}
