use std::collections::BTreeSet;

use protocol::{NotTec, Ter};

pub const PERMISSION_MAX_SIZE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DelegateSetPreclaimFacts {
    pub account_exists: bool,
    pub authorize_exists: bool,
    pub authorize_is_pseudo_account: bool,
    pub permissions_empty: bool,
    pub delegate_exists: bool,
}

pub trait DelegateSetDeleteSink {
    fn delegate_exists_for_delete(&mut self) -> bool;
    fn dir_remove_owner(&mut self) -> bool;
    fn dir_remove_destination(&mut self) -> Option<bool>;
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
    fn dir_insert_owner(&mut self) -> Option<Self::OwnerNode>;
    fn set_owner_node(&mut self, page: Self::OwnerNode);
    fn dir_insert_destination(&mut self) -> Option<Self::OwnerNode>;
    fn set_destination_node(&mut self, page: Self::OwnerNode);
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

    if facts.authorize_is_pseudo_account {
        return Ter::TEC_NO_PERMISSION;
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

    if !sink.dir_remove_owner() {
        return Ter::TEF_BAD_LEDGER;
    }

    if let Some(false) = sink.dir_remove_destination() {
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

    let Some(owner_page) = sink.dir_insert_owner() else {
        return Ter::TEC_DIR_FULL;
    };
    sink.set_owner_node(owner_page);

    let Some(dest_page) = sink.dir_insert_destination() else {
        return Ter::TEC_DIR_FULL;
    };
    sink.set_destination_node(dest_page);

    sink.insert_new_delegate();
    sink.adjust_owner_count(1);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_rejects_self_authorize() {
        let result = run_delegate_set_preflight(&1u32, &1u32, &[42u32], |_| true);
        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn preflight_rejects_too_many_permissions() {
        let permissions: Vec<u32> = (0..11).collect();
        let result = run_delegate_set_preflight(&1u32, &2u32, &permissions, |_| true);
        assert_eq!(result, Ter::TEM_ARRAY_TOO_LARGE);
    }

    #[test]
    fn preflight_rejects_duplicate_permissions() {
        let result = run_delegate_set_preflight(&1u32, &2u32, &[5u32, 5u32], |_| true);
        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn preflight_rejects_non_delegable_permission() {
        let result = run_delegate_set_preflight(&1u32, &2u32, &[5u32], |_| false);
        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn preflight_accepts_valid_input() {
        let result = run_delegate_set_preflight(&1u32, &2u32, &[5u32, 6u32], |_| true);
        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn preclaim_rejects_missing_account() {
        let facts = DelegateSetPreclaimFacts {
            account_exists: false,
            ..Default::default()
        };
        assert_eq!(run_delegate_set_preclaim(facts), Ter::TER_NO_ACCOUNT);
    }

    #[test]
    fn preclaim_rejects_missing_target() {
        let facts = DelegateSetPreclaimFacts {
            account_exists: true,
            authorize_exists: false,
            ..Default::default()
        };
        assert_eq!(run_delegate_set_preclaim(facts), Ter::TEC_NO_TARGET);
    }

    #[test]
    fn preclaim_rejects_pseudo_account_target() {
        let facts = DelegateSetPreclaimFacts {
            account_exists: true,
            authorize_exists: true,
            authorize_is_pseudo_account: true,
            ..Default::default()
        };
        assert_eq!(run_delegate_set_preclaim(facts), Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn preclaim_rejects_delete_of_nonexistent_delegate() {
        let facts = DelegateSetPreclaimFacts {
            account_exists: true,
            authorize_exists: true,
            authorize_is_pseudo_account: false,
            permissions_empty: true,
            delegate_exists: false,
        };
        assert_eq!(run_delegate_set_preclaim(facts), Ter::TEC_NO_ENTRY);
    }

    #[test]
    fn preclaim_accepts_valid_setup() {
        let facts = DelegateSetPreclaimFacts {
            account_exists: true,
            authorize_exists: true,
            authorize_is_pseudo_account: false,
            permissions_empty: false,
            delegate_exists: false,
        };
        assert_eq!(run_delegate_set_preclaim(facts), Ter::TES_SUCCESS);
    }
}
