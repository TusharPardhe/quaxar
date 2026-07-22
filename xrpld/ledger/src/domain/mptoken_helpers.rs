//! the reference implementation parity — MPToken freeze, auth, transfer, escrow, and
//! creation helpers.

use crate::views::apply_view::ApplyView;
use crate::views::read_view::{ReadView, ViewError};
use crate::{adjust_owner_count, dir_insert, dir_remove};
use basics::base_uint::Uint160;
use protocol::{
    AccountID, Asset, Issue, LedgerEntryType, MPTID, MPTIssue, Rate, STAmount, STLedgerEntry, Ter,
    TxType, account_keylet, get_field_by_symbol, line, lsfGlobalFreeze, lsfHighFreeze,
    lsfLowFreeze, lsfMPTAuthorized, lsfMPTCanTrade, lsfMPTCanTransfer, lsfMPTLocked,
    lsfMPTRequireAuth, mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid, owner_dir_keylet,
    unchecked_keylet,
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
const MAX_ASSET_CHECK_DEPTH: u8 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MPTAuthType {
    Weak,
    Strong,
}

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
    is_frozen_mpt_with_depth(view, account, mpt_issue, 0)
}

fn is_frozen_mpt_with_depth(
    view: &dyn ReadView,
    account: &AccountID,
    mpt_issue: &MPTIssue,
    depth: u8,
) -> Result<bool, ViewError> {
    Ok(is_global_frozen_mpt(view, mpt_issue)?
        || is_individual_frozen_mpt(view, account, mpt_issue)?
        || is_vault_share_underlying_frozen(view, account, mpt_issue, depth)?)
}

fn is_frozen_issue_for_account(
    view: &dyn ReadView,
    account: &AccountID,
    issue: Issue,
) -> Result<bool, ViewError> {
    if issue.native() || *account == issue.issuer() {
        return Ok(false);
    }

    if let Some(issuer_root) = view.read(account_keylet(to_uint160(issue.issuer())))?
        && issuer_root.is_flag(lsfGlobalFreeze)
    {
        return Ok(true);
    }

    let Some(line) = view.read(line(*account, issue.issuer(), issue.currency))? else {
        return Ok(false);
    };
    Ok(line.is_flag(if issue.issuer() > *account {
        lsfHighFreeze
    } else {
        lsfLowFreeze
    }))
}

