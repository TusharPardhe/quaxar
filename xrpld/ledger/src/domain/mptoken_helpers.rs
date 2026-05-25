//! the reference implementation parity — MPToken freeze, auth, transfer, escrow, and
//! creation helpers.

use crate::views::apply_view::ApplyView;
use crate::views::read_view::{ReadView, ViewError};
use crate::{adjust_owner_count, dir_insert, dir_remove};
use basics::base_uint::Uint160;
use protocol::{
    AccountID, Asset, MPTID, MPTIssue, Rate, STAmount, STLedgerEntry, Ter, TxType, account_keylet,
    get_field_by_symbol, lsfMPTAuthorized, lsfMPTCanTrade, lsfMPTCanTransfer, lsfMPTLocked,
    lsfMPTRequireAuth, mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid, owner_dir_keylet,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match")
}

/// Maximum MPT amount (2^63 - 1).
pub const MAX_MPT_AMOUNT: i64 = i64::MAX;

/// Maximum transfer fee in tenths of a basis point.
pub const MAX_TRANSFER_FEE: u16 = 50_000;

/// Parity rate (no fee).
pub const PARITY_RATE: Rate = Rate::new(1_000_000_000);

/// Check if an MPT issuance is globally frozen (locked).
pub fn is_global_frozen_mpt(view: &dyn ReadView, mpt_issue: &MPTIssue) -> Result<bool, ViewError> {
    let Some(sle) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
        return Ok(false);
    };
    Ok(sle.is_flag(lsfMPTLocked))
}

/// Check if an individual MPToken is frozen (locked).
pub fn is_individual_frozen_mpt(
    view: &dyn ReadView,
    account: &AccountID,
    mpt_issue: &MPTIssue,
) -> Result<bool, ViewError> {
    let Some(sle) = view.read(mptoken_keylet_from_mptid(
        mpt_issue.mpt_id(),
        to_uint160(*account),
    ))?
    else {
        return Ok(false);
    };
    Ok(sle.is_flag(lsfMPTLocked))
}

/// Check if an account's MPToken is frozen (global or individual).
pub fn is_frozen_mpt(
    view: &dyn ReadView,
    account: &AccountID,
    mpt_issue: &MPTIssue,
) -> Result<bool, ViewError> {
    Ok(is_global_frozen_mpt(view, mpt_issue)?
        || is_individual_frozen_mpt(view, account, mpt_issue)?)
}

/// Get the transfer rate for an MPT issuance.
pub fn transfer_rate_mpt(view: &dyn ReadView, issuance_id: MPTID) -> Result<Rate, ViewError> {
    let Some(sle) = view.read(mpt_issuance_keylet_from_mptid(issuance_id))? else {
        return Ok(PARITY_RATE);
    };
    if sle.is_field_present(sf("sfTransferFee")) {
        let fee = sle.get_field_u16(sf("sfTransferFee"));
        return Ok(Rate::new(1_000_000_000u32 + (10_000u32 * fee as u32)));
    }
    Ok(PARITY_RATE)
}

/// Check if a new holding can be added for this MPT issuance.
pub fn can_add_holding_mpt(view: &dyn ReadView, mpt_issue: &MPTIssue) -> Result<Ter, ViewError> {
    let Some(issuance) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };
    if !issuance.is_flag(lsfMPTCanTransfer) {
        return Ok(Ter::TEC_NO_AUTH);
    }
    Ok(Ter::TES_SUCCESS)
}

/// Check if transfer is allowed between two accounts for an MPT.
pub fn can_transfer_mpt(
    view: &dyn ReadView,
    mpt_issue: &MPTIssue,
    from: &AccountID,
    to: &AccountID,
) -> Result<Ter, ViewError> {
    let Some(sle_issuance) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    if !sle_issuance.is_flag(lsfMPTCanTransfer) {
        let issuer = sle_issuance.get_account_id(sf("sfIssuer"));
        if from != &issuer && to != &issuer {
            return Ok(Ter::TEC_NO_AUTH);
        }
    }
    Ok(Ter::TES_SUCCESS)
}

