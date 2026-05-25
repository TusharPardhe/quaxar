//! Integration tests that pin the narrowed Rust
//! `VaultClawback.cpp::doApply()` tail shell to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultClawbackAmountVault, VaultClawbackDoApplyFrontState, run_vault_clawback_do_apply_tail,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    asset: &'static str,
    owner: &'static str,
    share_mpt_id: &'static str,
}

impl VaultClawbackAmountVault for TestVault {
    type Asset = &'static str;
    type AccountId = &'static str;
    type ShareId = &'static str;

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }

    fn owner(&self) -> &Self::AccountId {
        &self.owner
    }

    fn share_mpt_id(&self) -> &Self::ShareId {
        &self.share_mpt_id
    }
}

fn test_state(
    holder: &'static str,
    assets_recovered: i64,
) -> VaultClawbackDoApplyFrontState<TestVault, &'static str, &'static str, &'static str, i64, i64> {
    VaultClawbackDoApplyFrontState {
        vault: TestVault {
            asset: "USD",
            owner: "vault-owner",
            share_mpt_id: "share-id",
        },
        vault_asset: "USD",
        vault_account: "vault-account",
        share_id: "share-id",
        holder,
        assets_recovered,
        shares_destroyed: 9,
    }
}

#[test]
fn vault_clawback_do_apply_tail_returns_share_transfer_failure_unchanged() {
    let touched = Cell::new(false);

    let result = run_vault_clawback_do_apply_tail(
        &"issuer",
        &test_state("holder", 4),
        |_, _, _| Ter::TER_NO_ACCOUNT,
        |_, _| {
            touched.set(true);
            Ter::TES_SUCCESS
        },
        |_| {
            touched.set(true);
            true
        },
        |_, _, _| {
            touched.set(true);
            Ter::TES_SUCCESS
        },
        |_, _| {
            touched.set(true);
            false
        },
        |_, _| touched.set(true),
    );

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
    assert!(!touched.get());
}

#[test]
fn vault_clawback_do_apply_tail_returns_cleanup_failure_unchanged() {
    let touched = Cell::new(false);

    let result = run_vault_clawback_do_apply_tail(
        &"issuer",
        &test_state("holder", 0),
        |_, _, _| Ter::TES_SUCCESS,
        |_, _| Ter::TEC_NO_PERMISSION,
        |_| {
            touched.set(true);
            false
        },
        |_, _, _| {
            touched.set(true);
            Ter::TES_SUCCESS
        },
        |_, _| {
            touched.set(true);
            false
        },
        |_, _| touched.set(true),
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
    assert!(!touched.get());
}

#[test]
fn vault_clawback_do_apply_tail_ignores_techas_obligations_and_zero_recovery() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_clawback_do_apply_tail(
        &"issuer",
        &test_state("holder", 0),
        {
            let steps = Rc::clone(&steps);
            move |holder, vault_account, shares_destroyed| {
                steps.borrow_mut().push(format!(
                    "send_shares:{holder}:{vault_account}:{shares_destroyed}"
                ));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |holder, shares_destroyed| {
                steps
                    .borrow_mut()
                    .push(format!("remove_empty:{holder}:{shares_destroyed}"));
                Ter::TEC_HAS_OBLIGATIONS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |assets_recovered| {
                steps
                    .borrow_mut()
                    .push(format!("is_positive:{assets_recovered}"));
                false
            }
        },
        |_, _, _| unreachable!("zero recovery should skip asset transfer"),
        |_, _| unreachable!("zero recovery should skip balance check"),
        {
            let steps = Rc::clone(&steps);
            move |vault, asset| {
                steps
                    .borrow_mut()
                    .push(format!("associate:{}:{asset}", vault.owner()));
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "send_shares:holder:vault-account:9",
            "remove_empty:holder:9",
            "is_positive:0",
            "associate:vault-owner:USD",
        ]
    );
}

#[test]
fn vault_clawback_do_apply_tail_returns_tefinternal_on_negative_vault_balance() {
    let associated = Cell::new(false);

    let result = run_vault_clawback_do_apply_tail(
        &"issuer",
        &test_state("holder", 4),
        |_, _, _| Ter::TES_SUCCESS,
        |_, _| Ter::TEC_HAS_OBLIGATIONS,
        |_| true,
        |_, _, _| Ter::TES_SUCCESS,
        |vault_account, asset| {
            assert_eq!(*vault_account, "vault-account");
            assert_eq!(*asset, "USD");
            true
        },
        |_, _| associated.set(true),
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
    assert!(!associated.get());
}

#[test]
fn vault_clawback_do_apply_tail_runs_full_success_path_in_current() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_clawback_do_apply_tail(
        &"issuer",
        &test_state("holder", 4),
        {
            let steps = Rc::clone(&steps);
            move |holder, vault_account, shares_destroyed| {
                steps.borrow_mut().push(format!(
                    "send_shares:{holder}:{vault_account}:{shares_destroyed}"
                ));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |holder, shares_destroyed| {
                steps
                    .borrow_mut()
                    .push(format!("remove_empty:{holder}:{shares_destroyed}"));
                Ter::TEC_HAS_OBLIGATIONS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |assets_recovered| {
                steps
                    .borrow_mut()
                    .push(format!("is_positive:{assets_recovered}"));
                *assets_recovered > 0
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |from, to, amount| {
                steps
                    .borrow_mut()
                    .push(format!("send_assets:{from}:{to}:{amount}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault_account, asset| {
                steps
                    .borrow_mut()
                    .push(format!("check_negative:{vault_account}:{asset}"));
                false
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault, asset| {
                steps
                    .borrow_mut()
                    .push(format!("associate:{}:{asset}", vault.owner()));
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "send_shares:holder:vault-account:9",
            "remove_empty:holder:9",
            "is_positive:4",
            "send_assets:vault-account:issuer:4",
            "check_negative:vault-account:USD",
            "associate:vault-owner:USD",
        ]
    );
}
