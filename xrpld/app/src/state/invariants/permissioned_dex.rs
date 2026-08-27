use super::common::*;
use basics::base_uint::Uint256;
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{LedgerEntryType, STLedgerEntry, Ter};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct PermissionedDexState {
    domains_old: BTreeSet<Uint256>,
    domains: BTreeSet<Uint256>,
    regular_offers_old: bool,
    regular_offers: bool,
    bad_hybrids_old: bool,
    bad_hybrids: bool,
}

pub(super) fn permissioned_dex_consumed_wrong_domain(
    state: &PermissionedDexState,
    domain: Uint256,
    fix_cleanup_3_4_0: bool,
) -> bool {
    let domains = if fix_cleanup_3_4_0 {
        &state.domains
    } else {
        &state.domains_old
    };
    domains.iter().any(|candidate| *candidate != domain)
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
                let domain = after.get_field_h256(sf("sfDomainID"));
                state.domains_old.insert(domain);
                if !is_delete {
                    state.domains.insert(domain);
                }
            }
        }
        LedgerEntryType::Offer => {
            if after.is_field_present(sf("sfDomainID")) {
                let domain = after.get_field_h256(sf("sfDomainID"));
                state.domains_old.insert(domain);
                if !is_delete {
                    state.domains.insert(domain);
                }
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

pub(super) fn validates_permissioned_dex<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    tx_domain: Option<Uint256>,
    fix_cleanup_3_1_3: bool,
    fix_cleanup_3_2_0: bool,
    fix_cleanup_3_4_0: bool,
    state: &PermissionedDexState,
) -> Result<bool, ledger::ViewError> {
    if !matches!(
        txn_type,
        protocol::TxType::PAYMENT | protocol::TxType::OFFER_CREATE
    ) || !protocol::is_tes_success(result)
    {
        return Ok(true);
    }

    let malformed_hybrid = if fix_cleanup_3_1_3 {
        state.bad_hybrids
    } else {
        state.bad_hybrids_old
    };
    if txn_type == protocol::TxType::OFFER_CREATE && malformed_hybrid {
        return Ok(false);
    }

    let Some(domain) = tx_domain else {
        return Ok(true);
    };

    if sandbox
        .read(protocol::permissioned_domain_keylet_from_id(domain))?
        .is_none()
    {
        return Ok(false);
    }

    if permissioned_dex_consumed_wrong_domain(state, domain, fix_cleanup_3_4_0) {
        return Ok(false);
    }

    let has_regular_offers = if fix_cleanup_3_2_0 {
        state.regular_offers
    } else {
        state.regular_offers_old
    };
    Ok(!has_regular_offers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{LedgerEntryType, get_field_by_symbol};

    #[test]
    fn deleted_domain_offer_is_legacy_only_domain_evidence() {
        let domain = Uint256::from_array([0x44; 32]);
        let mut offer = STLedgerEntry::from_type_and_key(
            LedgerEntryType::Offer,
            Uint256::from_array([0x55; 32]),
        );
        offer.set_field_h256(get_field_by_symbol("sfDomainID"), domain);
        let mut state = PermissionedDexState::default();
        record_permissioned_dex(&mut state, true, Some(&offer), None);
        assert!(state.domains_old.contains(&domain));
        assert!(!state.domains.contains(&domain));
    }
}
