//! MPT DEX validation and crossing helpers.
//!
//! Ported from `src/libxrpl/ledger/helpers/MPTokenHelpers.cpp` and
//! `src/libxrpl/tx/paths/BookStep.cpp` (MPT-aware offer crossing logic).

use basics::base_uint::Uint160;
use ledger::views::apply_view::ApplyView;
use ledger::views::read_view::ReadView;
use protocol::{
    Asset, Keylet, LedgerEntryType, MPTIssue, STLedgerEntry, Ter, get_field_by_symbol,
    is_tes_success, lsfMPTAuthorized, lsfMPTCanTrade, lsfMPTCanTransfer, lsfMPTLocked,
    lsfMPTRequireAuth, mpt_issuance_keylet_from_mptid, mptoken_keylet,
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
    let Some(sle) = view
        .read(issuance_keylet)
        .map_err(|_| Ter::TEF_BAD_LEDGER)?
    else {
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
    let Some(sle) = view
        .read(issuance_keylet)
        .map_err(|_| Ter::TEF_BAD_LEDGER)?
    else {
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
    let sle_issuance = match view.read(issuance_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_OBJECT_NOT_FOUND,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    let issuer = sle_issuance.get_account_id(sf("sfIssuer"));
    if issuer == *account {
        return Ter::TES_SUCCESS;
    }
    if !sle_issuance.is_flag(lsfMPTRequireAuth) {
        return Ter::TES_SUCCESS;
    }
    let token_keylet = mptoken_keylet(issuance_keylet.key, account_to_uint160(account));
    let sle_token = match view.read(token_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEC_NO_AUTH,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    if !sle_token.is_flag(lsfMPTAuthorized) {
        return Ter::TEC_NO_AUTH;
    }
    Ter::TES_SUCCESS
}

/// Check if the MPT issuance is globally frozen (locked).
pub fn is_mpt_frozen<V: ReadView>(view: &V, issue: &MPTIssue) -> Result<bool, Ter> {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let sle = match view.read(issuance_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ok(false),
        Err(_) => return Err(Ter::TEF_BAD_LEDGER),
    };
    Ok(sle.is_flag(lsfMPTLocked))
}

/// Check if a specific account's MPToken is individually frozen.
pub fn is_mpt_individual_frozen<V: ReadView>(
    view: &V,
    issue: &MPTIssue,
    account: &protocol::AccountID,
) -> Result<bool, Ter> {
    let issuance_keylet = mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let token_keylet = mptoken_keylet(issuance_keylet.key, account_to_uint160(account));
    let sle = match view.read(token_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ok(false),
        Err(_) => return Err(Ter::TEF_BAD_LEDGER),
    };
    Ok(sle.is_flag(lsfMPTLocked))
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
    match view.exists(token_keylet) {
        Ok(true) => return Ter::TES_SUCCESS,
        Ok(false) => {}
        Err(_) => return Ter::TEF_BAD_LEDGER,
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
    match ledger::apply_directory::dir_insert(
        view,
        &owner_dir,
        token_keylet.key,
        &ledger::describe_owner_dir(*holder),
    ) {
        Ok(Some(owner_node)) => {
            mptoken.set_field_u64(sf("sfOwnerNode"), owner_node);
        }
        Ok(None) => return Ter::TEC_DIR_FULL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    }

    if view.insert(Arc::new(mptoken)).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }

    // Adjust owner count
    let acct_keylet = protocol::account_keylet(account_to_uint160(holder));
    let acct_sle = match view.peek(acct_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEF_INTERNAL,
        Err(_) => return Ter::TEF_BAD_LEDGER,
    };
    if ledger::adjust_owner_count(view, &acct_sle, 1).is_err() {
        return Ter::TEF_BAD_LEDGER;
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
    match is_mpt_frozen(view, issue) {
        Ok(true) => return Ter::TEC_FROZEN,
        Ok(false) => {}
        Err(ter) => return ter,
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

#[cfg(test)]
mod tests {
    use super::{
        can_mpt_trade, check_create_mpt, is_mpt_frozen, is_mpt_individual_frozen, require_mpt_auth,
    };
    use basics::base_uint::Uint256;
    use ledger::{ApplyViewImpl, Fees, Ledger, LedgerHeader, ReadView, ReadViewTx, ViewError};
    use protocol::{AccountID, ApplyFlags, Keylet, MPTIssue, Rules, Ter, make_mpt_id};
    use std::sync::Arc;

    #[derive(Debug)]
    struct FaultReadView {
        base: Ledger,
        fail_exists: bool,
    }

    impl ReadView for FaultReadView {
        fn open(&self) -> bool {
            false
        }

        fn header(&self) -> LedgerHeader {
            ReadView::header(&self.base)
        }

        fn fees(&self) -> Fees {
            ReadView::fees(&self.base)
        }

        fn rules(&self) -> Rules {
            ReadView::rules(&self.base)
        }

        fn exists(&self, _keylet: Keylet) -> Result<bool, ViewError> {
            if self.fail_exists {
                Err(ViewError::Conversion("injected MPT exists failure".into()))
            } else {
                Ok(false)
            }
        }

        fn succ(
            &self,
            _key: Uint256,
            _last: Option<Uint256>,
        ) -> Result<Option<Uint256>, ViewError> {
            Err(ViewError::Conversion(
                "injected MPT successor failure".into(),
            ))
        }

        fn read(&self, _keylet: Keylet) -> Result<Option<Arc<protocol::STLedgerEntry>>, ViewError> {
            Err(ViewError::Conversion("injected MPT read failure".into()))
        }

        fn sles(&self) -> Result<Vec<Arc<protocol::STLedgerEntry>>, ViewError> {
            Err(ViewError::Conversion(
                "injected MPT traversal failure".into(),
            ))
        }

        fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
            ReadView::tx_exists(&self.base, key)
        }

        fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            ReadView::tx_read(&self.base, key)
        }

        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            ReadView::txs(&self.base)
        }
    }

    fn fixture_issue() -> (MPTIssue, AccountID) {
        let issuer = AccountID::from_array([0x31; 20]);
        let holder = AccountID::from_array([0x32; 20]);
        (MPTIssue::new(make_mpt_id(7, issuer)), holder)
    }

    #[test]
    fn mpt_dex_read_failures_are_hard_bad_ledger() {
        let view = FaultReadView {
            base: Ledger::from_ledger_seq_and_close_time(1, 1, false),
            fail_exists: false,
        };
        let (issue, holder) = fixture_issue();

        assert_eq!(can_mpt_trade(&view, &issue), Err(Ter::TEF_BAD_LEDGER));
        assert_eq!(
            require_mpt_auth(&view, &issue, &holder),
            Ter::TEF_BAD_LEDGER
        );
        assert_eq!(is_mpt_frozen(&view, &issue), Err(Ter::TEF_BAD_LEDGER));
        assert_eq!(
            is_mpt_individual_frozen(&view, &issue, &holder),
            Err(Ter::TEF_BAD_LEDGER)
        );
    }

    #[test]
    fn mpt_create_exists_and_directory_read_failures_are_not_semantic_absence() {
        let (issue, holder) = fixture_issue();
        let exists_fault = Arc::new(FaultReadView {
            base: Ledger::from_ledger_seq_and_close_time(1, 1, false),
            fail_exists: true,
        });
        let mut view = ApplyViewImpl::new(exists_fault, ApplyFlags::NONE);
        assert_eq!(
            check_create_mpt(&mut view, &issue, &holder),
            Ter::TEF_BAD_LEDGER
        );

        let directory_fault = Arc::new(FaultReadView {
            base: Ledger::from_ledger_seq_and_close_time(1, 1, false),
            fail_exists: false,
        });
        let mut view = ApplyViewImpl::new(directory_fault, ApplyFlags::NONE);
        assert_eq!(
            check_create_mpt(&mut view, &issue, &holder),
            Ter::TEF_BAD_LEDGER
        );
    }
}
