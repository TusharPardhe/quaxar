//! Integration tests that pin the narrowed Rust `VaultDelete.cpp::doApply()`
//! front shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{VaultDeleteDoApplyFront, VaultDeleteDoApplyFrontVault, load_vault_delete_do_apply_front};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    pseudo_id: &'static str,
    asset: &'static str,
}

impl VaultDeleteDoApplyFrontVault for TestVault {
    type AccountId = &'static str;
    type Asset = &'static str;

    fn pseudo_id(&self) -> &Self::AccountId {
        &self.pseudo_id
    }

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

fn test_vault() -> TestVault {
    TestVault {
        pseudo_id: "vault-pseudo",
        asset: "USD",
    }
}

#[test]
fn vault_delete_do_apply_front_returns_tefinternal_when_vault_is_missing() {
    let result = load_vault_delete_do_apply_front(
        || None::<TestVault>,
        |_, _| Ter::TES_SUCCESS,
        |_| Some("pseudo-account"),
    );

    assert_eq!(result, Err(Ter::TEF_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
}

#[test]
fn vault_delete_do_apply_front_returns_remove_empty_holding_failure_unchanged() {
    let result = load_vault_delete_do_apply_front(
        || Some(test_vault()),
        |pseudo_id, asset| {
            assert_eq!(*pseudo_id, "vault-pseudo");
            assert_eq!(*asset, "USD");
            Ter::TEC_INTERNAL
        },
        |_| Some("pseudo-account"),
    );

    assert_eq!(result, Err(Ter::TEC_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tecINTERNAL");
}

#[test]
fn vault_delete_do_apply_front_returns_tefbad_ledger_when_pseudo_account_is_missing() {
    let result = load_vault_delete_do_apply_front(
        || Some(test_vault()),
        |_, _| Ter::TES_SUCCESS,
        |_| None::<&'static str>,
    );

    assert_eq!(result, Err(Ter::TEF_BAD_LEDGER));
    assert_eq!(trans_token(result.unwrap_err()), "tefBAD_LEDGER");
}

#[test]
fn vault_delete_do_apply_front_loads_vault_then_returns_loaded_front_state() {
    let result = load_vault_delete_do_apply_front(
        || Some(test_vault()),
        |pseudo_id, asset| {
            assert_eq!(*pseudo_id, "vault-pseudo");
            assert_eq!(*asset, "USD");
            Ter::TES_SUCCESS
        },
        |pseudo_id| {
            assert_eq!(*pseudo_id, "vault-pseudo");
            Some("pseudo-account")
        },
    );

    assert_eq!(
        result,
        Ok(VaultDeleteDoApplyFront {
            vault: test_vault(),
            pseudo_id: "vault-pseudo",
            asset: "USD",
            pseudo_account: "pseudo-account",
        })
    );
}
