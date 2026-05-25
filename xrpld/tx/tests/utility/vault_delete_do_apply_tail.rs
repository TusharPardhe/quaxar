//! Integration tests that pin the narrowed Rust `VaultDelete.cpp::doApply()`
//! destruction tail to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
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
fn vault_delete_do_apply_tail_rejects_existing_pseudo_dir() {
    let result = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| true,
        |_| Some(base_pseudo()),
        |_| false,
        |_| {},
        |_, _| true,
        |_| Some(()),
        |_| {},
        || {},
    );

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn vault_delete_do_apply_tail_rejects_missing_or_mismatched_pseudo() {
    let missing = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| None::<TestPseudoAccount>,
        |_| false,
        |_| {},
        |_, _| true,
        |_| Some(()),
        |_| {},
        || {},
    );
    assert_eq!(missing, Ter::TEF_BAD_LEDGER);

    let mismatched = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| {
            Some(TestPseudoAccount {
                vault_key: "other-vault",
                ..base_pseudo()
            })
        },
        |_| false,
        |_| {},
        |_, _| true,
        |_| Some(()),
        |_| {},
        || {},
    );
    assert_eq!(mismatched, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(mismatched), "tefBAD_LEDGER");
}

#[test]
fn vault_delete_do_apply_tail_rejects_remaining_pseudo_obligations() {
    let nonzero_balance = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| {
            Some(TestPseudoAccount {
                balance_is_zero: false,
                ..base_pseudo()
            })
        },
        |_| false,
        |_| {},
        |_, _| true,
        |_| Some(()),
        |_| {},
        || {},
    );
    assert_eq!(nonzero_balance, Ter::TEC_HAS_OBLIGATIONS);

    let nonzero_owner_count = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| {
            Some(TestPseudoAccount {
                owner_count_is_zero: false,
                ..base_pseudo()
            })
        },
        |_| false,
        |_| {},
        |_, _| true,
        |_| Some(()),
        |_| {},
        || {},
    );
    assert_eq!(nonzero_owner_count, Ter::TEC_HAS_OBLIGATIONS);

    let remaining_dir = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| Some(base_pseudo()),
        |_| true,
        |_| {},
        |_, _| true,
        |_| Some(()),
        |_| {},
        || {},
    );
    assert_eq!(remaining_dir, Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn vault_delete_do_apply_tail_maps_final_owner_side_failures_to_tefbad_ledger() {
    let dir_remove_failure = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| Some(base_pseudo()),
        |_| false,
        |_| {},
        |_, _| false,
        |_| Some(()),
        |_| {},
        || {},
    );
    assert_eq!(dir_remove_failure, Ter::TEF_BAD_LEDGER);

    let owner_missing = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| Some(base_pseudo()),
        |_| false,
        |_| {},
        |_, _| true,
        |_| None::<()>,
        |_| {},
        || {},
    );
    assert_eq!(owner_missing, Ter::TEF_BAD_LEDGER);
}

#[test]
fn vault_delete_do_apply_tail_runs_success_path() {
    let mut adjusted = false;
    let mut erased_vault = false;

    let result = run_vault_delete_do_apply_tail(
        &test_vault(),
        |_| false,
        |_| Some(base_pseudo()),
        |_| false,
        |_| {},
        |owner_id, vault| {
            assert_eq!(*owner_id, "vault-owner");
            assert_eq!(vault.key(), &"vault-key");
            true
        },
        |owner_id| {
            assert_eq!(*owner_id, "vault-owner");
            Some("owner-account")
        },
        |owner| {
            assert_eq!(owner, "owner-account");
            adjusted = true;
        },
        || erased_vault = true,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(adjusted);
    assert!(erased_vault);
}