/// Check if an asset can be traded on the DEX.
pub fn can_trade(view: &dyn ReadView, asset: &Asset) -> Result<Ter, ViewError> {
    match asset {
        Asset::MPTIssue(mpt_issue) => {
            let Some(sle) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            };
            if !sle.is_flag(lsfMPTCanTrade) {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            Ok(Ter::TES_SUCCESS)
        }
        _ => Ok(Ter::TES_SUCCESS),
    }
}

/// Require authorization check for MPT.
pub fn require_auth_mpt(
    view: &dyn ReadView,
    mpt_issue: &MPTIssue,
    account: &AccountID,
) -> Result<Ter, ViewError> {
    let Some(sle_issuance) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    let issuer = sle_issuance.get_account_id(sf("sfIssuer"));
    if &issuer == account {
        return Ok(Ter::TES_SUCCESS);
    }

    let mptoken_key = mptoken_keylet_from_mptid(mpt_issue.mpt_id(), to_uint160(*account));
    let sle_token = view.read(mptoken_key)?;

    if sle_issuance.is_flag(lsfMPTRequireAuth) {
        match sle_token {
            Some(ref token) if token.is_flag(lsfMPTAuthorized) => {}
            _ => return Ok(Ter::TEC_NO_AUTH),
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Get the maximum MPT amount from an issuance SLE.
pub fn max_mpt_amount(sle_issuance: &STLedgerEntry) -> i64 {
    if sle_issuance.is_field_present(sf("sfMaximumAmount")) {
        sle_issuance.get_field_u64(sf("sfMaximumAmount")) as i64
    } else {
        MAX_MPT_AMOUNT
    }
}

/// Get the available (remaining mintable) MPT amount.
pub fn available_mpt_amount(sle_issuance: &STLedgerEntry) -> i64 {
    let max = max_mpt_amount(sle_issuance);
    let outstanding = sle_issuance.get_field_u64(sf("sfOutstandingAmount")) as i64;
    max - outstanding
}

/// Lock MPT tokens for escrow.
pub fn lock_escrow_mpt(
    view: &mut dyn ApplyView,
    sender: &AccountID,
    amount: &STAmount,
) -> Result<Ter, ViewError> {
    let mpt_amount = amount.mpt();
    let pay = mpt_amount.value() as u64;

    // Get issuance ID from the amount's asset
    let mpt_id = match amount.asset() {
        Asset::MPTIssue(i) => i.mpt_id(),
        _ => panic!("expected MPT"),
    };

    let Some(sle_issuance) = view.peek(mpt_issuance_keylet_from_mptid(mpt_id))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    // Decrease holder's MPTAmount, increase LockedAmount
    let mptoken_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*sender));
    let Some(sle_token) = view.peek(mptoken_key)? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    let current_amount = sle_token.get_field_u64(sf("sfMPTAmount"));
    if pay > current_amount {
        return Ok(Ter::TEC_INTERNAL);
    }

    let mut updated_token = (*sle_token).clone();
    updated_token.set_field_u64(sf("sfMPTAmount"), current_amount - pay);

    let locked = if updated_token.is_field_present(sf("sfLockedAmount")) {
        updated_token.get_field_u64(sf("sfLockedAmount"))
    } else {
        0
    };
    updated_token.set_field_u64(sf("sfLockedAmount"), locked + pay);
    view.update(Arc::new(updated_token))?;

    // Increase issuance LockedAmount
    let mut updated_issuance = (*sle_issuance).clone();
    let issuance_locked = if updated_issuance.is_field_present(sf("sfLockedAmount")) {
        updated_issuance.get_field_u64(sf("sfLockedAmount"))
    } else {
        0
    };
    updated_issuance.set_field_u64(sf("sfLockedAmount"), issuance_locked + pay);
    view.update(Arc::new(updated_issuance))?;

    Ok(Ter::TES_SUCCESS)
}

/// Unlock MPT tokens from escrow.
pub fn unlock_escrow_mpt(
    view: &mut dyn ApplyView,
    sender: &AccountID,
    receiver: &AccountID,
    net_amount: &STAmount,
    gross_amount: &STAmount,
) -> Result<Ter, ViewError> {
    let mpt_id = match net_amount.asset() {
        Asset::MPTIssue(i) => i.mpt_id(),
        _ => panic!("expected MPT"),
    };
    let issuer = net_amount.asset().issuer();

    let Some(sle_issuance) = view.peek(mpt_issuance_keylet_from_mptid(mpt_id))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    // Decrease issuance LockedAmount
    let issuance_locked = sle_issuance.get_field_u64(sf("sfLockedAmount"));
    let redeem = gross_amount.mpt().value() as u64;
    if redeem > issuance_locked {
        return Ok(Ter::TEC_INTERNAL);
    }

    let mut updated_issuance = (*sle_issuance).clone();
    let new_locked = issuance_locked - redeem;
    if new_locked == 0 {
        updated_issuance.make_field_absent(sf("sfLockedAmount"));
    } else {
        updated_issuance.set_field_u64(sf("sfLockedAmount"), new_locked);
    }
    view.update(Arc::new(updated_issuance))?;

    // Credit receiver or reduce outstanding
    if receiver != &issuer {
        let mptoken_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*receiver));
        let Some(sle_token) = view.peek(mptoken_key)? else {
            return Ok(Ter::TEC_OBJECT_NOT_FOUND);
        };
        let current = sle_token.get_field_u64(sf("sfMPTAmount"));
        let delta = net_amount.mpt().value() as u64;
        let mut updated = (*sle_token).clone();
        updated.set_field_u64(sf("sfMPTAmount"), current + delta);
        view.update(Arc::new(updated))?;
    }

    // Decrease sender's LockedAmount
    let sender_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*sender));
    let Some(sle_sender) = view.peek(sender_key)? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };
    let sender_locked = sle_sender.get_field_u64(sf("sfLockedAmount"));
    let delta = gross_amount.mpt().value() as u64;
    if delta > sender_locked {
        return Ok(Ter::TEC_INTERNAL);
    }
    let mut updated_sender = (*sle_sender).clone();
    let new_sender_locked = sender_locked - delta;
    if new_sender_locked == 0 {
        updated_sender.make_field_absent(sf("sfLockedAmount"));
    } else {
        updated_sender.set_field_u64(sf("sfLockedAmount"), new_sender_locked);
    }
    view.update(Arc::new(updated_sender))?;

    Ok(Ter::TES_SUCCESS)
}

