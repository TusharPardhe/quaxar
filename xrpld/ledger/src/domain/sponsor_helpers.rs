//! Sponsor reserve helpers ported from `xrpl/ledger/helpers/SponsorHelpers.h`.
//!
//! These utilities determine whether a transaction has fee/reserve sponsorship,
//! retrieve the sponsor SLE, and manage the sponsor field on ledger objects.

use crate::{ApplyView, ViewError};
use protocol::{AccountID, STLedgerEntry, TxType, get_field_by_symbol};
use std::sync::Arc;

/// Sponsor flags on the transaction `sfSponsorFlags` field.
pub const SPF_SPONSOR_FEE: u32 = 1;
pub const SPF_SPONSOR_RESERVE: u32 = 2;
pub const SPF_SPONSOR_FLAG_MASK: u32 = !(SPF_SPONSOR_FEE | SPF_SPONSOR_RESERVE);

/// Returns true if the transaction has fee sponsorship enabled.
pub fn is_fee_sponsored(sponsor_flags: u32) -> bool {
    (sponsor_flags & SPF_SPONSOR_FEE) != 0
}

/// Returns true if the transaction has reserve sponsorship enabled.
pub fn is_reserve_sponsored(sponsor_flags: u32) -> bool {
    (sponsor_flags & SPF_SPONSOR_RESERVE) != 0
}

/// Returns the set of transaction types that are allowed to use reserve
/// sponsorship (spfSponsorReserve). This matches the v1 explicit allow-list
/// from the C++ reference implementation.
pub fn reserve_sponsor_allowed_tx_types() -> &'static [TxType] {
    &[
        TxType::DELEGATE_SET,
        TxType::DEPOSIT_PREAUTH,
        TxType::PAYMENT,
        TxType::SIGNER_LIST_SET,
        TxType::CHECK_CANCEL,
        TxType::CHECK_CASH,
        TxType::CHECK_CREATE,
        TxType::ESCROW_CANCEL,
        TxType::ESCROW_CREATE,
        TxType::ESCROW_FINISH,
        TxType::PAYCHAN_CLAIM,
        TxType::PAYCHAN_CREATE,
        TxType::PAYCHAN_FUND,
        TxType::CLAWBACK,
        TxType::MPTOKEN_AUTHORIZE,
        TxType::MPTOKEN_ISSUANCE_CREATE,
        TxType::MPTOKEN_ISSUANCE_DESTROY,
        TxType::MPTOKEN_ISSUANCE_SET,
        TxType::TRUST_SET,
        TxType::CREDENTIAL_ACCEPT,
        TxType::CREDENTIAL_CREATE,
        TxType::CREDENTIAL_DELETE,
        TxType::ACCOUNT_SET,
        TxType::REGULAR_KEY_SET,
    ]
}

/// Returns `true` if the given transaction type is allowed to carry
/// `spfSponsorReserve` in `sfSponsorFlags`.
pub fn is_reserve_sponsor_allowed(tx_type: TxType) -> bool {
    reserve_sponsor_allowed_tx_types().contains(&tx_type)
}

/// Return the reserve-bearing owner count after sponsor accounting.
pub fn reserve_owner_count(sle: &STLedgerEntry, adjustment: i32) -> u32 {
    let owner = sle.get_field_u32(get_field_by_symbol("sfOwnerCount")) as i64;
    let sponsored = sle.get_field_u32(get_field_by_symbol("sfSponsoredOwnerCount")) as i64;
    let sponsoring = sle.get_field_u32(get_field_by_symbol("sfSponsoringOwnerCount")) as i64;
    (owner + adjustment as i64 - sponsored + sponsoring).clamp(0, u32::MAX as i64) as u32
}

