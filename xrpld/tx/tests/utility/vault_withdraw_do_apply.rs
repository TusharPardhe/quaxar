//! Integration tests that pin the higher narrowed Rust
//! `VaultWithdraw.cpp::doApply()` wrapper to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultWithdrawDoApplyExchangeVault, VaultWithdrawDoApplyVaultUpdateSink,
    run_vault_withdraw_do_apply,
};

#[derive(Clone)]
struct TestVault {
    asset: Rc<str>,
    account: &'static str,
    owner: &'static str,
    share_mpt_id: &'static str,
    assets_available: i64,
    assets_total: i64,
    loss_unrealized: i64,
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl VaultWithdrawDoApplyExchangeVault for TestVault {
    type Asset = Rc<str>;
    type AccountId = &'static str;
    type ShareId = &'static str;

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn owner(&self) -> &Self::AccountId {
        &self.owner
    }

    fn share_mpt_id(&self) -> &Self::ShareId {
        &self.share_mpt_id
    }
}

impl VaultWithdrawDoApplyVaultUpdateSink for TestVault {
    type Amount = i64;

    fn assets_available(&self) -> &Self::Amount {
        &self.assets_available
    }

    fn assets_total(&self) -> &Self::Amount {
        &self.assets_total
    }

    fn loss_unrealized(&self) -> &Self::Amount {
        &self.loss_unrealized
    }

    fn subtract_assets_total(&mut self, value: Self::Amount) {
        self.assets_total -= value;
        self.steps
            .borrow_mut()
            .push(format!("assets_total-={value}"));
    }

    fn subtract_assets_available(&mut self, value: Self::Amount) {
        self.assets_available -= value;
        self.steps
            .borrow_mut()
            .push(format!("assets_available-={value}"));
    }

    fn update_vault(&mut self) {
        self.steps.borrow_mut().push("update_vault".to_string());
    }
}

fn build_vault(steps: Rc<std::cell::RefCell<Vec<String>>>) -> TestVault {
    TestVault {
        asset: Rc::from("USD"),
        account: "vault-account",
        owner: "vault-owner",
        share_mpt_id: "share-id",
        assets_available: 80,
        assets_total: 100,
        loss_unrealized: 10,
        steps,
    }
}

#[test]
fn vault_withdraw_do_apply_runs_current_cpp_stage_order_on_success() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let share_branch_checked = Cell::new(false);

    let result = run_vault_withdraw_do_apply(
        &"depositor",
        None,
        &50_i64,
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
            move |amount, vault| {
                steps.borrow_mut().push("branch_asset".to_string());
                assert_eq!(*amount, 50);
                assert_eq!(vault.asset().as_ref(), "USD");
                true
            }
        },
        |_, _| {
            share_branch_checked.set(true);
            false
        },
        |amount| *amount,
        {
            let steps = Rc::clone(&steps);
            move |_, _, amount| {
                steps
                    .borrow_mut()
                    .push(format!("assets_to_shares:{amount}"));
                Ok::<_, ()>(Some(10_i64))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _, shares| {
                steps
                    .borrow_mut()
                    .push(format!("shares_to_assets:{shares}"));
                Ok::<_, ()>(Some(25_i64))
            }
        },
        |_| false,
        {
            let steps = Rc::clone(&steps);
            move |shares| {
                steps.borrow_mut().push(format!("has_shares:{shares}"));
                true
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault_account, shares| {
                steps
                    .borrow_mut()
                    .push(format!("send_shares:{vault_account}:{shares}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |account, share_id| {
                steps
                    .borrow_mut()
                    .push(format!("remove_empty_holding:{account}:{share_id}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, asset| {
                steps.borrow_mut().push(format!("associate:{asset}"));
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |dst_account, vault_account, assets| {
                steps.borrow_mut().push(format!(
                    "do_withdraw:{dst_account}:{vault_account}:{assets}"
                ));
                Ter::TES_SUCCESS
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(!share_branch_checked.get());
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault",
            "read_issuance",
            "branch_asset",
            "assets_to_shares:50",
            "shares_to_assets:10",
            "has_shares:10",
            "assets_total-=25",
            "assets_available-=25",
            "update_vault",
            "send_shares:vault-account:10",
            "remove_empty_holding:depositor:share-id",
            "associate:USD",
            "do_withdraw:depositor:vault-account:25",
        ]
    );
}

#[test]
fn vault_withdraw_do_apply_returns_exchange_failure_before_later_stages() {
    let update_called = Cell::new(false);

    let result = run_vault_withdraw_do_apply(
        &"depositor",
        Some("erin"),
        &50_i64,
        || None::<TestVault>,
        |_| Some("issuance"),
        |_, _| true,
        |_, _| false,
        |amount| *amount,
        |_, _, _| Ok::<_, ()>(Some(10_i64)),
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |_| false,
        |_| {
            update_called.set(true);
            true
        },
        |_, _| Ter::TES_SUCCESS,
        |_, _| Ter::TES_SUCCESS,
        |_, _| {},
        |_, _, _| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
    assert!(!update_called.get());
}

#[test]
fn vault_withdraw_do_apply_returns_guard_failure_before_transfer_tail() {
    let transfer_called = Cell::new(false);

    let result = run_vault_withdraw_do_apply(
        &"depositor",
        Some("erin"),
        &50_i64,
        || Some(build_vault(Rc::new(std::cell::RefCell::new(Vec::new())))),
        |_| Some("issuance"),
        |_, _| true,
        |_, _| false,
        |amount| *amount,
        |_, _, _| Ok::<_, ()>(Some(10_i64)),
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |_| false,
        |_| false,
        |_, _| {
            transfer_called.set(true);
            Ter::TES_SUCCESS
        },
        |_, _| Ter::TES_SUCCESS,
        |_, _| {},
        |_, _, _| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_FUNDS");
    assert!(!transfer_called.get());
}

#[test]
fn vault_withdraw_do_apply_returns_transfer_tail_failure_after_earlier_success() {
    let withdrew = Cell::new(false);

    let result = run_vault_withdraw_do_apply(
        &"depositor",
        Some("erin"),
        &50_i64,
        || Some(build_vault(Rc::new(std::cell::RefCell::new(Vec::new())))),
        |_| Some("issuance"),
        |_, _| true,
        |_, _| false,
        |amount| *amount,
        |_, _, _| Ok::<_, ()>(Some(10_i64)),
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |_| false,
        |_| true,
        |_, _| Ter::TEC_NO_PERMISSION,
        |_, _| Ter::TES_SUCCESS,
        |_, _| {},
        |_, _, _| {
            withdrew.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
    assert!(!withdrew.get());
}
