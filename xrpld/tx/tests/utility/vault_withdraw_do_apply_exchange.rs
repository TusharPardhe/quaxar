//! Integration tests that pin the narrowed Rust
//! `VaultWithdraw.cpp::doApply()` load and exchange shell to the current C++
//! behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{VaultWithdrawDoApplyExchangeVault, load_vault_withdraw_do_apply_exchange};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    asset: &'static str,
    account: &'static str,
    owner: &'static str,
    share_mpt_id: &'static str,
}

impl VaultWithdrawDoApplyExchangeVault for TestVault {
    type Asset = &'static str;
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

fn test_vault() -> TestVault {
    TestVault {
        asset: "USD",
        account: "vault-account",
        owner: "vault-owner",
        share_mpt_id: "share-id",
    }
}

#[test]
fn vault_withdraw_do_apply_exchange_returns_tefinternal_when_vault_is_missing() {
    let issuance_read = Cell::new(false);

    let result = load_vault_withdraw_do_apply_exchange(
        &50_i64,
        || None::<TestVault>,
        |_| {
            issuance_read.set(true);
            Some("issuance")
        },
        |_, _| true,
        |_, _| false,
        |amount| *amount,
        |_, _, _| Ok::<_, ()>(Some(10_i64)),
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |_| false,
    );

    assert_eq!(result, Err(Ter::TEF_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
    assert!(!issuance_read.get());
}

#[test]
fn vault_withdraw_do_apply_exchange_returns_asset_branch_failures() {
    let missing = load_vault_withdraw_do_apply_exchange(
        &50_i64,
        || Some(test_vault()),
        |_| Some("issuance"),
        |_, _| true,
        |_, _| false,
        |amount| *amount,
        |_, _, _| Ok::<_, ()>(None::<i64>),
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |_| false,
    );
    let zero = load_vault_withdraw_do_apply_exchange(
        &50_i64,
        || Some(test_vault()),
        |_| Some("issuance"),
        |_, _| true,
        |_, _| false,
        |amount| *amount,
        |_, _, _| Ok::<_, ()>(Some(0_i64)),
        |_, _, _| Ok::<_, ()>(Some(25_i64)),
        |shares| *shares == 0,
    );

    assert_eq!(missing, Err(Ter::TEC_INTERNAL));
    assert_eq!(zero, Err(Ter::TEC_PRECISION_LOSS));
    assert_eq!(trans_token(zero.unwrap_err()), "tecPRECISION_LOSS");
}

#[test]
fn vault_withdraw_do_apply_exchange_maps_overflow_to_path_dry() {
    let asset_branch = load_vault_withdraw_do_apply_exchange(
        &50_i64,
        || Some(test_vault()),
        |_| Some("issuance"),
        |_, _| true,
        |_, _| false,
        |amount| *amount,
        |_, _, _| Err::<Option<i64>, &'static str>("overflow"),
        |_, _, _| Ok::<_, &'static str>(Some(25_i64)),
        |_| false,
    );
    let share_branch = load_vault_withdraw_do_apply_exchange(
        &50_i64,
        || Some(test_vault()),
        |_| Some("issuance"),
        |_, _| false,
        |_, _| true,
        |amount| *amount,
        |_, _, _| Ok::<_, &'static str>(Some(10_i64)),
        |_, _, _| Err::<Option<i64>, &'static str>("overflow"),
        |_| false,
    );

    assert_eq!(asset_branch, Err(Ter::TEC_PATH_DRY));
    assert_eq!(share_branch, Err(Ter::TEC_PATH_DRY));
}

#[test]
fn vault_withdraw_do_apply_exchange_runs_current_on_success() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = load_vault_withdraw_do_apply_exchange(
        &50_i64,
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("read_vault");
                Some(test_vault())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |share_id| {
                steps.borrow_mut().push("read_issuance");
                assert_eq!(*share_id, "share-id");
                Some("issuance")
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |amount, vault| {
                steps.borrow_mut().push("branch_asset");
                assert_eq!(*amount, 50);
                assert_eq!(vault.asset(), &"USD");
                true
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _| {
                steps.borrow_mut().push("branch_share");
                false
            }
        },
        |amount| *amount,
        {
            let steps = Rc::clone(&steps);
            move |_, _, amount| {
                steps.borrow_mut().push("assets_to_shares");
                assert_eq!(*amount, 50);
                Ok::<_, ()>(Some(10_i64))
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |_, _, shares| {
                steps.borrow_mut().push("shares_to_assets");
                assert_eq!(*shares, 10);
                Ok::<_, ()>(Some(25_i64))
            }
        },
        |_| false,
    )
    .unwrap();

    assert_eq!(result.vault_asset, "USD");
    assert_eq!(result.vault_account, "vault-account");
    assert_eq!(result.share_id, "share-id");
    assert_eq!(result.shares_redeemed, 10);
    assert_eq!(result.assets_withdrawn, 25);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "read_vault",
            "read_issuance",
            "branch_asset",
            "assets_to_shares",
            "shares_to_assets",
        ]
    );
}
