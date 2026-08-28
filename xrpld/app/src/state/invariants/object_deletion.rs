use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, LedgerEntryType, STLedgerEntry};

use super::common::{raw_account_id, sf};

#[derive(Default)]
pub(super) struct ObjectDeletionState {
    pub deleted_pseudo_accounts: Vec<AccountID>,
    pub deleted_object_missing_account: bool,
}

pub(super) fn record_object_deletion_state(
    state: &mut ObjectDeletionState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
) {
    if !is_delete {
        return;
    }

    let sle = match before {
        Some(sle) => sle,
        None => return,
    };

    match sle.get_type() {
        LedgerEntryType::AMM | LedgerEntryType::Vault | LedgerEntryType::LoanBroker => {
            if sle.is_field_present(sf("sfAccount")) {
                let pseudo_id = sle.get_account_id(sf("sfAccount"));
                state.deleted_pseudo_accounts.push(pseudo_id);
            } else {
                state.deleted_object_missing_account = true;
            }
        }
        _ => {}
    }
}

pub(super) fn validates_object_deletion<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    state: &ObjectDeletionState,
) -> Result<bool, ledger::ViewError> {
    if state.deleted_object_missing_account {
        return Ok(false);
    }
    for pseudo_id in &state.deleted_pseudo_accounts {
        let keylet = protocol::account_keylet(raw_account_id(*pseudo_id));
        if sandbox.read(keylet)?.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}
