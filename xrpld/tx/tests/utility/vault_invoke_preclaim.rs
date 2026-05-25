//! Integration tests that pin the vault-family `invoke_preclaim(...)` shell to
//! the current C++ ordering.

use std::cell::RefCell;

use protocol::{Ter, TxType, trans_token};
use tx::{
    HasTxnType, UnknownTransactionType, run_vault_invoke_preclaim_for_txn_source,
    run_vault_invoke_preclaim_for_txn_type,
};

struct TestTx {
    txn_type: TxType,
}

impl HasTxnType for TestTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn vault_invoke_preclaim_skips_shared_checks_when_account_is_zero() {
    let result = run_vault_invoke_preclaim_for_txn_type(
        true,
        TxType::VAULT_CREATE,
        || panic!("zero account should skip seq-proxy"),
        || panic!("zero account should skip prior-tx"),
        || panic!("zero account should skip permission"),
        || panic!("zero account should skip sign"),
        || panic!("zero account should skip base-fee"),
        |_| panic!("zero account should skip fee"),
        || Ter::TEC_OBJECT_NOT_FOUND,
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
    )
    .expect("vault create should be a known vault transaction");

    assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(trans_token(result), "tecOBJECT_NOT_FOUND");
}

#[test]
fn vault_invoke_preclaim_preserves_current_for_selected_vault_type() {
    let trace = RefCell::new(Vec::new());

    let result = run_vault_invoke_preclaim_for_txn_type(
        false,
        TxType::VAULT_WITHDRAW,
        || {
            trace.borrow_mut().push("seq");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("prior");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("permission");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("sign");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("base-fee");
            20_u64
        },
        |fee| {
            trace.borrow_mut().push("fee");
            assert_eq!(fee, 20);
            Ter::TES_SUCCESS
        },
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || {
            trace.borrow_mut().push("withdraw-preclaim");
            Ter::TES_SUCCESS
        },
        || panic!("wrong vault preclaim selected"),
    )
    .expect("vault withdraw should be a known vault transaction");

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        trace.into_inner(),
        vec![
            "seq",
            "prior",
            "permission",
            "sign",
            "base-fee",
            "fee",
            "withdraw-preclaim"
        ]
    );
}

#[test]
fn vault_invoke_preclaim_returns_first_shared_failure_unchanged() {
    let result = run_vault_invoke_preclaim_for_txn_type(
        false,
        TxType::VAULT_SET,
        || Ter::TES_SUCCESS,
        || Ter::TEF_WRONG_PRIOR,
        || panic!("prior failure should skip permission"),
        || panic!("prior failure should skip sign"),
        || panic!("prior failure should skip base-fee"),
        |_| panic!("prior failure should skip fee"),
        || panic!("wrong vault preclaim selected"),
        || panic!("prior failure should skip set preclaim"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
        || panic!("wrong vault preclaim selected"),
    )
    .expect("vault set should be a known vault transaction");

    assert_eq!(result, Ter::TEF_WRONG_PRIOR);
    assert_eq!(trans_token(result), "tefWRONG_PRIOR");
}

#[test]
fn vault_invoke_preclaim_source_wrapper_preserves_unknowns_subset() {
    let tx = TestTx {
        txn_type: TxType::PAYMENT,
    };

    let result = run_vault_invoke_preclaim_for_txn_source(
        false,
        &tx,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || 20_u64,
        |_| Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Err(UnknownTransactionType::new(TxType::PAYMENT)));
}
