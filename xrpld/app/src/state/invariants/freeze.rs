use std::collections::BTreeMap;

use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, Issue, LedgerEntryType, STLedgerEntry};

use super::common::{raw_account_id, sf};

#[derive(Clone)]
struct BalanceChange {
    line: STLedgerEntry,
    sign: i8,
}

#[derive(Default)]
struct IssuerChanges {
    senders: Vec<BalanceChange>,
    receivers: Vec<BalanceChange>,
}

#[derive(Default)]
pub(super) struct FreezeState {
    issuers: BTreeMap<AccountID, STLedgerEntry>,
    changes: BTreeMap<Issue, IssuerChanges>,
}

pub(super) const fn freeze_override_allowed(
    has_override_privilege: bool,
    fix_cleanup_3_4_0: bool,
    is_amm_line: bool,
    global_freeze: bool,
) -> bool {
    has_override_privilege && (fix_cleanup_3_4_0 || !is_amm_line || global_freeze)
}

fn line_issue(line: &STLedgerEntry, field: &str, currency: protocol::Currency) -> Issue {
    Issue::new(currency, line.get_field_amount(sf(field)).issue().account)
}

pub(super) fn record_freeze_state(
    state: &mut FreezeState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: &STLedgerEntry,
) {
    if after.get_type() == LedgerEntryType::AccountRoot {
        state
            .issuers
            .insert(after.get_account_id(sf("sfAccount")), after.clone());
        return;
    }
    if after.get_type() != LedgerEntryType::RippleState
        || before.is_some_and(|sle| sle.get_type() != LedgerEntryType::RippleState)
    {
        return;
    }

    let template = after.get_field_amount(sf("sfBalance"));
    let before_balance = before
        .map(|sle| sle.get_field_amount(sf("sfBalance")))
        .unwrap_or_else(|| template.zeroed());
    let after_balance = if is_delete {
        template.zeroed()
    } else {
        template.clone()
    };
    let sign = after_balance - before_balance;
    let sign = sign.signum();
    if sign == 0 {
        return;
    }
    let sign = sign as i8;
    let currency = template.issue().currency;
    for (issue, direction) in [
        (line_issue(after, "sfHighLimit", currency), sign),
        (line_issue(after, "sfLowLimit", currency), -sign),
    ] {
        let change = BalanceChange {
            line: after.clone(),
            sign: direction,
        };
        let changes = state.changes.entry(issue).or_default();
        if direction < 0 {
            changes.senders.push(change);
        } else {
            changes.receivers.push(change);
        }
    }
}

pub(super) fn validates_transfers_not_frozen<V: ApplyView + ?Sized>(
    view: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    state: &FreezeState,
) -> Result<bool, ledger::ViewError> {
    let enforce = view.rules().enabled(&protocol::feature_id("DeepFreeze"));
    if !enforce {
        return Ok(true);
    }
    let override_freeze = txn_type == protocol::TxType::AMM_CLAWBACK;
    let fix_override = view
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_4_0"));

    for (issue, changes) in &state.changes {
        let issuer = if let Some(issuer) = state.issuers.get(&issue.account) {
            Some(issuer.clone())
        } else {
            view.read(protocol::account_keylet(raw_account_id(issue.account)))?
                .map(|sle| (*sle).clone())
        };
        let Some(issuer) = issuer else {
            return Ok(false);
        };
        if changes.senders.is_empty() || changes.receivers.is_empty() {
            continue;
        }
        let global = issuer.is_flag(protocol::lsfGlobalFreeze);
        let valid = changes
            .senders
            .iter()
            .chain(changes.receivers.iter())
            .all(|change| {
                let high = change
                    .line
                    .get_field_amount(sf("sfLowLimit"))
                    .issue()
                    .account
                    == issue.account;
                let freeze = change.sign < 0
                    && change.line.is_flag(if high {
                        protocol::lsfLowFreeze
                    } else {
                        protocol::lsfHighFreeze
                    });
                let deep = change.line.is_flag(if high {
                    protocol::lsfLowDeepFreeze
                } else {
                    protocol::lsfHighDeepFreeze
                });
                if !(global || deep || freeze) {
                    return true;
                }
                let amm_line = change.line.is_flag(protocol::lsfAMMNode);
                freeze_override_allowed(override_freeze, fix_override, amm_line, global)
            });
        if !valid {
            return Ok(false);
        }
    }
    Ok(true)
}