fn is_frozen_asset_for_accounts(
    view: &dyn ReadView,
    accounts: &[AccountID],
    asset: Asset,
    depth: u8,
) -> Result<bool, ViewError> {
    match asset {
        Asset::Issue(issue) => accounts.iter().try_fold(false, |frozen, account| {
            if frozen {
                Ok(true)
            } else {
                is_frozen_issue_for_account(view, account, issue)
            }
        }),
        Asset::MPTIssue(issue) => {
            if is_global_frozen_mpt(view, &issue)? {
                return Ok(true);
            }
            for account in accounts {
                if is_individual_frozen_mpt(view, account, &issue)? {
                    return Ok(true);
                }
            }
            for account in accounts {
                if is_vault_share_underlying_frozen(view, account, &issue, depth)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

fn is_vault_share_underlying_frozen(
    view: &dyn ReadView,
    account: &AccountID,
    mpt_issue: &MPTIssue,
    depth: u8,
) -> Result<bool, ViewError> {
    if !view
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"))
        && !view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_2_0"))
    {
        return Ok(false);
    }
    if depth >= MAX_ASSET_CHECK_DEPTH {
        return Ok(true);
    }

    let Some(share_issuance) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))?
    else {
        return Ok(false);
    };
    if !share_issuance.is_field_present(sf("sfReferenceHolding")) {
        return Ok(false);
    }

    let Some(holding) = view.read(unchecked_keylet(
        share_issuance.get_field_h256(sf("sfReferenceHolding")),
    ))?
    else {
        return Ok(false);
    };
    let Some(asset) = asset_of_holding(&share_issuance, &holding) else {
        return Ok(false);
    };
    let issuer = share_issuance.get_account_id(sf("sfIssuer"));
    is_frozen_asset_for_accounts(view, &[issuer, *account], asset, depth + 1)
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
    can_transfer_mpt_with_depth(view, mpt_issue, from, to, 0)
}

fn asset_of_holding(share_issuance: &STLedgerEntry, holding: &STLedgerEntry) -> Option<Asset> {
    match holding.get_type() {
        LedgerEntryType::MPToken => Some(Asset::from(MPTIssue::new(
            holding.get_field_h192(sf("sfMPTokenIssuanceID")),
        ))),
        LedgerEntryType::RippleState => {
            let vault_pseudo = share_issuance.get_account_id(sf("sfIssuer"));
            let low_limit = holding.get_field_amount(sf("sfLowLimit"));
            let high_limit = holding.get_field_amount(sf("sfHighLimit"));
            let low_issue = low_limit.issue();
            let high_issue = high_limit.issue();
            let issuer = if low_issue.issuer() != vault_pseudo {
                low_issue.issuer()
            } else {
                high_issue.issuer()
            };
            Some(Asset::from(Issue::new(low_issue.currency, issuer)))
        }
        _ => None,
    }
}

fn can_transfer_asset_with_depth(
    view: &dyn ReadView,
    asset: Asset,
    from: &AccountID,
    to: &AccountID,
    depth: u8,
) -> Result<Ter, ViewError> {
    match asset {
        Asset::MPTIssue(issue) => can_transfer_mpt_with_depth(view, &issue, from, to, depth),
        Asset::Issue(_) => Ok(Ter::TES_SUCCESS),
    }
}

fn can_transfer_mpt_with_depth(
    view: &dyn ReadView,
    mpt_issue: &MPTIssue,
    from: &AccountID,
    to: &AccountID,
    depth: u8,
) -> Result<Ter, ViewError> {
    let Some(sle_issuance) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    let issuer = sle_issuance.get_account_id(sf("sfIssuer"));
    if from == &issuer || to == &issuer {
        return Ok(Ter::TES_SUCCESS);
    }

    if !sle_issuance.is_flag(lsfMPTCanTransfer) {
        return Ok(Ter::TEC_NO_AUTH);
    }

    if view
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_2_0"))
        && sle_issuance.is_field_present(sf("sfReferenceHolding"))
    {
        if depth >= MAX_ASSET_CHECK_DEPTH {
            return Ok(Ter::TEC_INTERNAL);
        }
        let Some(holding) = view.read(unchecked_keylet(
            sle_issuance.get_field_h256(sf("sfReferenceHolding")),
        ))?
        else {
            return Ok(Ter::TEF_INTERNAL);
        };
        let Some(asset) = asset_of_holding(&sle_issuance, &holding) else {
            return Ok(Ter::TEF_INTERNAL);
        };
        return can_transfer_asset_with_depth(view, asset, from, to, depth + 1);
    }
    Ok(Ter::TES_SUCCESS)
}

/// Check if an asset can be traded on the DEX.
pub fn can_trade(view: &dyn ReadView, asset: &Asset) -> Result<Ter, ViewError> {
    can_trade_with_depth(view, asset, 0)
}

fn can_trade_with_depth(view: &dyn ReadView, asset: &Asset, depth: u8) -> Result<Ter, ViewError> {
    match asset {
        Asset::MPTIssue(mpt_issue) => {
            let Some(sle) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
                return Ok(Ter::TEC_OBJECT_NOT_FOUND);
            };
            if !sle.is_flag(lsfMPTCanTrade) {
                return Ok(Ter::TEC_NO_PERMISSION);
            }
            if view
                .rules()
                .enabled(&protocol::feature_id("fixCleanup3_2_0"))
                && sle.is_field_present(sf("sfReferenceHolding"))
            {
                if depth >= MAX_ASSET_CHECK_DEPTH {
                    return Ok(Ter::TEC_INTERNAL);
                }
                let Some(holding) = view.read(unchecked_keylet(
                    sle.get_field_h256(sf("sfReferenceHolding")),
                ))?
                else {
                    return Ok(Ter::TEF_INTERNAL);
                };
                let Some(asset) = asset_of_holding(&sle, &holding) else {
                    return Ok(Ter::TEF_INTERNAL);
                };
                return can_trade_with_depth(view, &asset, depth + 1);
            }
            Ok(Ter::TES_SUCCESS)
        }
        _ => Ok(Ter::TES_SUCCESS),
    }
}

pub fn can_mpt_trade_and_transfer(
    view: &dyn ReadView,
    asset: &Asset,
    from: &AccountID,
    to: &AccountID,
) -> Result<Ter, ViewError> {
    if !matches!(asset, Asset::MPTIssue(_)) {
        return Ok(Ter::TES_SUCCESS);
    }
    let trade = can_trade(view, asset)?;
    if trade != Ter::TES_SUCCESS {
        return Ok(trade);
    }
    can_transfer_asset_with_depth(view, *asset, from, to, 0)
}

/// Require authorization check for MPT.
pub fn require_auth_mpt(
    view: &dyn ReadView,
    mpt_issue: &MPTIssue,
    account: &AccountID,
) -> Result<Ter, ViewError> {
    require_auth_mpt_with_type(view, mpt_issue, account, MPTAuthType::Weak)
}

pub fn require_auth_mpt_with_type(
    view: &dyn ReadView,
    mpt_issue: &MPTIssue,
    account: &AccountID,
    auth_type: MPTAuthType,
) -> Result<Ter, ViewError> {
    let Some(sle_issuance) = view.read(mpt_issuance_keylet_from_mptid(mpt_issue.mpt_id()))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    let issuer = sle_issuance.get_account_id(sf("sfIssuer"));
    if &issuer == account {
        return Ok(Ter::TES_SUCCESS);
    }

    let account_key = account_keylet(to_uint160(*account));
    if (view
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"))
        || view.rules().enabled(&protocol::feature_id("MPTokensV2")))
        && let Some(account_root) = view.read(account_key)?
        && (account_root.is_field_present(sf("sfVaultID"))
            || account_root.is_field_present(sf("sfLoanBrokerID"))
            || account_root.is_field_present(sf("sfAMMID")))
    {
        return Ok(Ter::TES_SUCCESS);
    }

    let mptoken_key = mptoken_keylet_from_mptid(mpt_issue.mpt_id(), to_uint160(*account));
    let sle_token = view.read(mptoken_key)?;

    if auth_type == MPTAuthType::Strong && sle_token.is_none() {
        return Ok(Ter::TEC_NO_AUTH);
    }

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
    let Asset::MPTIssue(mpt_issue) = amount.asset() else {
        return Ok(Ter::TEC_INTERNAL);
    };
    if mpt_issue.issuer() == *sender {
        return Ok(Ter::TEC_INTERNAL);
    }
    let pay = amount.mpt().value();
    if pay <= 0 {
        return Ok(Ter::TEC_INTERNAL);
    }
    let pay = pay as u64;
    let mpt_id = mpt_issue.mpt_id();

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
    let Some(next_locked) = locked.checked_add(pay) else {
        return Ok(Ter::TEC_INTERNAL);
    };
    updated_token.set_field_u64(sf("sfLockedAmount"), next_locked);
    view.update(Arc::new(updated_token))?;

    // Increase issuance LockedAmount
    let mut updated_issuance = (*sle_issuance).clone();
    let issuance_locked = if updated_issuance.is_field_present(sf("sfLockedAmount")) {
        updated_issuance.get_field_u64(sf("sfLockedAmount"))
    } else {
        0
    };
    let Some(next_issuance_locked) = issuance_locked.checked_add(pay) else {
        return Ok(Ter::TEC_INTERNAL);
    };
    updated_issuance.set_field_u64(sf("sfLockedAmount"), next_issuance_locked);
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
    create_asset: bool,
    receiver_pre_fee_balance_drops: Option<i64>,
) -> Result<Ter, ViewError> {
    let Asset::MPTIssue(mpt_issue) = net_amount.asset() else {
        return Ok(Ter::TEC_INTERNAL);
    };
    if gross_amount.asset() != net_amount.asset() {
        return Ok(Ter::TEC_INTERNAL);
    }
    let gross = gross_amount.mpt().value();
    let net = net_amount.mpt().value();
    if gross <= 0 || net < 0 || net > gross {
        return Ok(Ter::TEC_INTERNAL);
    }
    let gross = gross as u64;
    let net = net as u64;
    let mpt_id = mpt_issue.mpt_id();
    let issuer = mpt_issue.issuer();

    if receiver != &issuer {
        let receiver_token_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*receiver));
        if view.peek(receiver_token_key)?.is_none() {
            if !create_asset {
                return Ok(Ter::TEC_NO_PERMISSION);
            }

            let Some(receiver_sle) = view.peek(account_keylet(to_uint160(*receiver)))? else {
                return Ok(Ter::TEC_NO_PERMISSION);
            };
            let owner_count = receiver_sle.get_field_u32(sf("sfOwnerCount"));
            let balance = receiver_pre_fee_balance_drops
                .unwrap_or_else(|| receiver_sle.get_field_amount(sf("sfBalance")).xrp().drops());
            if balance < view.fees().account_reserve(owner_count as usize + 1) as i64 {
                return Ok(Ter::TEC_INSUFFICIENT_RESERVE);
            }

            let result = check_create_mpt(view, &mpt_issue, receiver)?;
            if result != Ter::TES_SUCCESS && result != Ter::TEC_DUPLICATE {
                return Ok(result);
            }
        }
    }

    let Some(sle_issuance) = view.peek(mpt_issuance_keylet_from_mptid(mpt_id))? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };

    // Decrease issuance LockedAmount
    let issuance_locked = if sle_issuance.is_field_present(sf("sfLockedAmount")) {
        sle_issuance.get_field_u64(sf("sfLockedAmount"))
    } else {
        return Ok(Ter::TEC_INTERNAL);
    };

    let mut updated_issuance = (*sle_issuance).clone();
    let Some(new_locked) = issuance_locked.checked_sub(gross) else {
        return Ok(Ter::TEC_INTERNAL);
    };
    if new_locked == 0 {
        updated_issuance.make_field_absent(sf("sfLockedAmount"));
    } else {
        updated_issuance.set_field_u64(sf("sfLockedAmount"), new_locked);
    }
    if receiver == &issuer {
        let outstanding = updated_issuance.get_field_u64(sf("sfOutstandingAmount"));
        let Some(next_outstanding) = outstanding.checked_sub(net) else {
            return Ok(Ter::TEC_INTERNAL);
        };
        updated_issuance.set_field_u64(sf("sfOutstandingAmount"), next_outstanding);
    }
    view.update(Arc::new(updated_issuance))?;

    // Credit receiver or reduce outstanding
    if receiver != &issuer {
        let mptoken_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*receiver));
        let Some(sle_token) = view.peek(mptoken_key)? else {
            return Ok(Ter::TEC_OBJECT_NOT_FOUND);
        };
        let current = sle_token.get_field_u64(sf("sfMPTAmount"));
        let Some(next) = current.checked_add(net) else {
            return Ok(Ter::TEC_INTERNAL);
        };
        let mut updated = (*sle_token).clone();
        updated.set_field_u64(sf("sfMPTAmount"), next);
        view.update(Arc::new(updated))?;
    }

    if sender == &issuer {
        return Ok(Ter::TEC_INTERNAL);
    }

    // Decrease sender's LockedAmount
    let sender_key = mptoken_keylet_from_mptid(mpt_id, to_uint160(*sender));
    let Some(sle_sender) = view.peek(sender_key)? else {
        return Ok(Ter::TEC_OBJECT_NOT_FOUND);
    };
    let sender_locked = if sle_sender.is_field_present(sf("sfLockedAmount")) {
        sle_sender.get_field_u64(sf("sfLockedAmount"))
    } else {
        return Ok(Ter::TEC_INTERNAL);
    };
    let mut updated_sender = (*sle_sender).clone();
    let Some(new_sender_locked) = sender_locked.checked_sub(gross) else {
        return Ok(Ter::TEC_INTERNAL);
    };
    if new_sender_locked == 0 {
        updated_sender.make_field_absent(sf("sfLockedAmount"));
    } else {
        updated_sender.set_field_u64(sf("sfLockedAmount"), new_sender_locked);
    }
    view.update(Arc::new(updated_sender))?;

    let fee = gross - net;
    if fee != 0 {
        let Some(sle_issuance) = view.peek(mpt_issuance_keylet_from_mptid(mpt_id))? else {
            return Ok(Ter::TEC_OBJECT_NOT_FOUND);
        };
        let outstanding = sle_issuance.get_field_u64(sf("sfOutstandingAmount"));
        let Some(next_outstanding) = outstanding.checked_sub(fee) else {
            return Ok(Ter::TEC_INTERNAL);
        };
        let mut updated_issuance = (*sle_issuance).clone();
        updated_issuance.set_field_u64(sf("sfOutstandingAmount"), next_outstanding);
        view.update(Arc::new(updated_issuance))?;
    }

    Ok(Ter::TES_SUCCESS)
}

