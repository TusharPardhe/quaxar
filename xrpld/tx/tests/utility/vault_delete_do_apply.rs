//! Integration tests that pin the higher narrowed Rust
//! `VaultDelete.cpp::doApply()` composition shell to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultDeleteDoApplyFrontVault, VaultDeleteDoApplyIssuanceVault,
    VaultDeleteDoApplyTailPseudoAccount, VaultDeleteDoApplyTailVault, run_vault_delete_do_apply,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    pseudo_id: &'static str,
    asset: &'static str,
    share_mpt_id: &'static str,
    owner_id: &'static str,
    key: &'static str,
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

impl VaultDeleteDoApplyIssuanceVault for TestVault {
    type ShareMptId = &'static str;

    fn share_mpt_id(&self) -> &Self::ShareMptId {
        &self.share_mpt_id
    }
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
struct TestPseudo {
    vault_key: &'static str,
    balance_is_zero: bool,
    owner_count_is_zero: bool,
}

impl VaultDeleteDoApplyTailPseudoAccount<&'static str> for TestPseudo {
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
        asset: "USD",
        share_mpt_id: "share-mpt",
        owner_id: "vault-owner",
        key: "vault-key",
    }
}

fn tail_pseudo() -> TestPseudo {
    TestPseudo {
        vault_key: "vault-key",
        balance_is_zero: true,
        owner_count_is_zero: true,
    }
}

#[test]
fn vault_delete_do_apply_runs_current_cpp_stage_order() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_delete_do_apply(
        || Some(test_vault()),
        {
            let steps = Rc::clone(&steps);
            move |pseudo_id, asset| {
                steps
                    .borrow_mut()
                    .push(format!("front_remove:{pseudo_id}:{asset}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |pseudo_id| {
                steps.borrow_mut().push(format!("front_pseudo:{pseudo_id}"));
                Some("front-pseudo")
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |share_mpt_id| {
                steps
                    .borrow_mut()
                    .push(format!("issuance_read:{share_mpt_id}"));
                Some("issuance")
            }
        },
        |_| true,
        {
            let steps = Rc::clone(&steps);
            move |share_mpt_id| {
                steps
                    .borrow_mut()
                    .push(format!("owner_remove:{share_mpt_id}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |issuance| {
                steps
                    .borrow_mut()
                    .push(format!("issuance_dir_remove:{issuance}"));
                true
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || steps.borrow_mut().push("adjust_pseudo_owner".to_string())
        },
        {
            let steps = Rc::clone(&steps);
            move |issuance| {
                steps
                    .borrow_mut()
                    .push(format!("erase_issuance:{issuance}"))
            }
        },
        |_| false,
        |_| Some(tail_pseudo()),
        |_| false,
        {
            let steps = Rc::clone(&steps);
            move |_| steps.borrow_mut().push("erase_pseudo".to_string())
        },
        {
            let steps = Rc::clone(&steps);
            move |owner_id, vault: &TestVault| {
                steps
                    .borrow_mut()
                    .push(format!("remove_vault_dir:{owner_id}:{}", vault.key()));
                true
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |owner_id| {
                steps.borrow_mut().push(format!("read_owner:{owner_id}"));
                Some("owner")
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |owner| steps.borrow_mut().push(format!("adjust_owner:{owner}"))
        },
        {
            let steps = Rc::clone(&steps);
            move || steps.borrow_mut().push("erase_vault".to_string())
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "front_remove:vault-pseudo:USD",
            "front_pseudo:vault-pseudo",
            "issuance_read:share-mpt",
            "owner_remove:share-mpt",
            "issuance_dir_remove:issuance",
            "adjust_pseudo_owner",
            "erase_issuance:issuance",
            "erase_pseudo",
            "remove_vault_dir:vault-owner:vault-key",
            "read_owner:vault-owner",
            "adjust_owner:owner",
            "erase_vault",
        ]
    );
}

#[test]
fn vault_delete_do_apply_returns_front_failure_before_later_stages() {
    let issuance_called = Cell::new(false);

    let result = run_vault_delete_do_apply(
        || Some(test_vault()),
        |_, _| Ter::TEC_INTERNAL,
        |_| Some("front-pseudo"),
        |_| {
            issuance_called.set(true);
            Some("issuance")
        },
        |_| false,
        |_| Ter::TES_SUCCESS,
        |_| true,
        || {},
        |_| {},
        |_| false,
        |_| Some(tail_pseudo()),
        |_| false,
        |_| {},
        |_, _| true,
        |_| Some("owner"),
        |_| {},
        || {},
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
    assert!(!issuance_called.get());
}

#[test]
fn vault_delete_do_apply_returns_issuance_failure_before_tail() {
    let tail_called = Cell::new(false);

    let result = run_vault_delete_do_apply(
        || Some(test_vault()),
        |_, _| Ter::TES_SUCCESS,
        |_| Some("front-pseudo"),
        |_| Some("issuance"),
        |_| true,
        |_| Ter::TEC_INTERNAL,
        |_| true,
        || {},
        |_| {},
        |_| {
            tail_called.set(true);
            false
        },
        |_| Some(tail_pseudo()),
        |_| false,
        |_| {},
        |_, _| true,
        |_| Some("owner"),
        |_| {},
        || {},
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
    assert!(!tail_called.get());
}
