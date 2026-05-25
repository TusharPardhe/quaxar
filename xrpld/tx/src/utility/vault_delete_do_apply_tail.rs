//! Tail the reference implementation destruction shell.
//!
//! This ports the exact current behavior around:
//!
//! - the immediate pseudo-account owner-dir presence check,
//! - pseudo-account lookup and vault-link sanity,
//! - remaining pseudo-account obligation guards,
//! - pseudo-account erase before owner-dir unlink of the vault,
//! - `tefBAD_LEDGER` on failed vault dir removal or missing owner account, and
//! - final owner-count decrement plus vault erase before `tesSUCCESS`.

use protocol::Ter;

pub trait VaultDeleteDoApplyTailVault {
    type AccountId;
    type VaultKey;

    fn pseudo_id(&self) -> &Self::AccountId;
    fn owner_id(&self) -> &Self::AccountId;
    fn key(&self) -> &Self::VaultKey;
}

pub trait VaultDeleteDoApplyTailPseudoAccount<VaultKey> {
    fn belongs_to_vault(&self, vault_key: &VaultKey) -> bool;
    fn balance_is_zero(&self) -> bool;
    fn owner_count_is_zero(&self) -> bool;
}

pub fn run_vault_delete_do_apply_tail<
    Vault,
    PseudoAccount,
    OwnerAccount,
    PeekPseudoDir,
    ReadPseudoAccount,
    PseudoDirExistsAfter,
    ErasePseudoAccount,
    RemoveVaultFromOwnerDir,
    ReadOwnerAccount,
    AdjustOwnerCount,
    EraseVault,
