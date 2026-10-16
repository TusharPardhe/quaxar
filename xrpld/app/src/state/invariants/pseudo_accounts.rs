use super::common::{raw_account_id, sf};
use basics::base_uint::Uint256;
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, Asset, LedgerEntryType, STLedgerEntry};
use std::collections::BTreeMap;

/// Owner-directory entries added by the transaction, keyed by their owner.
/// Pseudo-account classification intentionally happens after the full delta is
/// applied, matching rippled's `ValidPseudoAccounts::finalize` behavior.
#[derive(Default)]
pub(super) struct PseudoAccountDirectoryState {
    additions: BTreeMap<AccountID, Vec<Uint256>>,
}

pub(super) fn record_pseudo_account_directory_state(
    state: &mut PseudoAccountDirectoryState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    if is_delete {
        return;
    }
    let Some(after) = after else {
        return;
    };
    if after.get_type() != LedgerEntryType::DirectoryNode || !after.is_field_present(sf("sfOwner")) {
        return;
    }

    let before_indexes = before
        .filter(|entry| entry.get_type() == LedgerEntryType::DirectoryNode)
        .map(|entry| entry.get_field_v256(sf("sfIndexes")).value().to_vec())
        .unwrap_or_default();
    let after_indexes = after.get_field_v256(sf("sfIndexes"));
    let added: Vec<_> = after_indexes
        .value()
        .iter()
        .copied()
        .filter(|index| !before_indexes.contains(index))
        .collect();
    state
        .additions
        .entry(after.get_account_id(sf("sfOwner")))
        .or_default()
        .extend(added);
}

fn is_holding(entry: &STLedgerEntry, asset: Asset, account: AccountID) -> bool {
    match asset {
        Asset::Issue(issue) if issue.native() => false,
        Asset::Issue(issue) => *entry.key() == protocol::line(account, issue.account, issue.currency).key,
        Asset::MPTIssue(issue) => {
            *entry.key()
                == protocol::mptoken_keylet_from_mptid(issue.mpt_id(), raw_account_id(account)).key
        }
    }
}

fn owned_by_pseudo_account<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    pseudo: &STLedgerEntry,
    entry: &STLedgerEntry,
) -> Result<bool, ledger::ViewError> {
    let pseudo_account = pseudo.get_account_id(sf("sfAccount"));

    if pseudo.is_field_present(sf("sfAMMID")) {
        let amm_id = pseudo.get_field_h256(sf("sfAMMID"));
        if entry.get_type() == LedgerEntryType::AMM {
            return Ok(*entry.key() == amm_id);
        }
        let Some(amm) = sandbox.read(protocol::Keylet::new(LedgerEntryType::AMM, amm_id))? else {
            return Ok(false);
        };
        let asset = amm.get_field_issue(sf("sfAsset")).asset();
        let asset2 = amm.get_field_issue(sf("sfAsset2")).asset();
        if is_holding(entry, asset, pseudo_account) || is_holding(entry, asset2, pseudo_account) {
            return Ok(true);
        }
        if entry.get_type() != LedgerEntryType::RippleState {
            return Ok(false);
        }
        let low = entry.get_field_amount(sf("sfLowLimit")).issue().account;
        let high = entry.get_field_amount(sf("sfHighLimit")).issue().account;
        let lp_currency = amm.get_field_amount(sf("sfLPTokenBalance")).issue().currency;
        return Ok((low == pseudo_account || high == pseudo_account)
            && *entry.key() == protocol::line(low, high, lp_currency).key);
    }

    if pseudo.is_field_present(sf("sfVaultID")) {
        let vault_id = pseudo.get_field_h256(sf("sfVaultID"));
        if entry.get_type() == LedgerEntryType::LoanBroker {
            return Ok(entry.get_field_h256(sf("sfVaultID")) == vault_id);
        }
        let Some(vault) = sandbox.read(protocol::vault_keylet_from_key(vault_id))? else {
            return Ok(false);
        };
        return Ok(*entry.key()
            == protocol::mpt_issuance_keylet_from_mptid(vault.get_field_h192(sf("sfShareMPTID"))).key
            || is_holding(entry, vault.get_field_issue(sf("sfAsset")).asset(), pseudo_account));
    }

    if pseudo.is_field_present(sf("sfLoanBrokerID")) {
        let broker_id = pseudo.get_field_h256(sf("sfLoanBrokerID"));
        if entry.get_type() == LedgerEntryType::Loan {
            return Ok(entry.get_field_h256(sf("sfLoanBrokerID")) == broker_id);
        }
        let Some(broker) = sandbox.read(protocol::loan_broker_keylet_from_key(broker_id))? else {
            return Ok(false);
        };
        let Some(vault) = sandbox.read(protocol::vault_keylet_from_key(
            broker.get_field_h256(sf("sfVaultID")),
        ))? else {
            return Ok(false);
        };
        return Ok(is_holding(
            entry,
            vault.get_field_issue(sf("sfAsset")).asset(),
            pseudo_account,
        ));
    }

    Ok(false)
}

pub(super) fn validates_pseudo_account_directory_contents<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    state: &PseudoAccountDirectoryState,
) -> Result<bool, ledger::ViewError> {
    for (owner, indexes) in &state.additions {
        let Some(root) = sandbox.read(protocol::account_keylet(raw_account_id(*owner)))? else {
            continue;
        };
        if !ledger::is_pseudo_account(&root) {
            continue;
        }
        for index in indexes {
            let Some(entry) = sandbox.read(protocol::unchecked_keylet(*index))? else {
                return Ok(false);
            };
            if !owned_by_pseudo_account(sandbox, &root, &entry)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
