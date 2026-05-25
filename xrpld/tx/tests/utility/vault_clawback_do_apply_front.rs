//! Integration tests that pin the narrowed Rust
//! `VaultClawback.cpp::doApply()` front shell to the current C++ behavior.

use std::{
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

use protocol::{Ter, trans_token};
use tx::{
    VAULT_CLAWBACK_DO_APPLY_FRONT_ASSERT_MESSAGE, VaultClawbackAmountVault,
    VaultClawbackAssetsToClawback, VaultClawbackAssetsToClawbackVault,
    VaultClawbackDoApplyFrontState, VaultClawbackDoApplyFrontVault,
    load_vault_clawback_do_apply_front,
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
fn vault_clawback_do_apply_front_returns_tefinternal_when_vault_is_missing() {
    let issuance_read = Cell::new(false);

    let result = load_vault_clawback_do_apply_front(
        &"issuer",
        "holder",
        None::<i64>,
        || None::<TestVault>,
        |_| {
            issuance_read.set(true);
            Some("issuance")
        },
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
    );

    assert_eq!(result, Err(Ter::TEF_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
    assert!(!issuance_read.get());
}

#[test]
fn vault_clawback_do_apply_front_panics_on_loss_invariant_before_branch_work() {
    let holds_called = Cell::new(false);
    let assets_called = Cell::new(false);

    let panic = catch_unwind(AssertUnwindSafe(|| {
        load_vault_clawback_do_apply_front(
            &"issuer",
            "holder",
            Some(4_i64),
            || {
                Some(TestVault {
                    loss_unrealized: 25,
                    ..build_vault(Rc::new(RefCell::new(Vec::new())))
                })
            },
            |_| Some("issuance"),
            |_| 0_i64,
            |_| 0_i64,
            |_| 0_i64,
            |_, _| false,
            |_, _| {
                holds_called.set(true);
                0_i64
            },
            |_, _, _, _| {
                assets_called.set(true);
                Ok(VaultClawbackAssetsToClawback {
                    assets_recovered: 1_i64,
                    shares_destroyed: 1_i64,
                })
            },
            |_| false,
        )
    }))
    .expect_err("invalid loss invariant should panic");

    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&'static str>().copied())
        .expect("panic payload should be a string");

    assert!(message.contains(VAULT_CLAWBACK_DO_APPLY_FRONT_ASSERT_MESSAGE));
    assert!(!holds_called.get());
    assert!(!assets_called.get());
}

#[test]
fn vault_clawback_do_apply_front_runs_owner_share_branch_in_current() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let assets_called = Cell::new(false);

    let result = load_vault_clawback_do_apply_front(
        &"vault-owner",
        "holder",
        None::<i64>,
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
        {
            let steps = Rc::clone(&steps);
            move |share_id| {
                steps.borrow_mut().push(format!("make_share:{share_id}"));
                0_i64
            }
        },
        |_| 0_i64,
        {
            let steps = Rc::clone(&steps);
            move |_| {
                steps.borrow_mut().push("make_zero_recovered".to_string());
                0_i64
            }
        },
        |amount, _| *amount == 0,
        {
            let steps = Rc::clone(&steps);
            move |share_id, holder| {
                steps
                    .borrow_mut()
                    .push(format!("account_holds:{share_id}:{holder}"));
                9_i64
            }
        },
        |_, _, _, _| {
            assets_called.set(true);
            Ok(VaultClawbackAssetsToClawback {
                assets_recovered: 1_i64,
                shares_destroyed: 1_i64,
            })
        },
        |_| false,
    );

    assert_eq!(
        result,
        Ok(VaultClawbackDoApplyFrontState {
            vault: TestVault {
                assets_available: 80,
                assets_total: 100,
                loss_unrealized: 10,
                asset: "USD",
                owner: "vault-owner",
                account: "vault-account",
                share_mpt_id: "share-id",
                steps: Rc::clone(&steps),
            },
            vault_asset: "USD",
            vault_account: "vault-account",
            share_id: "share-id",
            holder: "holder",
            assets_recovered: 0_i64,
            shares_destroyed: 9_i64,
        })
    );
    assert!(!assets_called.get());
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault",
            "read_issuance",
            "make_share:share-id",
            "make_zero_recovered",
            "account_holds:share-id:holder",
            "assets_total-=0",
            "assets_available-=0",
            "update_vault",
        ]
    );
}

#[test]
fn vault_clawback_do_apply_front_returns_asset_branch_failure_before_update() {
    let zero_amount_built = Cell::new(false);

    let result = load_vault_clawback_do_apply_front(
        &"issuer",
        "holder",
        Some(4_i64),
        || Some(build_vault(Rc::new(RefCell::new(Vec::new())))),
        |_| Some("issuance"),
        |_| 0_i64,
        |_| 0_i64,
        |_| {
            zero_amount_built.set(true);
            0_i64
        },
        |_, _| false,
        |_, _| 0_i64,
        |_, _, _, _| Err(Ter::TEC_PATH_DRY),
        |_| false,
    );

    assert_eq!(result, Err(Ter::TEC_PATH_DRY));
    assert_eq!(trans_token(result.unwrap_err()), "tecPATH_DRY");
    assert!(zero_amount_built.get());
}