/// Add one owned object, assigning its reserve to `sponsor_sle` when present.
pub fn increase_owner_count_for_object(
    view: &mut dyn ApplyView,
    account_sle: &Arc<STLedgerEntry>,
    sponsor_sle: Option<&Arc<STLedgerEntry>>,
) -> Result<(), ViewError> {
    let owner_field = get_field_by_symbol("sfOwnerCount");
    let sponsored_field = get_field_by_symbol("sfSponsoredOwnerCount");
    let sponsoring_field = get_field_by_symbol("sfSponsoringOwnerCount");
    let account = account_sle.get_account_id(get_field_by_symbol("sfAccount"));
    let current = account_sle.get_field_u32(owner_field);
    let next = current.saturating_add(1);
    view.adjust_owner_count_hook(account, current, next);

    let mut account_obj = account_sle.clone_as_object();
    account_obj.set_field_u32(owner_field, next);
    if sponsor_sle.is_some() {
        account_obj.set_field_u32(
            sponsored_field,
            account_sle.get_field_u32(sponsored_field).saturating_add(1),
        );
    }
    view.update(Arc::new(STLedgerEntry::from_stobject(
        account_obj,
        *account_sle.key(),
    )))?;

    if let Some(sponsor_sle) = sponsor_sle {
        let sponsor = sponsor_sle.get_account_id(get_field_by_symbol("sfAccount"));
        let mut sponsor_obj = sponsor_sle.clone_as_object();
        sponsor_obj.set_field_u32(
            sponsoring_field,
            sponsor_sle
                .get_field_u32(sponsoring_field)
                .saturating_add(1),
        );
        view.update(Arc::new(STLedgerEntry::from_stobject(
            sponsor_obj,
            *sponsor_sle.key(),
        )))?;

        // A prefunded Sponsorship consumes one remaining reserve assignment
        // when a new object is assigned to the sponsor. Directly authorized
        // sponsorship has no Sponsorship SLE and therefore no counter here.
        let sponsorship_keylet = protocol::sponsorship_keylet(
            basics::base_uint::Uint160::from_void(sponsor.data()),
            basics::base_uint::Uint160::from_void(account.data()),
        );
        if let Some(sponsorship_sle) = view.peek(sponsorship_keylet)? {
            let remaining_field = get_field_by_symbol("sfRemainingOwnerCount");
            let mut sponsorship_obj = sponsorship_sle.clone_as_object();
            sponsorship_obj.set_field_u32(
                remaining_field,
                sponsorship_sle
                    .get_field_u32(remaining_field)
                    .saturating_sub(1),
            );
            view.update(Arc::new(STLedgerEntry::from_stobject(
                sponsorship_obj,
                *sponsorship_sle.key(),
            )))?;
        }
    }
    Ok(())
}

/// Remove reserve units for an existing owned object, deriving any reserve
/// sponsor from the object's `sfSponsor` field. This is the Rust equivalent
/// of rippled's `decreaseOwnerCountForObject`.
pub fn decrease_owner_count_for_object(
    view: &mut dyn ApplyView,
    account_sle: &Arc<STLedgerEntry>,
    object_sle: &Arc<STLedgerEntry>,
    count: u32,
) -> Result<(), ViewError> {
    let owner_field = get_field_by_symbol("sfOwnerCount");
    let sponsored_field = get_field_by_symbol("sfSponsoredOwnerCount");
    let sponsoring_field = get_field_by_symbol("sfSponsoringOwnerCount");
    let sponsor_field = get_field_by_symbol("sfSponsor");
    let account = account_sle.get_account_id(get_field_by_symbol("sfAccount"));

    let current = account_sle.get_field_u32(owner_field);
    let next = current.saturating_sub(count);
    let mut account_obj = account_sle.clone_as_object();
    account_obj.set_field_u32(owner_field, next);

    if object_sle.is_field_present(sponsor_field) {
        let sponsor = object_sle.get_account_id(sponsor_field);
        let sponsor_sle = view
            .peek(protocol::account_keylet(
                basics::base_uint::Uint160::from_void(sponsor.data()),
            ))?
            .ok_or_else(|| ViewError::Conversion("reserve sponsor account missing".into()))?;

        account_obj.set_field_u32(
            sponsored_field,
            account_sle
                .get_field_u32(sponsored_field)
                .saturating_sub(count),
        );
        let mut sponsor_obj = sponsor_sle.clone_as_object();
        sponsor_obj.set_field_u32(
            sponsoring_field,
            sponsor_sle
                .get_field_u32(sponsoring_field)
                .saturating_sub(count),
        );
        view.update(Arc::new(STLedgerEntry::from_stobject(
            sponsor_obj,
            *sponsor_sle.key(),
        )))?;
    }

    view.adjust_owner_count_hook(account, current, next);
    view.update(Arc::new(STLedgerEntry::from_stobject(
        account_obj,
        *account_sle.key(),
    )))?;
    Ok(())
}

