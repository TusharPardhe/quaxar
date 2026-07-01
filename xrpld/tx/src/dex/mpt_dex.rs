//! MPT DEX validation and crossing helpers.
//!
//! Ported from `src/libxrpl/ledger/helpers/MPTokenHelpers.cpp` and
//! `src/libxrpl/tx/paths/BookStep.cpp` (MPT-aware offer crossing logic).

use basics::base_uint::Uint160;
use ledger::views::apply_view::ApplyView;
use ledger::views::read_view::ReadView;
use protocol::{
    Asset, Keylet, LedgerEntryType, MPTIssue, STLedgerEntry, Ter,
    get_field_by_symbol, is_tes_success,
    lsfMPTAuthorized, lsfMPTCanTrade, lsfMPTCanTransfer, lsfMPTLocked, lsfMPTRequireAuth,
    mpt_issuance_keylet_from_mptid, mptoken_keylet,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account_to_uint160(account: &protocol::AccountID) -> Uint160 {
    Uint160::from_void(account.data())
}

/// Returns `true` if the MPT issuance has the `lsfMPTCanTrade` flag set.
pub fn can_mpt_trade<V: ReadView>(view: &V, issue: &MPTIssue) -> Result<bool, Ter> {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let Some(sle) = view.read(issuance_keylet).map_err(|_| Ter::TEF_INTERNAL)? else {
        return Err(Ter::TEC_OBJECT_NOT_FOUND);
    };
    Ok(sle.is_flag(lsfMPTCanTrade))
}

/// Returns `true` if the MPT issuance has the `lsfMPTCanTransfer` flag set.
pub fn can_mpt_transfer<V: ReadView>(
    view: &V,
    issue: &MPTIssue,
    from: &protocol::AccountID,
    to: &protocol::AccountID,
) -> Result<bool, Ter> {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let Some(sle) = view.read(issuance_keylet).map_err(|_| Ter::TEF_INTERNAL)? else {
        return Err(Ter::TEC_OBJECT_NOT_FOUND);
    };
    let issuer = sle.get_account_id(sf("sfIssuer"));
    if *from == issuer || *to == issuer {
        return Ok(true);
    }
    Ok(sle.is_flag(lsfMPTCanTransfer))
}

/// Combined check: asset can be traded and transferred between `from` and `to`.
/// For non-MPT assets this is always `tesSUCCESS`.
pub fn can_mpt_trade_and_transfer<V: ReadView>(
    view: &V,
    asset: &Asset,
    from: &protocol::AccountID,
    to: &protocol::AccountID,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };
    match can_mpt_trade(view, issue) {
        Ok(true) => {}
        Ok(false) => return Ter::TEC_NO_PERMISSION,
        Err(ter) => return ter,
    }
    match can_mpt_transfer(view, issue, from, to) {
        Ok(true) => Ter::TES_SUCCESS,
        Ok(false) => Ter::TEC_NO_AUTH,
        Err(ter) => ter,
    }
}

/// Check that `account` is authorized to hold the given MPT issuance.
/// Matches C++ `requireAuth(view, mptIssue, account, AuthType::WeakAuth)`.
///
/// WeakAuth means we do NOT require the MPToken to already exist (it may be
/// created on demand), but if the issuance has `lsfMPTRequireAuth` then an
/// existing MPToken must carry `lsfMPTAuthorized`.
pub fn require_mpt_auth<V: ReadView>(
    view: &V,
    issue: &MPTIssue,
    account: &protocol::AccountID,
) -> Ter {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let Ok(Some(sle_issuance)) = view.read(issuance_keylet) else {
        return Ter::TEC_OBJECT_NOT_FOUND;
    };
    let issuer = sle_issuance.get_account_id(sf("sfIssuer"));
    if issuer == *account {
        return Ter::TES_SUCCESS;
    }
    if !sle_issuance.is_flag(lsfMPTRequireAuth) {
        return Ter::TES_SUCCESS;
    }
    let token_keylet = mptoken_keylet(issuance_keylet.key, account_to_uint160(account));
    let Ok(Some(sle_token)) = view.read(token_keylet) else {
        // WeakAuth: token doesn't exist yet — that's fine for offer creation
        // (will be created during crossing). But if requireAuth is set and no
        // token exists, we can't verify authorization.
        return Ter::TEC_NO_AUTH;
    };
    if !sle_token.is_flag(lsfMPTAuthorized) {
        return Ter::TEC_NO_AUTH;
    }
    Ter::TES_SUCCESS
}

