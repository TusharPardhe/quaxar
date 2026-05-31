use super::common::*;
use ledger::{ApplyView, FlowSandbox};
use protocol::{AccountID, Asset, Issue, LedgerEntryType, STAmount, STLedgerEntry, Ter};

#[derive(Default)]
pub(super) struct ClawbackState {
    trustlines_changed: u32,
    mptokens_changed: u32,
}
pub(super) fn record_clawback_state(state: &mut ClawbackState, before: Option<&STLedgerEntry>) {
    match before.map(STLedgerEntry::get_type) {
        Some(LedgerEntryType::RippleState) => {
            state.trustlines_changed = state.trustlines_changed.saturating_add(1);
        }
        Some(LedgerEntryType::MPToken) => {
            state.mptokens_changed = state.mptokens_changed.saturating_add(1);
        }
        _ => {}
    }
}

pub(super) fn validates_clawback<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    tx_account: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<&STAmount>,
    mptokens_v2_enabled: bool,
    state: &ClawbackState,
) -> bool {
    if txn_type != protocol::TxType::CLAWBACK {
        return true;
    }

    if !protocol::is_tes_success(result) {
        return state.trustlines_changed == 0 && state.mptokens_changed == 0;
    }

    if state.trustlines_changed > 1 || state.mptokens_changed > 1 {
        return false;
    }

    let should_check_balance =
        state.trustlines_changed == 1 || (mptokens_v2_enabled && state.mptokens_changed == 1);
    if !should_check_balance {
        return true;
    }

    let (Some(issuer), Some(amount)) = (tx_account, tx_amount) else {
        return false;
    };

    let (holder, asset) = match amount.asset() {
        Asset::Issue(issue) => (
            issue.account,
            Asset::Issue(Issue {
                currency: issue.currency,
                account: issuer,
            }),
        ),
        Asset::MPTIssue(issue) => {
            let Some(holder) = tx_holder else {
                return false;
            };
            (holder, Asset::MPTIssue(issue))
        }
    };

    account_holds_asset_amount(sandbox, holder, asset, sf("sfAmount"))
        .is_some_and(|balance| balance.signum() >= 0)
}
