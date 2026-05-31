use super::common::*;
use super::mpt::{max_mpt_token_amount, mpt_max_amount};
use protocol::{LedgerEntryType, STLedgerEntry};

pub(super) fn validate_mpt_entry(sle: &STLedgerEntry) -> bool {
    match sle.get_type() {
        LedgerEntryType::MPTokenIssuance => {
            let outstanding = optional_u64(sle, sf("sfOutstandingAmount"));
            let locked = optional_u64(sle, sf("sfLockedAmount"));
            outstanding <= mpt_max_amount(sle) && locked <= outstanding
        }
        LedgerEntryType::MPToken => {
            let account = sle.get_account_id(sf("sfAccount"));
            let id = sle.get_field_h192(sf("sfMPTokenIssuanceID"));
            let mpt_amount = optional_u64(sle, sf("sfMPTAmount"));
            let locked = optional_u64(sle, sf("sfLockedAmount"));
            let max_amount = max_mpt_token_amount();
            account != protocol::MPTIssue::new(id).issuer()
                && mpt_amount <= max_amount
                && locked <= max_amount.saturating_sub(mpt_amount)
        }
        _ => true,
    }
}

pub(super) fn validate_amm_entry(sle: &STLedgerEntry) -> bool {
    let asset = sle.get_field_issue(sf("sfAsset")).asset();
    let asset2 = sle.get_field_issue(sf("sfAsset2")).asset();
    if asset == asset2 {
        return false;
    }

    let lp_tokens = sle.get_field_amount(sf("sfLPTokenBalance"));
    if lp_tokens.negative() || lp_tokens.signum() == 0 {
        return false;
    }

    if sle.is_field_present(sf("sfTradingFee")) && sle.get_field_u16(sf("sfTradingFee")) > 1000 {
        return false;
    }

    if sle.is_field_present(sf("sfAuctionSlot")) {
        let slot = sle.get_field_object(sf("sfAuctionSlot"));
        if slot.is_field_present(sf("sfAuthAccounts"))
            && slot.get_field_array(sf("sfAuthAccounts")).iter().count() > 4
        {
            return false;
        }
    }

    true
}

pub(super) fn validate_ripple_state_entry(sle: &STLedgerEntry) -> bool {
    if sle.get_field_amount(sf("sfLowLimit")).asset().native()
        || sle.get_field_amount(sf("sfHighLimit")).asset().native()
    {
        return false;
    }

    let flags = if sle.is_field_present(sf("sfFlags")) {
        sle.get_field_u32(sf("sfFlags"))
    } else {
        0
    };
    let low_freeze = (flags & protocol::lsfLowFreeze) != 0;
    let low_deep_freeze = (flags & protocol::lsfLowDeepFreeze) != 0;
    let high_freeze = (flags & protocol::lsfHighFreeze) != 0;
    let high_deep_freeze = (flags & protocol::lsfHighDeepFreeze) != 0;

    !(low_deep_freeze && !low_freeze || high_deep_freeze && !high_freeze)
}