/// Check if the MPT issuance is globally frozen (locked).
pub fn is_mpt_frozen<V: ReadView>(view: &V, issue: &MPTIssue) -> bool {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let Ok(Some(sle)) = view.read(issuance_keylet) else {
        return false;
    };
    sle.is_flag(lsfMPTLocked)
}

/// Check if a specific account's MPToken is individually frozen.
pub fn is_mpt_individual_frozen<V: ReadView>(
    view: &V,
    issue: &MPTIssue,
    account: &protocol::AccountID,
) -> bool {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let token_keylet = mptoken_keylet(issuance_keylet.key, account_to_uint160(account));
    let Ok(Some(sle)) = view.read(token_keylet) else {
        return false;
    };
    sle.is_flag(lsfMPTLocked)
}

/// Create an MPToken for `holder` if one does not already exist.
/// Matches C++ `checkCreateMPT(view, mptIssue, holder, journal)`.
pub fn check_create_mpt<V: ApplyView>(
    view: &mut V,
    issue: &MPTIssue,
    holder: &protocol::AccountID,
) -> Ter {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let issuer = issue.issuer();
    if issuer == *holder {
        return Ter::TES_SUCCESS;
    }

    let token_keylet = mptoken_keylet(issuance_keylet.key, account_to_uint160(holder));
    if view.exists(token_keylet).unwrap_or(false) {
        return Ter::TES_SUCCESS;
    }

    // Create a new MPToken for this holder
    let mut mptoken = STLedgerEntry::new(Keylet {
        entry_type: LedgerEntryType::MPToken,
        key: token_keylet.key,
    });
    mptoken.set_account_id(sf("sfAccount"), *holder);
    mptoken.set_field_h192(sf("sfMPTokenIssuanceID"), issue.mpt_id());
    mptoken.set_field_u32(sf("sfFlags"), 0);

    // Link into owner directory and adjust owner count
    let owner_dir = protocol::owner_dir_keylet(account_to_uint160(holder));
    match ledger::apply_directory::dir_append(view, &owner_dir, token_keylet.key, &|_| {}) {
        Ok(Some(owner_node)) => {
            mptoken.set_field_u64(sf("sfOwnerNode"), owner_node);
        }
        _ => return Ter::TEC_DIR_FULL,
    }

    if view.insert(Arc::new(mptoken)).is_err() {
        return Ter::TEF_INTERNAL;
    }

    // Adjust owner count
    let acct_keylet = protocol::account_keylet(account_to_uint160(holder));
    if let Ok(Some(acct_sle)) = view.peek(acct_keylet) {
        let _ = ledger::adjust_owner_count(view, &acct_sle, 1);
    }

    Ter::TES_SUCCESS
}

/// Validate MPT DEX preconditions for offer creation.
/// Called from OfferCreate preclaim to check:
/// 1. The issuance can be traded
/// 2. The offer creator is authorized to hold the asset
/// 3. The issuance is not globally frozen
pub fn check_mpt_dex_preclaim<V: ReadView>(
    view: &V,
    account: &protocol::AccountID,
    asset: &Asset,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };

    // Check global freeze
    if is_mpt_frozen(view, issue) {
        return Ter::TEC_FROZEN;
    }

    // Check canTrade flag on issuance
    match can_mpt_trade(view, issue) {
        Ok(true) => {}
        Ok(false) => return Ter::TEC_NO_PERMISSION,
        Err(ter) => return ter,
    }

    // Check authorization (WeakAuth — token need not exist yet)
    let auth = require_mpt_auth(view, issue, account);
    if !is_tes_success(auth) {
        return auth;
    }

    Ter::TES_SUCCESS
}

/// During crossing, verify the offer owner can receive the incoming MPT asset
/// and create an MPToken if needed.
pub fn check_mpt_dex_crossing<V: ApplyView>(
    view: &mut V,
    issue: &MPTIssue,
    owner: &protocol::AccountID,
) -> Ter {
    let ter = check_create_mpt(view, issue, owner);
    if !is_tes_success(ter) {
        return ter;
    }
    require_mpt_auth(view, issue, owner)
}
