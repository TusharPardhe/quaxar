//! the reference implementation parity helpers.
//!
//! This now covers the read-only account-root checks plus the pseudo-account
//! creation seam used by vault and lending transactors.

use crate::Ledger;
use crate::views::{
    apply_view::ApplyView,
    read_view::{ReadView, ViewError},
};
use basics::base_uint::Uint160;
use protocol::{
    AccountID, LedgerEntryType, LedgerFormats, SField, STAmount, STLedgerEntry, Ter, XRPAmount,
    account_keylet, get_field_by_symbol, lsfDefaultRipple, lsfDepositAuth, lsfDisableMaster,
    lsfGlobalFreeze, lsfRequireDestTag, ripesha, sha512_half_slices,
};
use shamap::traversal::TraversalError;
use std::sync::Arc;

pub const ACCOUNT_TRANSFER_RATE_PARITY: u32 = 1_000_000_000;
const MAX_PSEUDO_ACCOUNT_ATTEMPTS: u16 = 256;

/// Canonical counterpart to rippled's `isPseudoAccount(SLE::const_pointer)`.
///
/// Pseudo-account discriminator fields are identified by the ledger format's
/// metadata rather than by a hand-maintained list, so newly registered pseudo
/// account kinds automatically receive the same treatment.
pub fn is_pseudo_account(account_root: &STLedgerEntry) -> bool {
    if account_root.get_type() != LedgerEntryType::AccountRoot {
        return false;
    }

    LedgerFormats::get_instance()
        .find_by_type(LedgerEntryType::AccountRoot)
        .expect("account root format must exist")
        .so_template()
        .iter()
        .map(|element| element.sfield())
        .any(|field| {
            field.should_meta(SField::S_MD_PSEUDO_ACCOUNT) && account_root.is_field_present(field)
        })
}

fn read_account_root(
    ledger: &Ledger,
    issuer: Uint160,
) -> Result<Option<STLedgerEntry>, TraversalError> {
    ledger.read(account_keylet(issuer))
}

pub fn is_global_frozen(ledger: &Ledger, issuer: Uint160) -> Result<bool, TraversalError> {
    if issuer.is_zero() {
        return Ok(false);
    }

    Ok(read_account_root(ledger, issuer)?
        .as_ref()
        .is_some_and(|entry| entry.is_flag(lsfGlobalFreeze)))
}

pub fn transfer_rate(ledger: &Ledger, issuer: Uint160) -> Result<u32, TraversalError> {
    Ok(read_account_root(ledger, issuer)?
        .as_ref()
        .and_then(|entry| {
            entry
                .is_field_present(get_field_by_symbol("sfTransferRate"))
                .then(|| entry.get_field_u32(get_field_by_symbol("sfTransferRate")))
        })
        .unwrap_or(ACCOUNT_TRANSFER_RATE_PARITY))
}

pub fn pseudo_account_address<V: ReadView>(
    view: &V,
    pseudo_owner_key: basics::base_uint::Uint256,
) -> Result<AccountID, ViewError> {
    let parent_hash = *view.header().parent_hash.as_uint256();
    for attempt in 0..MAX_PSEUDO_ACCOUNT_ATTEMPTS {
        let hash = sha512_half_slices(&[
            &attempt.to_be_bytes(),
            parent_hash.data(),
            pseudo_owner_key.data(),
        ]);
        let candidate =
            AccountID::from_slice(&ripesha(hash.data())).expect("ripesha digest width should fit");
        if view
            .read(account_keylet(Uint160::from_void(candidate.data())))?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Ok(AccountID::default())
}

pub fn create_pseudo_account<V: ApplyView>(
    view: &mut V,
    pseudo_owner_key: basics::base_uint::Uint256,
    owner_field: &'static protocol::SField,
) -> Result<Arc<STLedgerEntry>, Ter> {
    let account_id =
        pseudo_account_address(view, pseudo_owner_key).map_err(|_| Ter::TEF_BAD_LEDGER)?;
    if account_id == AccountID::default() {
        return Err(Ter::TEC_DUPLICATE);
    }

    let sequence = if view
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"))
        || view
            .rules()
            .enabled(&protocol::feature_id("LendingProtocol"))
    {
        0
    } else {
        view.seq()
    };

    let mut entry = STLedgerEntry::from_type_and_key(
        protocol::LedgerEntryType::AccountRoot,
        account_keylet(Uint160::from_void(account_id.data())).key,
    );
    entry.set_account_id(get_field_by_symbol("sfAccount"), account_id);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(0)),
    );
    entry.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    entry.set_field_u32(
        get_field_by_symbol("sfFlags"),
        lsfDisableMaster | lsfDefaultRipple | lsfDepositAuth,
    );
    entry.set_field_h256(owner_field, pseudo_owner_key);

    let sle = Arc::new(entry);
    view.insert(sle.clone()).map_err(|_| Ter::TEF_BAD_LEDGER)?;
    Ok(sle)
}

pub fn check_destination_and_tag(to_sle: Option<&STLedgerEntry>, has_destination_tag: bool) -> Ter {
    let Some(to_sle) = to_sle else {
        return Ter::TEC_NO_DST;
    };

    if to_sle.is_flag(lsfRequireDestTag) && !has_destination_tag {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    Ter::TES_SUCCESS
}
