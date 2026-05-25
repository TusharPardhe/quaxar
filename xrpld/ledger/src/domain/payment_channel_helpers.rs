//! the reference implementation parity — payment channel close logic.

use crate::views::apply_view::ApplyView;
use crate::views::read_view::ViewError;
use crate::{adjust_owner_count, dir_remove};
use basics::base_uint::{Uint160, Uint256};
use protocol::{STLedgerEntry, Ter, account_keylet, get_field_by_symbol, owner_dir_keylet};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_uint160(account: protocol::AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

/// Closes a payment channel: removes from owner directories, returns remaining
/// funds to the source account, decrements owner count, and erases the channel.
///
pub fn close_channel(
    view: &mut dyn ApplyView,
    slep: Arc<STLedgerEntry>,
    key: Uint256,
) -> Result<Ter, ViewError> {
    let src = slep.get_account_id(sf("sfAccount"));

    // Remove PayChan from source owner directory
    let page = slep.get_field_u64(sf("sfOwnerNode"));
    if !dir_remove(view, &owner_dir_keylet(to_uint160(src)), page, key, true)? {
        return Ok(Ter::TEF_BAD_LEDGER);
    }

    // Remove PayChan from destination's owner directory, if present
    if slep.is_field_present(sf("sfDestinationNode")) {
        let dst_page = slep.get_field_u64(sf("sfDestinationNode"));
        let dst = slep.get_account_id(sf("sfDestination"));
        if !dir_remove(
            view,
            &owner_dir_keylet(to_uint160(dst)),
            dst_page,
            key,
            true,
        )? {
            return Ok(Ter::TEF_BAD_LEDGER);
        }
    }

    // Transfer remaining amount back to owner, decrement owner count
    let Some(account_sle) = view.peek(account_keylet(to_uint160(src)))? else {
        return Ok(Ter::TEF_INTERNAL);
    };

    // balance += (channel_amount - channel_balance)
    let channel_amount = slep.get_field_amount(sf("sfAmount"));
    let channel_balance = slep.get_field_amount(sf("sfBalance"));
    let account_balance = account_sle.get_field_amount(sf("sfBalance"));
    let refund = channel_amount.xrp() - channel_balance.xrp();
    let new_balance = account_balance.xrp() + refund;

    let mut updated_sle = (*account_sle).clone();
    updated_sle.set_field_amount(
        sf("sfBalance"),
        protocol::STAmount::from_xrp_amount(new_balance),
    );
    let updated_arc = Arc::new(updated_sle);
    view.update(updated_arc.clone())?;

    adjust_owner_count(view, &updated_arc, -1)?;

    // Remove PayChan from ledger
    view.erase(slep)?;

    Ok(Ter::TES_SUCCESS)
}
