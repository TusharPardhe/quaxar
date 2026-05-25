//! Integration tests that pin the narrowed Rust
//! `VaultClawback.cpp::doApply()` wrapper to the current C++ behavior.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use protocol::{Ter, trans_token};
use tx::{
    VaultClawbackAmountVault, VaultClawbackAssetsToClawback, VaultClawbackAssetsToClawbackVault,
    VaultClawbackDoApplyFrontVault, run_vault_clawback_do_apply,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestVault {
    asset: &'static str,
    owner: &'static str,
    account: &'static str,
    share_mpt_id: &'static str,
    assets_available: i64,
    assets_total: i64,
    loss_unrealized: i64,
    steps: Rc<RefCell<Vec<String>>>,
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

impl VaultClawbackAssetsToClawbackVault for TestVault {
    type Amount = i64;

    fn assets_available(&self) -> &Self::Amount {
        &self.assets_available
    }
}

impl VaultClawbackDoApplyFrontVault for TestVault {
    fn account(&self) -> &<Self as VaultClawbackAmountVault>::AccountId {
        &self.account
    }

    fn assets_total(&self) -> &<Self as VaultClawbackAssetsToClawbackVault>::Amount {
        &self.assets_total
    }

    fn loss_unrealized(&self) -> &<Self as VaultClawbackAssetsToClawbackVault>::Amount {
        &self.loss_unrealized
    }

    fn subtract_assets_total(
        &mut self,
        value: <Self as VaultClawbackAssetsToClawbackVault>::Amount,
    ) {
        self.assets_total -= value;
        self.steps
            .borrow_mut()
            .push(format!("assets_total-={value}"));
    }

    fn subtract_assets_available(
        &mut self,
        value: <Self as VaultClawbackAssetsToClawbackVault>::Amount,
    ) {
        self.assets_available -= value;
        self.steps
            .borrow_mut()
            .push(format!("assets_available-={value}"));
    }

    fn update_vault(&mut self) {
        self.steps.borrow_mut().push("update_vault".to_string());
    }
}

fn build_vault(steps: Rc<RefCell<Vec<String>>>) -> TestVault {
    TestVault {
        asset: "USD",
        owner: "vault-owner",
        account: "vault-account",
        share_mpt_id: "share-id",
        assets_available: 80,
        assets_total: 100,
        loss_unrealized: 10,
        steps,
    }
}

#[test]
fn vault_clawback_do_apply_returns_front_failure_unchanged() {
    let tail_called = Cell::new(false);

    let result = run_vault_clawback_do_apply(
        &"issuer",
        "holder",
        None::<i64>,
        || None::<TestVault>,
        |_| Some("issuance"),
        |_| 0_i64,
        |_| 0_i64,
        |_| 0_i64,
        |_, _| false,
        |_, _| 0_i64,
        |_, _, _, _| {
            Ok(VaultClawbackAssetsToClawback {
                assets_recovered: 1_i64,
                shares_destroyed: 1_i64,
            })
        },
        |_| false,
        |_, _, _| {
            tail_called.set(true);
            Ter::TES_SUCCESS
        },
        |_, _| {
            tail_called.set(true);
            Ter::TES_SUCCESS
        },
        |_| {
            tail_called.set(true);
            false
        },
        |_, _, _| {
            tail_called.set(true);
            Ter::TES_SUCCESS
        },
        |_, _| {
            tail_called.set(true);
            false
        },
        |_, _| tail_called.set(true),
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
    assert!(!tail_called.get());
}

#[test]
fn vault_clawback_do_apply_returns_tail_failure_unchanged() {
    let balance_checked = Cell::new(false);
    let associated = Cell::new(false);

    let result = run_vault_clawback_do_apply(
        &"issuer",
        "holder",
        Some(4_i64),
        || Some(build_vault(Rc::new(RefCell::new(Vec::new())))),
        |_| Some("issuance"),
        |_| 0_i64,
        |_| 0_i64,
        |_| 0_i64,
        |_, _| false,
        |_, _| 0_i64,
        |_, _, _, _| {
            Ok(VaultClawbackAssetsToClawback {
                assets_recovered: 4_i64,
                shares_destroyed: 9_i64,
            })
        },
        |_| false,
        |_, _, _| Ter::TES_SUCCESS,
        |_, _| Ter::TEC_HAS_OBLIGATIONS,
        |_| true,
        |_, _, _| Ter::TER_RETRY,
        |_, _| {
            balance_checked.set(true);
            false
        },
        |_, _| associated.set(true),
    );

    assert_eq!(result, Ter::TER_RETRY);
    assert_eq!(trans_token(result), "terRETRY");
    assert!(!balance_checked.get());
    assert!(!associated.get());
}

#[test]
fn vault_clawback_do_apply_runs_current_cpp_stage_order_on_success() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let assets_called = Rc::new(Cell::new(false));

    let result = run_vault_clawback_do_apply(
        &"issuer",
        "holder",
        Some(4_i64),
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("read_vault".to_string());
                Some(build_vault(Rc::clone(&steps)))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps.borrow_mut().push("read_issuance".to_string());
                Some("issuance")
            }
        },
        |_| 0_i64,
        |_| 0_i64,
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps.borrow_mut().push("make_zero_recovered".to_string());
                0_i64
            }
        },
        |_, _| false,
        {
            let steps = Rc::clone(&steps);
            move |_, _| {
                steps.borrow_mut().push("account_holds".to_string());
                9_i64
            }
        },
        {
            let steps = Rc::clone(&steps);
            let assets_called = Rc::clone(&assets_called);
            move |_, _, _, _| {
                assets_called.set(true);
                steps.borrow_mut().push("assets_to_clawback".to_string());
                Ok(VaultClawbackAssetsToClawback {
                    assets_recovered: 4_i64,
                    shares_destroyed: 9_i64,
                })
            }
        },
        |_| false,
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
    assert!(assets_called.get());
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault",
            "read_issuance",
            "make_zero_recovered",
            "assets_to_clawback",
            "assets_total-=4",
            "assets_available-=4",
            "update_vault",
            "send_shares:holder:vault-account:9",
            "remove_empty:holder:9",
            "is_positive:4",
            "send_assets:vault-account:issuer:4",
            "check_negative:vault-account:USD",
            "associate:vault-owner:USD",
        ]
    );
}
