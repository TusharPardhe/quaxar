//! Integration tests that pin the narrowed Rust
//! `Transactor.cpp::checkSingleSign(...)` helper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{TransactorSingleSignAccountState, run_transactor_check_single_sign};

struct StubAccountState {
    regular_key: Option<&'static str>,
    is_master_disabled: bool,
}

impl TransactorSingleSignAccountState<&'static str> for StubAccountState {
    fn regular_key(&self) -> Option<&&'static str> {
        self.regular_key.as_ref()
    }

    fn is_master_disabled(&self) -> bool {
        self.is_master_disabled
    }
}

#[test]
fn tx_transactor_single_sign_accepts_regular_key_even_if_master_is_disabled() {
    let result = run_transactor_check_single_sign(
        &"regular",
        &"alice",
        &StubAccountState {
            regular_key: Some("regular"),
            is_master_disabled: true,
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_transactor_single_sign_accepts_enabled_master_key() {
    let result = run_transactor_check_single_sign(
        &"alice",
        &"alice",
        &StubAccountState {
            regular_key: Some("regular"),
            is_master_disabled: false,
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_transactor_single_sign_rejects_disabled_master_key() {
    let result = run_transactor_check_single_sign(
        &"alice",
        &"alice",
        &StubAccountState {
            regular_key: Some("regular"),
            is_master_disabled: true,
        },
    );

    assert_eq!(result, Ter::TEF_MASTER_DISABLED);
    assert_eq!(trans_token(result), "tefMASTER_DISABLED");
}

#[test]
fn tx_transactor_single_sign_rejects_any_other_signer() {
    let result = run_transactor_check_single_sign(
        &"mallory",
        &"alice",
        &StubAccountState {
            regular_key: Some("regular"),
            is_master_disabled: false,
        },
    );

    assert_eq!(result, Ter::TEF_BAD_AUTH);
    assert_eq!(trans_token(result), "tefBAD_AUTH");
}
