use super::common::*;
use protocol::{AccountID, LedgerEntryType, STLedgerEntry, Ter};

pub(super) struct PermissionedDomainStatus {
    credentials_size: usize,
    sorted: bool,
    unique: bool,
    deleted: bool,
}

#[derive(Default)]
pub(super) struct PermissionedDomainState {
    statuses: Vec<PermissionedDomainStatus>,
}

pub(super) fn credential_sort_key(credential: &protocol::STObject) -> (AccountID, Vec<u8>) {
    (
        credential.get_account_id(sf("sfIssuer")),
        credential.get_field_vl(sf("sfCredentialType")),
    )
}

pub(super) fn record_permissioned_domain_state(
    state: &mut PermissionedDomainState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    let candidate = if is_delete { before } else { after };
    let Some(sle) = candidate else {
        return;
    };
    if sle.get_type() != LedgerEntryType::PermissionedDomain {
        return;
    }

    let credentials = sle.get_field_array(sf("sfAcceptedCredentials"));
    let keys = credentials
        .iter()
        .map(credential_sort_key)
        .collect::<Vec<_>>();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    sorted_keys.dedup();

    state.statuses.push(PermissionedDomainStatus {
        credentials_size: keys.len(),
        sorted: keys == sorted_keys,
        unique: keys.len() == sorted_keys.len(),
        deleted: is_delete,
    });
}

pub(super) fn validates_permissioned_domain_status(status: &PermissionedDomainStatus) -> bool {
    const MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE: usize = 10;

    status.credentials_size > 0
        && status.credentials_size <= MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE
        && status.unique
        && status.sorted
}

pub(super) fn validates_permissioned_domain(
    txn_type: protocol::TxType,
    result: Ter,
    fix_cleanup_3_1_3: bool,
    state: &PermissionedDomainState,
) -> bool {
    if fix_cleanup_3_1_3 {
        if !protocol::is_tes_success(result) {
            return state.statuses.is_empty();
        }
        if state.statuses.len() > 1 {
            return false;
        }

        match txn_type {
            protocol::TxType::PERMISSIONED_DOMAIN_SET => {
                let Some(status) = state.statuses.first() else {
                    return false;
                };
                !status.deleted && validates_permissioned_domain_status(status)
            }
            protocol::TxType::PERMISSIONED_DOMAIN_DELETE => {
                state.statuses.first().is_some_and(|status| status.deleted)
            }
            _ => state.statuses.is_empty(),
        }
    } else {
        if txn_type != protocol::TxType::PERMISSIONED_DOMAIN_SET
            || !protocol::is_tes_success(result)
            || state.statuses.is_empty()
        {
            return true;
        }
        validates_permissioned_domain_status(&state.statuses[0])
    }
}
