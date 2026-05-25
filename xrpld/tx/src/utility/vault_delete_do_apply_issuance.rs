//! Middle the reference implementation share-issuance destruction shell.
//!
//! This ports the exact current behavior around:
//!
//! - `tefINTERNAL` when the vault-share issuance lookup is unexpectedly
//!   missing,
//! - conditional owner-MPToken empty-holding removal with first-failure return,
//! - `tefBAD_LEDGER` when the pseudo-account directory unlink fails, and
//! - pseudo-account owner-count adjustment plus issuance erase only after that
//!   unlink succeeds.

use protocol::{Ter, is_tes_success};

pub trait VaultDeleteDoApplyIssuanceVault {
    type ShareMptId;

    fn share_mpt_id(&self) -> &Self::ShareMptId;
}

pub fn run_vault_delete_do_apply_issuance<
    Vault,
    Issuance,
    ReadIssuance,
    OwnerHasToken,
    RemoveOwnerHolding,
    DirRemoveIssuance,
    AdjustPseudoOwnerCount,
    EraseIssuance,
>(
    vault: &Vault,
    read_issuance: ReadIssuance,
    owner_has_token: OwnerHasToken,
    remove_owner_holding: RemoveOwnerHolding,
    dir_remove_issuance: DirRemoveIssuance,
    adjust_pseudo_owner_count: AdjustPseudoOwnerCount,
    erase_issuance: EraseIssuance,
) -> Ter
where
    Vault: VaultDeleteDoApplyIssuanceVault,
    Vault::ShareMptId: Clone,
    ReadIssuance: FnOnce(&Vault::ShareMptId) -> Option<Issuance>,
    OwnerHasToken: FnOnce(&Vault::ShareMptId) -> bool,
    RemoveOwnerHolding: FnOnce(&Vault::ShareMptId) -> Ter,
    DirRemoveIssuance: FnOnce(&Issuance) -> bool,
    AdjustPseudoOwnerCount: FnOnce(),
    EraseIssuance: FnOnce(Issuance),
{
    let share_mpt_id = vault.share_mpt_id().clone();
    let issuance = match read_issuance(&share_mpt_id) {
        Some(issuance) => issuance,
        None => return Ter::TEF_INTERNAL,
    };

    if owner_has_token(&share_mpt_id) {
        let ter = remove_owner_holding(&share_mpt_id);
        if !is_tes_success(ter) {
            return ter;
        }
    }

    if !dir_remove_issuance(&issuance) {
        return Ter::TEF_BAD_LEDGER;
    }

    adjust_pseudo_owner_count();
    erase_issuance(issuance);
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{VaultDeleteDoApplyIssuanceVault, run_vault_delete_do_apply_issuance};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        share_mpt_id: &'static str,
    }

    impl VaultDeleteDoApplyIssuanceVault for TestVault {
        type ShareMptId = &'static str;

        fn share_mpt_id(&self) -> &Self::ShareMptId {
            &self.share_mpt_id
        }
    }

    fn test_vault() -> TestVault {
        TestVault {
            share_mpt_id: "share-mpt",
        }
    }

    #[test]
    fn vault_delete_do_apply_issuance_returns_tefinternal_when_issuance_is_missing() {
        let touched = Cell::new(false);

        let result = run_vault_delete_do_apply_issuance(
            &test_vault(),
            |_| None::<&'static str>,
            |_| {
                touched.set(true);
                false
            },
            |_| {
                touched.set(true);
                Ter::TES_SUCCESS
            },
            |_| {
                touched.set(true);
                true
            },
            || touched.set(true),
            |_| touched.set(true),
        );

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(trans_token(result), "tefINTERNAL");
        assert!(!touched.get());
    }

    #[test]
    fn vault_delete_do_apply_issuance_returns_owner_holding_failure_unchanged() {
        let dir_removed = Cell::new(false);

        let result = run_vault_delete_do_apply_issuance(
            &test_vault(),
            |share_mpt_id| {
                assert_eq!(*share_mpt_id, "share-mpt");
                Some("issuance")
            },
            |_| true,
            |share_mpt_id| {
                assert_eq!(*share_mpt_id, "share-mpt");
                Ter::TEC_INTERNAL
            },
            |_| {
                dir_removed.set(true);
                true
            },
            || unreachable!("failed owner-holding removal should skip owner-count adjustment"),
            |_| unreachable!("failed owner-holding removal should skip issuance erase"),
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
        assert!(!dir_removed.get());
    }

    #[test]
    fn vault_delete_do_apply_issuance_maps_dir_remove_failure_to_tefbad_ledger() {
        let adjusted = Cell::new(false);

        let result = run_vault_delete_do_apply_issuance(
            &test_vault(),
            |_| Some("issuance"),
            |_| false,
            |_| unreachable!("missing owner token should skip owner holding removal"),
            |_| false,
            || adjusted.set(true),
            |_| unreachable!("failed dirRemove should skip issuance erase"),
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(result), "tefBAD_LEDGER");
        assert!(!adjusted.get());
    }

    #[test]
    fn vault_delete_do_apply_issuance_skips_owner_holding_removal_when_owner_has_no_token() {
        let removed_owner_holding = Cell::new(false);
        let erased = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_delete_do_apply_issuance(
            &test_vault(),
            |_| Some("issuance"),
            |_| false,
            |_| {
                removed_owner_holding.set(true);
                Ter::TES_SUCCESS
            },
            |_| true,
            || erased.borrow_mut().push("adjust"),
            {
                let erased = Rc::clone(&erased);
                move |_| erased.borrow_mut().push("erase")
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert!(!removed_owner_holding.get());
        assert_eq!(erased.borrow().as_slice(), ["adjust", "erase"]);
    }

    #[test]
    fn vault_delete_do_apply_issuance_runs_success_path_in() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_delete_do_apply_issuance(
            &test_vault(),
            |share_mpt_id| {
                assert_eq!(*share_mpt_id, "share-mpt");
                Some("issuance")
            },
            {
                let events = Rc::clone(&events);
                move |share_mpt_id| {
                    events.borrow_mut().push("has_token");
                    assert_eq!(*share_mpt_id, "share-mpt");
                    true
                }
            },
            {
                let events = Rc::clone(&events);
                move |share_mpt_id| {
                    events.borrow_mut().push("remove");
                    assert_eq!(*share_mpt_id, "share-mpt");
                    Ter::TES_SUCCESS
                }
            },
            {
                let events = Rc::clone(&events);
                move |issuance| {
                    events.borrow_mut().push("dir_remove");
                    assert_eq!(*issuance, "issuance");
                    true
                }
            },
            {
                let events = Rc::clone(&events);
                move || events.borrow_mut().push("adjust")
            },
            {
                let events = Rc::clone(&events);
                move |issuance| {
                    events.borrow_mut().push("erase");
                    assert_eq!(issuance, "issuance");
                }
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            events.borrow().as_slice(),
            ["has_token", "remove", "dir_remove", "adjust", "erase"]
        );
    }
}
