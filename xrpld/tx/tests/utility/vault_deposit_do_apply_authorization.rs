//! Integration tests that pin the narrowed Rust
//! `VaultDeposit.cpp::doApply()` load and authorization shell to the current
//! C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultDepositDoApplyAuthorizationIssuance, VaultDepositDoApplyAuthorizationState,
    VaultDepositDoApplyAuthorizationVault, VaultDepositDoApplyAuthorizeRequest,
    load_vault_deposit_do_apply_authorization,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestVault {
    asset: &'static str,
    account: &'static str,
    owner: &'static str,
    share_mpt_id: &'static str,
    private: bool,
}

impl VaultDepositDoApplyAuthorizationVault for TestVault {
    type Asset = &'static str;
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

fn public_vault() -> TestVault {
    TestVault {
        asset: "USD",
        account: "vault-account",
        owner: "vault-owner",
        share_mpt_id: "share-id",
        private: false,
    }
}

fn private_vault() -> TestVault {
    TestVault {
        private: true,
        ..public_vault()
    }
}

fn issuance() -> TestIssuance {
    TestIssuance {
        issuer: "share-issuer",
    }
}

#[test]
fn vault_deposit_do_apply_authorization_returns_tefinternal_when_vault_is_missing() {
    let issuance_read = Cell::new(false);

    let result = load_vault_deposit_do_apply_authorization(
        &"submitter",
        &11_u32,
        || None::<TestVault>,
        |_| {
            issuance_read.set(true);
            Some(issuance())
        },
        |_, _| false,
        |_, _, _| Ter::TES_SUCCESS,
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Err(Ter::TEF_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
    assert!(!issuance_read.get());
}

#[test]
fn vault_deposit_do_apply_authorization_returns_private_non_owner_enforce_failure_unchanged() {
    let authorize_called = Cell::new(false);

    let result = load_vault_deposit_do_apply_authorization(
        &"depositor",
        &17_u32,
        || Some(private_vault()),
        |_| Some(issuance()),
        |_, _| {
            panic!("mptoken_exists should not run for private non-owner deposits");
        },
        |share_mpt_id, submitter, prior_balance| {
            assert_eq!(*share_mpt_id, "share-id");
            assert_eq!(*submitter, "depositor");
            assert_eq!(*prior_balance, 17);
            Ter::TEC_NO_AUTH
        },
        |_| {
            authorize_called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Err(Ter::TEC_NO_AUTH));
    assert_eq!(trans_token(result.unwrap_err()), "tecNO_AUTH");
    assert!(!authorize_called.get());
}

#[test]
fn vault_deposit_do_apply_authorization_skips_authorize_when_public_token_exists() {
    let authorize_called = Cell::new(false);

    let result = load_vault_deposit_do_apply_authorization(
        &"depositor",
        &23_u32,
        || Some(public_vault()),
        |_| Some(issuance()),
        |share_mpt_id, submitter| {
            assert_eq!(*share_mpt_id, "share-id");
            assert_eq!(*submitter, "depositor");
            true
        },
        |_, _, _| Ter::TES_SUCCESS,
        |_| {
            authorize_called.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(
        result,
        Ok(VaultDepositDoApplyAuthorizationState {
            vault: public_vault(),
            vault_asset: "USD",
            vault_account: "vault-account",
            share_mpt_id: "share-id",
            share_issuance: issuance(),
        })
    );
    assert!(!authorize_called.get());
}

#[test]
fn vault_deposit_do_apply_authorization_runs_public_missing_token_path_in_current() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = load_vault_deposit_do_apply_authorization(
        &"depositor",
        &31_u32,
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("vault");
                Some(public_vault())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |share_mpt_id| {
                steps.borrow_mut().push("issuance");
                assert_eq!(*share_mpt_id, "share-id");
                Some(issuance())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |share_mpt_id, submitter| {
                steps.borrow_mut().push("exists");
                assert_eq!(*share_mpt_id, "share-id");
                assert_eq!(*submitter, "depositor");
                false
            }
        },
        |_, _, _| {
            panic!("enforce should not run for public deposits");
        },
        {
            let steps = Rc::clone(&steps);
            move |request: VaultDepositDoApplyAuthorizeRequest<'_, _, _, _>| {
                steps.borrow_mut().push("authorize");
                assert_eq!(request.account, &"depositor");
                assert_eq!(request.holder, None);
                assert_eq!(request.prior_balance, &31_u32);
                Ter::TES_SUCCESS
            }
        },
    );

    assert!(result.is_ok());
    assert_eq!(
        steps.borrow().as_slice(),
        ["vault", "issuance", "exists", "authorize"]
    );
}

#[test]
fn vault_deposit_do_apply_authorization_runs_private_owner_path_in_current() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = load_vault_deposit_do_apply_authorization(
        &"vault-owner",
        &41_u32,
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("vault");
                Some(private_vault())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |share_mpt_id| {
                steps.borrow_mut().push("issuance");
                assert_eq!(*share_mpt_id, "share-id");
                Some(issuance())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |share_mpt_id, submitter| {
                steps.borrow_mut().push("exists");
                assert_eq!(*share_mpt_id, "share-id");
                assert_eq!(*submitter, "vault-owner");
                false
            }
        },
        |_, _, _| {
            panic!("enforce should not run for the private owner path");
        },
        {
            let steps = Rc::clone(&steps);
            move |request: VaultDepositDoApplyAuthorizeRequest<'_, _, _, _>| {
                steps.borrow_mut().push(match request.holder {
                    None => "authorize-owner-token",
                    Some(holder) => {
                        assert_eq!(holder, &"vault-owner");
                        "authorize-issuer-holder"
                    }
                });
                Ter::TES_SUCCESS
            }
        },
    );

    assert!(result.is_ok());
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "vault",
            "issuance",
            "exists",
            "authorize-owner-token",
            "authorize-issuer-holder",
        ]
    );
}

#[test]
fn vault_deposit_do_apply_authorization_returns_second_authorize_failure_unchanged() {
    let result = load_vault_deposit_do_apply_authorization(
        &"vault-owner",
        &59_u32,
        || Some(private_vault()),
        |_| Some(issuance()),
        |_, _| false,
        |_, _, _| Ter::TES_SUCCESS,
        |request| {
            if request.holder.is_none() {
                Ter::TES_SUCCESS
            } else {
                assert_eq!(request.account, &"share-issuer");
                assert_eq!(request.holder, Some(&"vault-owner"));
                Ter::TEC_INSUFFICIENT_FUNDS
            }
        },
    );

    assert_eq!(result, Err(Ter::TEC_INSUFFICIENT_FUNDS));
    assert_eq!(trans_token(result.unwrap_err()), "tecINSUFFICIENT_FUNDS");
}
