//! Sponsor reserve helpers ported from `xrpl/ledger/helpers/SponsorHelpers.h`.
//!
//! These utilities determine whether a transaction has fee/reserve sponsorship,
//! retrieve the sponsor SLE, and manage the sponsor field on ledger objects.

use protocol::{AccountID, STLedgerEntry, TxType, get_field_by_symbol};

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

/// Extract the sponsor AccountID from an STLedgerEntry, defaulting to `sfSponsor`.
pub fn get_ledger_entry_sponsor(sle: &STLedgerEntry) -> Option<AccountID> {
    let field = get_field_by_symbol("sfSponsor");
    if sle.is_field_present(field) {
        Some(sle.get_account_id(field))
    } else {
        None
    }
}

/// Extract the high-side sponsor from a RippleState entry.
pub fn get_ledger_entry_high_sponsor(sle: &STLedgerEntry) -> Option<AccountID> {
    let field = get_field_by_symbol("sfHighSponsor");
    if sle.is_field_present(field) {
        Some(sle.get_account_id(field))
    } else {
        None
    }
}

/// Extract the low-side sponsor from a RippleState entry.
pub fn get_ledger_entry_low_sponsor(sle: &STLedgerEntry) -> Option<AccountID> {
    let field = get_field_by_symbol("sfLowSponsor");
    if sle.is_field_present(field) {
        Some(sle.get_account_id(field))
    } else {
        None
    }
}
