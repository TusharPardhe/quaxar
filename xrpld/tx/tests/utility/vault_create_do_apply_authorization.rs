//! Integration tests that pin the narrowed Rust
//! `VaultCreate.cpp::doApply()` authorization and asset-association tail to the
//! current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VAULT_PRIVATE_FLAG, VaultCreateDoApplyAuthorizeRequest,
    run_vault_create_do_apply_authorization_tail,
};

#[test]
fn vault_create_do_apply_authorization_tail_runs_private_vault_tail_in_current() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let asset: Rc<str> = Rc::from("USD");

    let result = run_vault_create_do_apply_authorization_tail(
        VAULT_PRIVATE_FLAG,
        &"share-id",
        &"owner",
        &"pseudo",
        &asset,
        {
            let steps = Rc::clone(&steps);
            move |request| {
                steps.borrow_mut().push(format!(
                    "authorize:{}:{}",
                    request.account,
                    request.holder.copied().unwrap_or("none")
                ));
                assert_eq!(request.share_mpt_id, &"share-id");
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |asset| {
                steps.borrow_mut().push(format!("associate:{asset}"));
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "authorize:owner:none",
            "authorize:pseudo:owner",
            "associate:USD",
        ]
    );
}

#[test]
fn vault_create_do_apply_authorization_tail_skips_pseudo_auth_for_public_vault() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));
    let asset: Rc<str> = Rc::from("XRP");

    let result = run_vault_create_do_apply_authorization_tail(
        0,
        &"share-id",
        &"owner",
        &"pseudo",
        &asset,
        {
            let steps = Rc::clone(&steps);
            move |request: VaultCreateDoApplyAuthorizeRequest<'_, _, _>| {
                steps.borrow_mut().push(format!(
                    "authorize:{}:{}",
                    request.account,
                    request.holder.copied().unwrap_or("none")
                ));
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |asset| {
                steps.borrow_mut().push(format!("associate:{asset}"));
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        ["authorize:owner:none", "associate:XRP"]
    );
}

#[test]
fn vault_create_do_apply_authorization_tail_returns_owner_authorize_failure_unchanged() {
    let pseudo_called = Cell::new(false);
    let associate_called = Cell::new(false);

    let result = run_vault_create_do_apply_authorization_tail(
        VAULT_PRIVATE_FLAG,
        &"share-id",
        &"owner",
        &"pseudo",
        &"USD",
        |request| {
            if request.account == &"pseudo" {
                pseudo_called.set(true);
            }
            Ter::TEC_NO_AUTH
        },
        |_| {
            associate_called.set(true);
        },
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
    assert!(!pseudo_called.get());
    assert!(!associate_called.get());
}

#[test]
fn vault_create_do_apply_authorization_tail_returns_private_authorize_failure_unchanged() {
    let associate_called = Cell::new(false);

    let result = run_vault_create_do_apply_authorization_tail(
        VAULT_PRIVATE_FLAG,
        &"share-id",
        &"owner",
        &"pseudo",
        &"USD",
        |request| {
            if request.account == &"owner" {
                Ter::TES_SUCCESS
            } else {
                assert_eq!(request.holder, Some(&"owner"));
                Ter::TEC_INSUFFICIENT_RESERVE
            }
        },
        |_| {
            associate_called.set(true);
        },
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
    assert!(!associate_called.get());
}
