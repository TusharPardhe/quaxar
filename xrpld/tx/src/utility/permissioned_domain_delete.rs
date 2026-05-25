//! Deterministic the reference implementation shells.
//!
//! This ports the current narrow compatibility-safe behavior for:
//!
//! - the `preflight(...)` zero-domain check,
//! - the `preclaim(...)` existence and owner checks,
//! - and the loaded delete ordering across owner-dir removal, owner-count
//!   adjustment, and ledger erase.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PermissionedDomainDeletePreclaimFacts {
    pub domain_exists: bool,
    pub tx_account_matches_owner: bool,
}

pub trait PermissionedDomainDeleteLoadedSink {
    fn dir_remove(&mut self) -> bool;
    fn owner_exists_with_nonzero_count(&mut self) -> bool;
    fn adjust_owner_count(&mut self, delta: i32);
    fn erase_domain(&mut self);
}

pub trait PermissionedDomainDeleteApplySink {
    fn loaded_domain_exists(&mut self) -> bool;
    fn delete_loaded_domain(&mut self) -> Ter;
}

pub fn run_permissioned_domain_delete_preflight(domain_id_is_zero: bool) -> NotTec {
    if domain_id_is_zero {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_permissioned_domain_delete_preclaim(
    facts: PermissionedDomainDeletePreclaimFacts,
) -> Ter {
    if !facts.domain_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.tx_account_matches_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    Ter::TES_SUCCESS
}

pub fn run_permissioned_domain_delete_loaded<S: PermissionedDomainDeleteLoadedSink>(
    sink: &mut S,
) -> Ter {
    if !sink.dir_remove() {
        return Ter::TEF_BAD_LEDGER;
    }

    assert!(
        sink.owner_exists_with_nonzero_count(),
        "PermissionedDomainDelete::doApply expects owner and nonzero owner count"
    );

    sink.adjust_owner_count(-1);
    sink.erase_domain();
    Ter::TES_SUCCESS
}

pub fn run_permissioned_domain_delete_do_apply<S: PermissionedDomainDeleteApplySink>(
    sink: &mut S,
) -> Ter {
    if !sink.loaded_domain_exists() {
        return Ter::TEF_INTERNAL;
    }

    sink.delete_loaded_domain()
}
