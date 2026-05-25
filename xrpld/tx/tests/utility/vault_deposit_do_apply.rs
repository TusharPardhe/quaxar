//! Integration tests that pin the narrowed Rust higher
//! `VaultDeposit.cpp::doApply()` composition wrapper to the current C++
//! behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultDepositDoApplyAuthorizationIssuance, VaultDepositDoApplyAuthorizationVault,
    VaultDepositDoApplyAuthorizeRequest, VaultDepositDoApplyVaultUpdateSink,
    run_vault_deposit_do_apply,
};

#[derive(Clone)]
struct TestVault {
    asset: Rc<str>,
    account: &'static str,
    owner: &'static str,
    share_mpt_id: &'static str,
    private: bool,
    maximum: Option<i64>,
    steps: Rc<std::cell::RefCell<Vec<String>>>,
}

impl VaultDepositDoApplyAuthorizationVault for TestVault {
    type Asset = Rc<str>;
    type AccountId = &'static str;
    type ShareId = &'static str;
    type Amount = i64;

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

    fn is_private(&self) -> bool {
        self.private
    }
}

impl VaultDepositDoApplyVaultUpdateSink for TestVault {
    type Amount = i64;

    fn add_assets_total(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("assets_total+={value}"));
    }

    fn add_assets_available(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("assets_available+={value}"));
    }

    fn assets_total(&self) -> &Self::Amount {
        static TOTAL: i64 = 0;
        &TOTAL
    }

    fn update_vault(&mut self) {
        self.steps.borrow_mut().push("update_vault".to_string());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestIssuance {
    issuer: &'static str,
}

impl VaultDepositDoApplyAuthorizationIssuance for TestIssuance {
    type AccountId = &'static str;

    fn issuer(&self) -> &Self::AccountId {
        &self.issuer
    }
}

fn build_vault(steps: Rc<std::cell::RefCell<Vec<String>>>) -> TestVault {
    TestVault {
        asset: Rc::from("USD"),
        account: "vault-account",
        owner: "vault-owner",
        share_mpt_id: "share-id",
        private: false,
        maximum: Some(100),
        steps,
    }
}

#[test]
fn vault_deposit_do_apply_runs_current_cpp_stage_order_on_success() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_deposit_do_apply(
        &"depositor",
        &10_u32,
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
                Some(TestIssuance {
                    issuer: "share-issuer",
                })
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _| {
                steps.borrow_mut().push("mptoken_exists".to_string());
                false
            }
        },
        |_, _, _| Ter::TES_SUCCESS,
        {
            let steps = Rc::clone(&steps);
            move |request: VaultDepositDoApplyAuthorizeRequest<'_, _, _, _>| {
                steps
                    .borrow_mut()
                    .push(format!("authorize:{}", request.account));
                Ter::TES_SUCCESS
            }
        },
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
        |assets, amount| *assets > *amount,
        |vault| vault.maximum,
        {
            let steps = Rc::clone(&steps);
            move |vault_account, assets| {
                steps
                    .borrow_mut()
                    .push(format!("send_assets:{vault_account}:{assets}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |assets| {
                steps.borrow_mut().push(format!("check_negative:{assets}"));
                false
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
            move |_, asset| {
                steps.borrow_mut().push(format!("associate:{asset}"));
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault",
            "read_issuance",
            "mptoken_exists",
            "authorize:depositor",
            "assets_to_shares:50",
            "shares_to_assets:10",
            "assets_total+=25",
            "assets_available+=25",
            "update_vault",
            "send_assets:vault-account:25",
            "check_negative:25",
            "send_shares:vault-account:10",
            "associate:USD",
        ]
    );
}

#[test]
fn vault_deposit_do_apply_returns_authorization_failure_first() {
    let assets_to_shares_called = Cell::new(false);

    let result = run_vault_deposit_do_apply(
        &"depositor",
        &10_u32,
        &50_i64,
        || Some(build_vault(Rc::new(std::cell::RefCell::new(Vec::new())))),
        |_| {
            Some(TestIssuance {
                issuer: "share-issuer",
            })
        },
        |_, _| false,
        |_, _, _| Ter::TES_SUCCESS,
        |_| Ter::TEC_NO_AUTH,
        |_, _, _| {
            assets_to_shares_called.set(true);
            Ok::<_, ()>(Some(10_i64))
        },
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |_| false,
        |assets, amount| *assets > *amount,
        |vault| vault.maximum,
        |_, _| Ter::TES_SUCCESS,
        |_| false,
        |_, _| Ter::TES_SUCCESS,
        |_, _| {},
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
    assert!(!assets_to_shares_called.get());
}

#[test]
fn vault_deposit_do_apply_returns_limit_exceeded_before_transfer() {
    let send_assets_called = Cell::new(false);

    let result = run_vault_deposit_do_apply(
        &"depositor",
        &10_u32,
        &50_i64,
        || Some(build_vault(Rc::new(std::cell::RefCell::new(Vec::new())))),
        |_| {
            Some(TestIssuance {
                issuer: "share-issuer",
            })
        },
        |_, _| true,
        |_, _, _| Ter::TES_SUCCESS,
        |_| Ter::TES_SUCCESS,
        |_, _, _| Ok::<_, ()>(Some(10_i64)),
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |_| false,
        |assets, amount| *assets > *amount,
        |_| Some(-1_i64),
        |_, _| {
            send_assets_called.set(true);
            Ter::TES_SUCCESS
        },
        |_| false,
        |_, _| Ter::TES_SUCCESS,
        |_, _| {},
    );

    assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
    assert_eq!(trans_token(result), "tecLIMIT_EXCEEDED");
    assert!(!send_assets_called.get());
}
