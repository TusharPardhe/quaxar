use super::common::*;
use ledger::{ApplyView, FlowSandbox};
use protocol::{AccountID, Asset, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry, Ter};

#[derive(Default)]
pub(super) struct ClawbackState {
    trustlines_changed: u32,
    mptokens_changed: u32,
    iou_before: Option<STLedgerEntry>,
    iou_after: Option<STLedgerEntry>,
    mpt_before: Option<STLedgerEntry>,
    mpt_after: Option<STLedgerEntry>,
}

pub(super) fn record_clawback_state(
    state: &mut ClawbackState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    if let Some(before) = before {
        match before.get_type() {
            LedgerEntryType::RippleState => {
                state.trustlines_changed = state.trustlines_changed.saturating_add(1);
                state.iou_before = Some(before.clone());
            }
            LedgerEntryType::MPToken => {
                state.mptokens_changed = state.mptokens_changed.saturating_add(1);
                state.mpt_before = Some(before.clone());
            }
            _ => {}
        }
    }

    if !is_delete && let Some(after) = after {
        match after.get_type() {
            LedgerEntryType::RippleState => state.iou_after = Some(after.clone()),
            LedgerEntryType::MPToken => state.mpt_after = Some(after.clone()),
            _ => {}
        }
    }
}

fn clawback_line_balance(
    sle: Option<&STLedgerEntry>,
    holder: AccountID,
    issuer: AccountID,
    currency: protocol::Currency,
) -> Option<STAmount> {
    let Some(sle) = sle else {
        return Some(STAmount::from_iou_amount(
            sf("sfAmount"),
            IOUAmount::new(),
            Issue::new(currency, issuer),
        ));
    };
    if sle.get_type() != LedgerEntryType::RippleState
        || *sle.key() != protocol::line(holder, issuer, currency).key
    {
        return None;
    }

    let mut balance = sle.get_field_amount(sf("sfBalance"));
    if holder > issuer {
        balance.negate();
    }
    balance.set_issuer(issuer);
    Some(balance)
}

fn legacy_violation_allowed(mptokens_v2_enabled: bool) -> bool {
    !mptokens_v2_enabled
}

pub(super) fn validates_clawback<V: ApplyView + ?Sized>(
    _sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    tx_account: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<&STAmount>,
    mptokens_v2_enabled: bool,
    state: &ClawbackState,
) -> Result<bool, ledger::ViewError> {
    if txn_type != protocol::TxType::CLAWBACK {
        return Ok(true);
    }

    if !protocol::is_tes_success(result) {
        return Ok(state.trustlines_changed == 0 && state.mptokens_changed == 0);
    }

    if state.trustlines_changed > 1 || state.mptokens_changed > 1 {
        return Ok(false);
    }
    if state.trustlines_changed != 0 && state.mptokens_changed != 0 && mptokens_v2_enabled {
        return Ok(false);
    }

    let should_check_balance =
        state.trustlines_changed == 1 || (mptokens_v2_enabled && state.mptokens_changed == 1);
    if !should_check_balance {
        return Ok(true);
    }

    let (Some(issuer), Some(amount)) = (tx_account, tx_amount) else {
        return Ok(false);
    };

    match amount.asset() {
        Asset::Issue(issue) => {
            let holder = issue.account;
            let before =
                clawback_line_balance(state.iou_before.as_ref(), holder, issuer, issue.currency);
            let after =
                clawback_line_balance(state.iou_after.as_ref(), holder, issuer, issue.currency);
            let (Some(before), Some(after)) = (before, after) else {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            };
            if after.signum() < 0 {
                return Ok(false);
            }
            if amount.signum() <= 0 {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            }

            let mut claw_amount = amount.clone();
            claw_amount.set_issuer(issuer);
            let expected = if before < claw_amount {
                before.clone()
            } else {
                claw_amount
            };
            if after > before || before - after != expected {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            }
            Ok(true)
        }
        Asset::MPTIssue(issue) => {
            let Some(holder) = tx_holder else {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            };
            let (Some(before_sle), Some(after_sle)) =
                (state.mpt_before.as_ref(), state.mpt_after.as_ref())
            else {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            };
            if before_sle.get_account_id(sf("sfAccount")) != holder
                || after_sle.get_account_id(sf("sfAccount")) != holder
                || before_sle.get_field_h192(sf("sfMPTokenIssuanceID")) != issue.mpt_id()
                || after_sle.get_field_h192(sf("sfMPTokenIssuanceID")) != issue.mpt_id()
            {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            }

            let before = before_sle.get_field_u64(sf("sfMPTAmount"));
            let after = after_sle.get_field_u64(sf("sfMPTAmount"));
            if amount.signum() <= 0 {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            }
            let claw_amount = amount.mpt().value() as u64;
            if after > before || before - after != before.min(claw_amount) {
                return Ok(legacy_violation_allowed(mptokens_v2_enabled));
            }
            Ok(true)
        }
    }
}