/// Remove one reserve unit from a RippleState side. RippleState stores the
/// reserve sponsor in `sfLowSponsor`/`sfHighSponsor`, rather than `sfSponsor`.
pub fn decrease_owner_count_for_trust_line(
    view: &mut dyn ApplyView,
    account_sle: &Arc<STLedgerEntry>,
    sponsor: Option<AccountID>,
) -> Result<(), ViewError> {
    let owner_field = get_field_by_symbol("sfOwnerCount");
    let sponsored_field = get_field_by_symbol("sfSponsoredOwnerCount");
    let sponsoring_field = get_field_by_symbol("sfSponsoringOwnerCount");
    let account = account_sle.get_account_id(get_field_by_symbol("sfAccount"));
    let current = account_sle.get_field_u32(owner_field);
    let next = current.saturating_sub(1);
    let mut account_obj = account_sle.clone_as_object();
    account_obj.set_field_u32(owner_field, next);

    if let Some(sponsor) = sponsor {
        let sponsor_sle = view
            .peek(protocol::account_keylet(
                basics::base_uint::Uint160::from_void(sponsor.data()),
            ))?
            .ok_or_else(|| ViewError::Conversion("reserve sponsor account missing".into()))?;
        account_obj.set_field_u32(
            sponsored_field,
            account_sle.get_field_u32(sponsored_field).saturating_sub(1),
        );
        let mut sponsor_obj = sponsor_sle.clone_as_object();
        sponsor_obj.set_field_u32(
            sponsoring_field,
            sponsor_sle
                .get_field_u32(sponsoring_field)
                .saturating_sub(1),
        );
        view.update(Arc::new(STLedgerEntry::from_stobject(
            sponsor_obj,
            *sponsor_sle.key(),
        )))?;
    }

    view.adjust_owner_count_hook(account, current, next);
    view.update(Arc::new(STLedgerEntry::from_stobject(
        account_obj,
        *account_sle.key(),
    )))
}

/// Extract the sponsor AccountID from an STLedgerEntry, defaulting to `sfSponsor`.
/// Reserved rippled-parity helper; unused until sponsor-reserve flow work lands.
#[allow(dead_code)]
pub fn get_ledger_entry_sponsor(sle: &STLedgerEntry) -> Option<AccountID> {
    let field = get_field_by_symbol("sfSponsor");
    if sle.is_field_present(field) {
        Some(sle.get_account_id(field))
    } else {
        None
    }
}

/// Extract the high-side sponsor from a RippleState entry.
/// Reserved rippled-parity helper; unused until sponsor-reserve flow work lands.
#[allow(dead_code)]
pub fn get_ledger_entry_high_sponsor(sle: &STLedgerEntry) -> Option<AccountID> {
    let field = get_field_by_symbol("sfHighSponsor");
    if sle.is_field_present(field) {
        Some(sle.get_account_id(field))
    } else {
        None
    }
}

/// Extract the low-side sponsor from a RippleState entry.
/// Reserved rippled-parity helper; unused until sponsor-reserve flow work lands.
#[allow(dead_code)]
pub fn get_ledger_entry_low_sponsor(sle: &STLedgerEntry) -> Option<AccountID> {
    let field = get_field_by_symbol("sfLowSponsor");
    if sle.is_field_present(field) {
        Some(sle.get_account_id(field))
    } else {
        None
    }
}