/// Check MPT allowed for a specific transaction type.
pub fn check_mpt_tx_allowed(
    view: &dyn ReadView,
    _tx_type: TxType,
    asset: &Asset,
    account_id: &AccountID,
) -> Result<Ter, ViewError> {
    let Asset::MPTIssue(mpt_issue) = asset else {
        return Ok(Ter::TES_SUCCESS);
    };

    let issuance_key = mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id());
    let Some(sle) = view.read(issuance_key)? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    let flags = sle.get_flags();

    if (flags & lsfMPTLocked) != 0 {
        return Ok(Ter::TEC_LOCKED);
    }
    if (flags & lsfMPTCanTrade) == 0 {
        return Ok(Ter::TEC_NO_PERMISSION);
    }

    let issuer = sle.get_account_id(sf("sfIssuer"));
    if account_id != &issuer {
        if (flags & lsfMPTCanTransfer) == 0 {
            return Ok(Ter::TEC_NO_PERMISSION);
        }

        let mpt_sle = view.read(mptoken_keylet_from_mptid(
            mpt_issue.mpt_id(),
            to_uint160(*account_id),
        ))?;
        if let Some(mpt) = mpt_sle
            && mpt.is_flag(lsfMPTLocked)
        {
            return Ok(Ter::TEC_LOCKED);
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Check if any of the given accounts have a frozen MPToken.
///
pub fn is_any_frozen_mpt(
    view: &dyn ReadView,
    accounts: &[AccountID],
    mpt_issue: &MPTIssue,
) -> Result<bool, ViewError> {
    if is_global_frozen_mpt(view, mpt_issue)? {
        return Ok(true);
    }
    for account in accounts {
        if is_individual_frozen_mpt(view, account, mpt_issue)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Add an empty MPToken holding for an account.
///
pub fn add_empty_holding_mpt(
    view: &mut dyn ApplyView,
    account_id: &AccountID,
    mpt_issue: &MPTIssue,
) -> Result<Ter, ViewError> {
    let mpt_id = mpt_issue.mpt_id();
    let Some(mpt) = view.peek(mpt_issuance_keylet_from_mptid(mpt_id))? else {
        return Ok(Ter::TEF_INTERNAL);
    };
    if mpt.is_flag(lsfMPTLocked) {
        return Ok(Ter::TEF_INTERNAL);
    }
    if view
        .peek(mptoken_keylet_from_mptid(mpt_id, to_uint160(*account_id)))?
        .is_some()
    {
        return Ok(Ter::TEC_DUPLICATE);
    }
    if *account_id == mpt_issue.issuer() {
        return Ok(Ter::TES_SUCCESS);
    }

    // Create the MPToken
    authorize_mp_token(view, mpt_id, account_id)
}

/// Remove an empty MPToken holding.
///
pub fn remove_empty_holding_mpt(
    view: &mut dyn ApplyView,
    account_id: &AccountID,
    mpt_issue: &MPTIssue,
) -> Result<Ter, ViewError> {
    let mpt_id = mpt_issue.mpt_id();
    let Some(mptoken) = view.peek(mptoken_keylet_from_mptid(mpt_id, to_uint160(*account_id)))?
    else {
        if *account_id == mpt_issue.issuer() {
            return Ok(Ter::TES_SUCCESS);
        }
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    if mptoken.get_field_u64(sf("sfMPTAmount")) != 0 {
        return Ok(Ter::TEC_HAS_OBLIGATIONS);
    }

    // Delete the MPToken
    let owner_node = mptoken.get_field_u64(sf("sfOwnerNode"));
    if !dir_remove(
        view,
        &owner_dir_keylet(to_uint160(*account_id)),
        owner_node,
        *mptoken.key(),
        false,
    )? {
        return Ok(Ter::TEF_INTERNAL);
    }

    if let Some(acct) = view.peek(account_keylet(to_uint160(*account_id)))? {
        adjust_owner_count(view, &acct, -1)?;
    }

    view.erase(mptoken)?;
    Ok(Ter::TES_SUCCESS)
}

/// Create a new MPToken for a holder.
///
pub fn create_mp_token(
    view: &mut dyn ApplyView,
    mpt_issuance_id: MPTID,
    account: &AccountID,
    flags: u32,
) -> Result<Ter, ViewError> {
    let mptoken_key = mptoken_keylet_from_mptid(mpt_issuance_id, to_uint160(*account));

    let owner_node = dir_insert(
        view,
        &owner_dir_keylet(to_uint160(*account)),
        mptoken_key.key,
        &|_| {},
    )?;
    let Some(node) = owner_node else {
        return Ok(Ter::TEC_DIR_FULL);
    };

    let mut mptoken = STLedgerEntry::new(mptoken_key);
    mptoken.set_account_id(sf("sfAccount"), *account);
    mptoken.set_field_u32(sf("sfFlags"), flags);
    mptoken.set_field_u64(sf("sfOwnerNode"), node);
    view.insert(Arc::new(mptoken))?;

    Ok(Ter::TES_SUCCESS)
}

/// Check if MPT overflow would occur.
///
pub fn is_mpt_overflow(send_amount: i64, outstanding_amount: u64, maximum_amount: i64) -> bool {
    send_amount > maximum_amount
        || outstanding_amount > (maximum_amount as u64).saturating_sub(send_amount as u64)
}

/// Authorize an MPToken (create it for a holder).
fn authorize_mp_token(
    view: &mut dyn ApplyView,
    mpt_id: MPTID,
    account: &AccountID,
) -> Result<Ter, ViewError> {
    let mptoken_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*account));

    let owner_node = dir_insert(
        view,
        &owner_dir_keylet(to_uint160(*account)),
        mptoken_key.key,
        &|_| {},
    )?;
    let Some(node) = owner_node else {
        return Ok(Ter::TEC_DIR_FULL);
    };

    let mut mptoken = STLedgerEntry::new(mptoken_key);
    mptoken.set_account_id(sf("sfAccount"), *account);
    mptoken.set_field_u32(sf("sfFlags"), 0);
    mptoken.set_field_u64(sf("sfOwnerNode"), node);
    view.insert(Arc::new(mptoken))?;

    if let Some(acct) = view.peek(account_keylet(to_uint160(*account)))? {
        adjust_owner_count(view, &acct, 1)?;
    }

    Ok(Ter::TES_SUCCESS)
}

/// Check and create MPToken if it doesn't exist for a holder.
///
pub fn check_create_mpt(
    view: &mut dyn ApplyView,
    mpt_issue: &MPTIssue,
    holder: &AccountID,
) -> Result<Ter, ViewError> {
    if mpt_issue.issuer() == *holder {
        return Ok(Ter::TES_SUCCESS);
    }

    let mpt_id = mpt_issue.mpt_id();
    let mptoken_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*holder));
    if view.peek(mptoken_key)?.is_some() {
        return Ok(Ter::TES_SUCCESS);
    }

    // Create the MPToken
    let result = create_mp_token(view, mpt_id, holder, 0)?;
    if result != Ter::TES_SUCCESS {
        return Ok(result);
    }

    if let Some(acct) = view.peek(account_keylet(to_uint160(*holder)))? {
        adjust_owner_count(view, &acct, 1)?;
    }

    Ok(Ter::TES_SUCCESS)
}

/// Enforce MPToken authorization — checks domain credentials or explicit auth.
///
pub fn enforce_mp_token_authorization(
    view: &mut dyn ApplyView,
    mpt_issuance_id: MPTID,
    account: &AccountID,
) -> Result<Ter, ViewError> {
    let Some(sle_issuance) = view.read(mpt_issuance_keylet_from_mptid(mpt_issuance_id))? else {
        return Ok(Ter::TEF_INTERNAL);
    };

    if *account == sle_issuance.get_account_id(sf("sfIssuer")) {
        return Ok(Ter::TEF_INTERNAL);
    }

    let mptoken_key = mptoken_keylet_from_mptid(mpt_issuance_id, to_uint160(*account));
    let sle_token = view.read(mptoken_key)?;

    // Check domain-based authorization
    let maybe_domain_id = if sle_issuance.is_field_present(sf("sfDomainID")) {
        Some(sle_issuance.get_field_h256(sf("sfDomainID")))
    } else {
        None
    };

    let authorized_by_domain = if let Some(domain_id) = maybe_domain_id {
        let result = super::credential_helpers::verify_valid_domain(view, account, domain_id)?;
        result == Ter::TES_SUCCESS
    } else {
        false
    };

    if !authorized_by_domain && sle_token.is_none() {
        return Ok(Ter::TEC_NO_AUTH);
    }

    if !authorized_by_domain && maybe_domain_id.is_some() {
        return Ok(Ter::TEC_NO_AUTH);
    }

    if !authorized_by_domain {
        // Classic MPToken authorization check
        if let Some(ref token) = sle_token
            && token.is_flag(lsfMPTAuthorized)
        {
            return Ok(Ter::TES_SUCCESS);
        }
        return Ok(Ter::TEC_NO_AUTH);
    }

    // Authorized by domain — create MPToken if needed
    if authorized_by_domain && sle_token.is_none() {
        let result = authorize_mp_token(view, mpt_issuance_id, account)?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }

    Ok(Ter::TES_SUCCESS)
}

/// Get the issuer's available funds for self-issuance.
///
pub fn issuer_funds_to_self_issue(
    view: &dyn ReadView,
    issue: &MPTIssue,
) -> Result<STAmount, ViewError> {
    let Some(sle) = view.read(mpt_issuance_keylet_from_mptid(issue.mpt_id()))? else {
        return Ok(STAmount::from_mpt_amount(
            protocol::sf_generic(),
            protocol::MPTAmount::new(),
            *issue,
        ));
    };
    let available = available_mpt_amount(&sle);
    Ok(STAmount::from_mpt_amount(
        protocol::sf_generic(),
        protocol::MPTAmount::from_value(available),
        *issue,
    ))
}
