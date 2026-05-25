//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! vault-pseudo freeze guard to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{LOAN_SET_VAULT_PSEUDO_ACCOUNT_IS_FROZEN_WARNING, check_loan_set_preclaim_vault_frozen};

#[test]
fn tx_loan_set_preclaim_vault_frozen_returns_success_when_not_frozen() {
    let result = check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"XRP", |account, asset| {
        assert_eq!(*account, "vault-pseudo");
        assert_eq!(*asset, "XRP");
        Ter::TES_SUCCESS
    });

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_preclaim_vault_frozen_returns_frozen_ter_unchanged() {
    let result =
        check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"USD", |_, _| Ter::TEC_FROZEN);

    let err = result.unwrap_err();
    assert_eq!(err.ter(), Ter::TEC_FROZEN);
    assert_eq!(trans_token(err.ter()), "tecFROZEN");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_VAULT_PSEUDO_ACCOUNT_IS_FROZEN_WARNING
    );
}

#[test]
fn tx_loan_set_preclaim_vault_frozen_returns_locked_ter_unchanged() {
    let result =
        check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"MPT", |_, _| Ter::TEC_LOCKED);

    let err = result.unwrap_err();
    assert_eq!(err.ter(), Ter::TEC_LOCKED);
    assert_eq!(trans_token(err.ter()), "tecLOCKED");
}

#[test]
fn tx_loan_set_preclaim_vault_frozen_checks_account_and_asset_exactly_once() {
    let mut seen = Vec::new();

    let result = check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"USD", |account, asset| {
        seen.push((*account, *asset));
        Ter::TEC_FROZEN
    });

    assert_eq!(result.unwrap_err().ter(), Ter::TEC_FROZEN);
    assert_eq!(seen, vec![("vault-pseudo", "USD")]);
}
