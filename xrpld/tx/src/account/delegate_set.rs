//! Deterministic the reference implementation shells.
//!
//! This ports the current narrow compatibility-safe behavior for:
//!
//! - the ordered `preflight(...)` permission-array, self-authorize,
//!   duplicate-permission, and delegable-permission checks,
//! - the `preclaim(...)` account, target, and delete-missing-entry checks,
//! - the `doApply()` existing-update, delete, and create branches,
//! - and the shared `deleteDelegate(...)` first-failure remove flow.

use std::collections::BTreeSet;

use protocol::{NotTec, Ter};

pub const PERMISSION_MAX_SIZE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DelegateSetPreclaimFacts {
    pub account_exists: bool,
    pub authorize_exists: bool,
    pub permissions_empty: bool,
    pub delegate_exists: bool,
}

pub trait DelegateSetDeleteSink {
    fn delegate_exists_for_delete(&mut self) -> bool;
    fn dir_remove(&mut self) -> bool;
    fn owner_exists(&mut self) -> bool;
    fn adjust_owner_count(&mut self, delta: i32);
    fn erase_delegate(&mut self);
}

pub trait DelegateSetApplySink<Permission>: DelegateSetDeleteSink {
    type OwnerNode: Copy;

    fn owner_exists_for_apply(&mut self) -> bool;
    fn delegate_exists_for_apply(&mut self) -> bool;
    fn update_existing_permissions(&mut self, permissions: Vec<Permission>);
    fn owner_has_reserve_for_create(&mut self) -> bool;
    fn stage_new_delegate(&mut self, permissions: Vec<Permission>);
    fn dir_insert_new_delegate(&mut self) -> Option<Self::OwnerNode>;
    fn set_new_delegate_owner_node(&mut self, page: Self::OwnerNode);
    fn insert_new_delegate(&mut self);
}

pub fn run_delegate_set_preflight<AccountId: Eq, Permission: Ord>(
    account: &AccountId,
    authorize: &AccountId,
    permissions: &[Permission],
    mut is_delegable: impl FnMut(&Permission) -> bool,
) -> NotTec {
    if permissions.len() > PERMISSION_MAX_SIZE {
        return Ter::TEM_ARRAY_TOO_LARGE;
    }

    if account == authorize {
        return Ter::TEM_MALFORMED;
    }

    let mut permission_set = BTreeSet::new();
    for permission in permissions {
        if !permission_set.insert(permission) {
            return Ter::TEM_MALFORMED;
        }

        if !is_delegable(permission) {
            return Ter::TEM_MALFORMED;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_delegate_set_preclaim(facts: DelegateSetPreclaimFacts) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.authorize_exists {
        return Ter::TEC_NO_TARGET;
    }

    if facts.permissions_empty && !facts.delegate_exists {
        return Ter::TEC_NO_ENTRY;
    }

    Ter::TES_SUCCESS
}

pub fn run_delegate_set_delete_delegate<S: DelegateSetDeleteSink>(sink: &mut S) -> Ter {
    if !sink.delegate_exists_for_delete() {
        return Ter::TEC_INTERNAL;
    }

    if !sink.dir_remove() {
        return Ter::TEF_BAD_LEDGER;
    }

    if !sink.owner_exists() {
        return Ter::TEC_INTERNAL;
    }

    sink.adjust_owner_count(-1);
    sink.erase_delegate();
    Ter::TES_SUCCESS
}

pub fn run_delegate_set_do_apply<Permission: Clone, S: DelegateSetApplySink<Permission>>(
    permissions: &[Permission],
    sink: &mut S,
) -> Ter {
    if !sink.owner_exists_for_apply() {
        return Ter::TEF_INTERNAL;
    }

    if sink.delegate_exists_for_apply() {
        if permissions.is_empty() {
            return run_delegate_set_delete_delegate(sink);
        }

        sink.update_existing_permissions(permissions.to_vec());
        return Ter::TES_SUCCESS;
    }

    if permissions.is_empty() {
        return Ter::TEC_INTERNAL;
    }

    if !sink.owner_has_reserve_for_create() {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    sink.stage_new_delegate(permissions.to_vec());

    let Some(page) = sink.dir_insert_new_delegate() else {
        return Ter::TEC_DIR_FULL;
    };

    sink.set_new_delegate_owner_node(page);
    sink.insert_new_delegate();
    sink.adjust_owner_count(1);
    Ter::TES_SUCCESS
}
