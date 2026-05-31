use super::common::*;
use basics::base_uint::Uint256;
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{LedgerEntryType, STLedgerEntry, Ter};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct PermissionedDexState {
    domains: BTreeSet<Uint256>,
    regular_offers_old: bool,
    regular_offers: bool,
    bad_hybrids_old: bool,
    bad_hybrids: bool,
}

pub(super) fn record_permissioned_dex(
    state: &mut PermissionedDexState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    let Some(after) = after.or(if is_delete { before } else { None }) else {
        return;
    };

    match after.get_type() {
        LedgerEntryType::DirectoryNode => {
            if after.is_field_present(sf("sfDomainID")) {
                state.domains.insert(after.get_field_h256(sf("sfDomainID")));
            }
        }
        LedgerEntryType::Offer => {
            if after.is_field_present(sf("sfDomainID")) {
                state.domains.insert(after.get_field_h256(sf("sfDomainID")));
            } else {
                state.regular_offers_old = true;
                if !is_delete {
                    state.regular_offers = true;
                }
            }

            if after.is_flag(protocol::lsfHybrid) {
                let has_domain = after.is_field_present(sf("sfDomainID"));
                let additional_len = if after.is_field_present(sf("sfAdditionalBooks")) {
                    Some(after.get_field_array(sf("sfAdditionalBooks")).len())
                } else {
                    None
                };

                if !has_domain || additional_len.is_none_or(|len| len > 1) {
                    state.bad_hybrids_old = true;
                }
                if !has_domain || additional_len != Some(1) {
                    state.bad_hybrids = true;
                }
            }
        }
        _ => {}
    }
}

pub(super) fn validates_permissioned_dex<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    tx_domain: Option<Uint256>,
    fix_cleanup_3_1_3: bool,
    fix_cleanup_3_2_0: bool,
    state: &PermissionedDexState,
) -> bool {
    if !matches!(
        txn_type,
        protocol::TxType::PAYMENT | protocol::TxType::OFFER_CREATE
    ) || !protocol::is_tes_success(result)
    {
        return true;
    }

    let malformed_hybrid = if fix_cleanup_3_1_3 {
        state.bad_hybrids
    } else {
        state.bad_hybrids_old
    };
    if txn_type == protocol::TxType::OFFER_CREATE && malformed_hybrid {
        return false;
    }

    let Some(domain) = tx_domain else {
        return true;
    };

    if !matches!(
        sandbox.read(protocol::permissioned_domain_keylet_from_id(domain)),
        Ok(Some(_))
    ) {
        return false;
    }

    if state.domains.iter().any(|candidate| *candidate != domain) {
        return false;
    }

    let has_regular_offers = if fix_cleanup_3_2_0 {
        state.regular_offers
    } else {
        state.regular_offers_old
    };
    !has_regular_offers
}
