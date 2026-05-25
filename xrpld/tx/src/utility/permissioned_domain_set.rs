//! Deterministic the reference implementation shells.
//!
//! This ports the current narrow compatibility-safe behavior for:
//!
//! - the credentials amendment gate from `checkExtraFeatures(...)`,
//! - the ordered `preflight(...)` credentials-array then `sfDomainID` checks,
//! - the `preclaim(...)` account, issuer, and owner checks,
//! - and the `doApply()` update-versus-create flow including sorted credential
//!   storage, reserve, owner-dir, and owner-count ordering.

use protocol::{NotTec, Ter};

pub const MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PermissionedDomainCredential<AccountId, CredentialType> {
    pub issuer: AccountId,
    pub credential_type: CredentialType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PermissionedDomainSetPreclaimFacts {
    pub account_exists: bool,
    pub domain_id_present: bool,
    pub domain_exists: bool,
    pub domain_owned_by_account: bool,
}

pub trait PermissionedDomainSetApplySink<Credential> {
    type OwnerNode: Copy;

    fn owner_exists(&mut self) -> bool;
    fn existing_domain_exists(&mut self) -> bool;
    fn replace_existing_domain_credentials(&mut self, credentials: Vec<Credential>);
    fn owner_has_reserve_for_new_domain(&mut self) -> bool;
    fn stage_new_domain(&mut self, credentials: Vec<Credential>);
    fn dir_insert_new_domain(&mut self) -> Option<Self::OwnerNode>;
    fn set_new_domain_owner_node(&mut self, page: Self::OwnerNode);
    fn adjust_owner_count(&mut self, delta: i32);
    fn insert_new_domain(&mut self);
}

pub fn permissioned_domain_set_check_extra_features(feature_credentials_enabled: bool) -> bool {
    feature_credentials_enabled
}

pub fn sort_permissioned_domain_credentials<Credential: Ord>(
    mut credentials: Vec<Credential>,
) -> Vec<Credential> {
    credentials.sort();
    credentials
}

pub fn run_permissioned_domain_set_preflight(
    domain_id_present: bool,
    domain_id_is_zero: bool,
    check_credentials_array: impl FnOnce() -> NotTec,
) -> NotTec {
    let err = check_credentials_array();
    if err != Ter::TES_SUCCESS {
        return err;
    }

    if domain_id_present && domain_id_is_zero {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_permissioned_domain_set_preclaim(
    facts: PermissionedDomainSetPreclaimFacts,
    credential_issuer_exists: impl IntoIterator<Item = bool>,
) -> Ter {
    if !facts.account_exists {
        return Ter::TEF_INTERNAL;
    }

    for issuer_exists in credential_issuer_exists {
        if !issuer_exists {
            return Ter::TEC_NO_ISSUER;
        }
    }

    if facts.domain_id_present {
        if !facts.domain_exists {
            return Ter::TEC_NO_ENTRY;
        }

        if !facts.domain_owned_by_account {
            return Ter::TEC_NO_PERMISSION;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_permissioned_domain_set_do_apply<
    Credential: Ord,
    S: PermissionedDomainSetApplySink<Credential>,
>(
    tx_credentials: Vec<Credential>,
    modifying_existing_domain: bool,
    sink: &mut S,
) -> Ter {
    if !sink.owner_exists() {
        return Ter::TEF_INTERNAL;
    }

    let sorted = sort_permissioned_domain_credentials(tx_credentials);

    if modifying_existing_domain {
        if !sink.existing_domain_exists() {
            return Ter::TEF_INTERNAL;
        }

        sink.replace_existing_domain_credentials(sorted);
        return Ter::TES_SUCCESS;
    }

    if !sink.owner_has_reserve_for_new_domain() {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    sink.stage_new_domain(sorted);

    let Some(page) = sink.dir_insert_new_domain() else {
        return Ter::TEC_DIR_FULL;
    };

    sink.set_new_domain_owner_node(page);
    sink.adjust_owner_count(1);
    sink.insert_new_domain();
    Ter::TES_SUCCESS
}