>(
    vault: &Vault,
    peek_pseudo_dir: PeekPseudoDir,
    read_pseudo_account: ReadPseudoAccount,
    pseudo_dir_exists_after: PseudoDirExistsAfter,
    erase_pseudo_account: ErasePseudoAccount,
    remove_vault_from_owner_dir: RemoveVaultFromOwnerDir,
    read_owner_account: ReadOwnerAccount,
    adjust_owner_count: AdjustOwnerCount,
    erase_vault: EraseVault,
) -> Ter
where
    Vault: VaultDeleteDoApplyTailVault,
    PseudoAccount: VaultDeleteDoApplyTailPseudoAccount<Vault::VaultKey>,
    PeekPseudoDir: FnOnce(&Vault::AccountId) -> bool,
    ReadPseudoAccount: FnOnce(&Vault::AccountId) -> Option<PseudoAccount>,
    PseudoDirExistsAfter: FnOnce(&Vault::AccountId) -> bool,
    ErasePseudoAccount: FnOnce(PseudoAccount),
    RemoveVaultFromOwnerDir: FnOnce(&Vault::AccountId, &Vault) -> bool,
    ReadOwnerAccount: FnOnce(&Vault::AccountId) -> Option<OwnerAccount>,
    AdjustOwnerCount: FnOnce(OwnerAccount),
    EraseVault: FnOnce(),
{
    let pseudo_id = vault.pseudo_id();
    if peek_pseudo_dir(pseudo_id) {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    let pseudo_account = match read_pseudo_account(pseudo_id) {
        Some(pseudo_account) => pseudo_account,
        None => return Ter::TEF_BAD_LEDGER,
    };

    if !pseudo_account.belongs_to_vault(vault.key()) {
        return Ter::TEF_BAD_LEDGER;
    }

    if !pseudo_account.balance_is_zero() {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if !pseudo_account.owner_count_is_zero() {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if pseudo_dir_exists_after(pseudo_id) {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    erase_pseudo_account(pseudo_account);

    let owner_id = vault.owner_id();
    if !remove_vault_from_owner_dir(owner_id, vault) {
        return Ter::TEF_BAD_LEDGER;
    }

    let owner_account = match read_owner_account(owner_id) {
        Some(owner_account) => owner_account,
        None => return Ter::TEF_BAD_LEDGER,
    };

    adjust_owner_count(owner_account);
    erase_vault();
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        VaultDeleteDoApplyTailPseudoAccount, VaultDeleteDoApplyTailVault,
        run_vault_delete_do_apply_tail,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_id: &'static str,
        owner_id: &'static str,
        key: &'static str,
    }

    impl VaultDeleteDoApplyTailVault for TestVault {
        type AccountId = &'static str;
        type VaultKey = &'static str;

        fn pseudo_id(&self) -> &Self::AccountId {
            &self.pseudo_id
        }

        fn owner_id(&self) -> &Self::AccountId {
            &self.owner_id
        }

        fn key(&self) -> &Self::VaultKey {
            &self.key
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestPseudoAccount {
        vault_key: &'static str,
        balance_is_zero: bool,
        owner_count_is_zero: bool,
    }

    impl VaultDeleteDoApplyTailPseudoAccount<&'static str> for TestPseudoAccount {
        fn belongs_to_vault(&self, vault_key: &&'static str) -> bool {
            self.vault_key == *vault_key
        }

        fn balance_is_zero(&self) -> bool {
            self.balance_is_zero
        }

        fn owner_count_is_zero(&self) -> bool {
            self.owner_count_is_zero
        }
    }

    fn test_vault() -> TestVault {
        TestVault {
            pseudo_id: "vault-pseudo",
            owner_id: "vault-owner",
            key: "vault-key",
        }
    }

    fn base_pseudo() -> TestPseudoAccount {
        TestPseudoAccount {
            vault_key: "vault-key",
            balance_is_zero: true,
            owner_count_is_zero: true,
        }
    }

    #[test]
    fn vault_delete_do_apply_tail_rejects_existing_pseudo_dir_before_reload() {
        let touched = Cell::new(false);

        let result = run_vault_delete_do_apply_tail(
            &test_vault(),
            |_| true,
            |_| {
                touched.set(true);
                Some(base_pseudo())
            },
            |_| {
                touched.set(true);
                false
            },
            |_| touched.set(true),
            |_, _| {
                touched.set(true);
                true
            },
            |_| {
                touched.set(true);
                Some(())
            },
            |_| touched.set(true),
            || touched.set(true),
        );

        assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
        assert!(!touched.get());
    }

    #[test]
    fn vault_delete_do_apply_tail_requires_matching_loaded_pseudo_account() {
        let result = run_vault_delete_do_apply_tail(
            &test_vault(),
            |_| false,
            |_| {
                Some(TestPseudoAccount {
                    vault_key: "other-vault",
                    ..base_pseudo()
                })
            },
            |_| false,
            |_| unreachable!("mismatched pseudo should skip erase"),
            |_, _| unreachable!("mismatched pseudo should skip owner dir removal"),
            |_| unreachable!("mismatched pseudo should skip owner load"),
            |_: ()| unreachable!("mismatched pseudo should skip owner adjustment"),
            || unreachable!("mismatched pseudo should skip vault erase"),
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(result), "tefBAD_LEDGER");
    }

    #[test]
    fn vault_delete_do_apply_tail_rejects_remaining_pseudo_obligations() {
        let balance_result = run_vault_delete_do_apply_tail(
            &test_vault(),
            |_| false,
            |_| {
                Some(TestPseudoAccount {
                    balance_is_zero: false,
                    ..base_pseudo()
                })
            },
            |_| false,
            |_| unreachable!("nonzero balance should skip erase"),
            |_, _| unreachable!("nonzero balance should skip owner dir removal"),
            |_| unreachable!("nonzero balance should skip owner load"),
            |_: ()| unreachable!("nonzero balance should skip owner adjustment"),
            || unreachable!("nonzero balance should skip vault erase"),
        );
        assert_eq!(balance_result, Ter::TEC_HAS_OBLIGATIONS);

        let owner_count_result = run_vault_delete_do_apply_tail(
            &test_vault(),
            |_| false,
            |_| {
                Some(TestPseudoAccount {
                    owner_count_is_zero: false,
                    ..base_pseudo()
                })
            },
            |_| false,
            |_| unreachable!("nonzero owner count should skip erase"),
            |_, _| unreachable!("nonzero owner count should skip owner dir removal"),
            |_| unreachable!("nonzero owner count should skip owner load"),
            |_: ()| unreachable!("nonzero owner count should skip owner adjustment"),
            || unreachable!("nonzero owner count should skip vault erase"),
        );
        assert_eq!(owner_count_result, Ter::TEC_HAS_OBLIGATIONS);

        let dir_result = run_vault_delete_do_apply_tail(
            &test_vault(),
            |_| false,
            |_| Some(base_pseudo()),
            |_| true,
            |_| unreachable!("existing pseudo dir should skip erase"),
            |_, _| unreachable!("existing pseudo dir should skip owner dir removal"),
            |_| unreachable!("existing pseudo dir should skip owner load"),
            |_: ()| unreachable!("existing pseudo dir should skip owner adjustment"),
            || unreachable!("existing pseudo dir should skip vault erase"),
        );
        assert_eq!(dir_result, Ter::TEC_HAS_OBLIGATIONS);
    }

    #[test]
    fn vault_delete_do_apply_tail_maps_owner_dir_or_owner_lookup_failure_to_tefbad_ledger() {
        let dir_remove_result = run_vault_delete_do_apply_tail(
            &test_vault(),
            |_| false,
            |_| Some(base_pseudo()),
            |_| false,
            |_| {},
            |owner_id, vault| {
                assert_eq!(*owner_id, "vault-owner");
                assert_eq!(vault.key(), &"vault-key");
                false
            },
            |_| unreachable!("failed owner dir removal should skip owner load"),
            |_: ()| unreachable!("failed owner dir removal should skip owner adjustment"),
            || unreachable!("failed owner dir removal should skip vault erase"),
        );
        assert_eq!(dir_remove_result, Ter::TEF_BAD_LEDGER);

        let owner_missing_result = run_vault_delete_do_apply_tail(
            &test_vault(),
            |_| false,
            |_| Some(base_pseudo()),
            |_| false,
            |_| {},
            |_, _| true,
            |_| None::<()>,
            |_: ()| unreachable!("missing owner should skip owner adjustment"),
            || unreachable!("missing owner should skip vault erase"),
        );
        assert_eq!(owner_missing_result, Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(owner_missing_result), "tefBAD_LEDGER");
    }

    #[test]
    fn vault_delete_do_apply_tail_runs_success_path_in() {
        let events = Rc::new(std::cell::RefCell::new(Vec::new()));

        let result = run_vault_delete_do_apply_tail(
            &test_vault(),
            {
                let events = Rc::clone(&events);
                move |pseudo_id| {
                    events.borrow_mut().push("peek_pseudo_dir");
                    assert_eq!(*pseudo_id, "vault-pseudo");
                    false
                }
            },
            {
                let events = Rc::clone(&events);
                move |pseudo_id| {
                    events.borrow_mut().push("read_pseudo");
                    assert_eq!(*pseudo_id, "vault-pseudo");
                    Some(base_pseudo())
                }
            },
            {
                let events = Rc::clone(&events);
                move |pseudo_id| {
                    events.borrow_mut().push("exists_pseudo_dir");
                    assert_eq!(*pseudo_id, "vault-pseudo");
                    false
                }
            },
            {
                let events = Rc::clone(&events);
                move |_| events.borrow_mut().push("erase_pseudo")
            },
            {
                let events = Rc::clone(&events);
                move |owner_id, vault| {
                    events.borrow_mut().push("remove_vault_dir");
                    assert_eq!(*owner_id, "vault-owner");
                    assert_eq!(vault.key(), &"vault-key");
                    true
                }
            },
            {
                let events = Rc::clone(&events);
                move |owner_id| {
                    events.borrow_mut().push("read_owner");
                    assert_eq!(*owner_id, "vault-owner");
                    Some("owner-account")
                }
            },
            {
                let events = Rc::clone(&events);
                move |owner| {
                    events.borrow_mut().push("adjust_owner");
                    assert_eq!(owner, "owner-account");
                }
            },
            {
                let events = Rc::clone(&events);
                move || events.borrow_mut().push("erase_vault")
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            events.borrow().as_slice(),
            [
                "peek_pseudo_dir",
                "read_pseudo",
                "exists_pseudo_dir",
                "erase_pseudo",
                "remove_vault_dir",
                "read_owner",
                "adjust_owner",
                "erase_vault"
            ]
        );
    }
}