/// Check MPT allowed for a specific transaction type.
pub fn check_mpt_tx_allowed(
    view: &dyn ReadView,
    tx_type: TxType,
    asset: &Asset,
    account_id: &AccountID,
) -> Result<Ter, ViewError> {
    check_mpt_tx_allowed_with_depth(view, tx_type, asset, account_id, 0)
}

fn check_mpt_tx_allowed_with_depth(
    view: &dyn ReadView,
    tx_type: TxType,
    asset: &Asset,
    account_id: &AccountID,
    depth: u8,
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

        if view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_2_0"))
            && sle.is_field_present(sf("sfReferenceHolding"))
        {
            if depth >= MAX_ASSET_CHECK_DEPTH {
                return Ok(Ter::TEC_INTERNAL);
            }
            let Some(holding) = view.read(unchecked_keylet(
                sle.get_field_h256(sf("sfReferenceHolding")),
            ))?
            else {
                return Ok(Ter::TEF_INTERNAL);
            };
            let Some(underlying) = asset_of_holding(&sle, &holding) else {
                return Ok(Ter::TEF_INTERNAL);
            };
            return check_mpt_tx_allowed_with_depth(
                view,
                tx_type,
                &underlying,
                account_id,
                depth + 1,
            );
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
        if is_frozen_mpt(view, account, mpt_issue)? {
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

    let locked_amount = if mptoken.is_field_present(sf("sfLockedAmount")) {
        mptoken.get_field_u64(sf("sfLockedAmount"))
    } else {
        0
    };
    if mptoken.get_field_u64(sf("sfMPTAmount")) != 0
        || (view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_1_3"))
            && locked_amount != 0)
    {
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
