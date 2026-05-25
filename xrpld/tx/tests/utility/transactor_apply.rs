//! Integration tests that pin the narrowed Rust `Transactor::apply()` shell to
//! the current C++ behavior.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use protocol::{Ter, trans_token};
use tx::{TRANSACTOR_APPLY_ASSERT_MESSAGE, run_transactor_apply};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Account {
    balance: i64,
    sequence: u32,
    account_txn_id: Option<&'static str>,
}

#[test]
fn tx_transactor_apply_skips_account_shell_for_zero_account() {
    let result = run_transactor_apply(
        true,
        None::<&mut Account>,
        || {},
        |_| panic!("zero-account apply should skip balance capture"),
        |_| panic!("zero-account apply should skip pre-fee capture"),
        |_| panic!("zero-account apply should skip sequence handling"),
        |_| panic!("zero-account apply should skip fee handling"),
        |_| panic!("zero-account apply should skip account-txn-id inspection"),
        || panic!("zero-account apply should skip tx id extraction"),
        |_, _| panic!("zero-account apply should skip account-txn-id writes"),
        |_| panic!("zero-account apply should skip view update"),
        || Ter::TEC_NO_ENTRY,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn tx_transactor_apply_asserts_missing_nonzero_source_account() {
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = run_transactor_apply(
            false,
            None::<&mut Account>,
            || {},
            |_| panic!("assert should happen before balance capture"),
            |_| panic!("assert should happen before pre-fee capture"),
            |_| panic!("assert should happen before sequence handling"),
            |_| panic!("assert should happen before fee handling"),
            |_| panic!("assert should happen before account-txn-id inspection"),
            || panic!("assert should happen before tx id extraction"),
            |_, _| panic!("assert should happen before account-txn-id writes"),
            |_| panic!("assert should happen before view update"),
            || panic!("assert should happen before doApply"),
        );
    }))
    .expect_err("missing non-zero source account should assert");

    let message = if let Some(message) = panic.downcast_ref::<String>() {
        message.as_str()
    } else if let Some(message) = panic.downcast_ref::<&'static str>() {
        message
    } else {
        panic!("unexpected panic payload");
    };

    assert!(message.contains(TRANSACTOR_APPLY_ASSERT_MESSAGE));
}

#[test]
fn tx_transactor_apply_returns_first_source_account_failure_unchanged() {
    let mut account = Account {
        balance: 50,
        sequence: 7,
        account_txn_id: Some("old"),
    };

    let result = run_transactor_apply(
        false,
        Some(&mut account),
        || {},
        |account| account.balance,
        |balance| assert_eq!(balance, 50),
        |_| Ter::TES_SUCCESS,
        |_| Ter::TEC_INSUFF_FEE,
        |_| panic!("fee failure should skip account-txn-id inspection"),
        || panic!("fee failure should skip tx id extraction"),
        |_, _| panic!("fee failure should skip account-txn-id writes"),
        |_| panic!("fee failure should skip view update"),
        || panic!("fee failure should skip doApply"),
    );

    assert_eq!(result, Ter::TEC_INSUFF_FEE);
    assert_eq!(trans_token(result), "tecINSUFF_FEE");
    assert_eq!(account.account_txn_id, Some("old"));
}

#[test]
fn tx_transactor_apply_runs_account_txn_id_update_then_do_apply() {
    let mut account = Account {
        balance: 50,
        sequence: 7,
        account_txn_id: Some("old"),
    };
    let updated = Cell::new(false);

    let result = run_transactor_apply(
        false,
        Some(&mut account),
        || {},
        |account| account.balance,
        |balance| assert_eq!(balance, 50),
        |account| {
            account.sequence = 8;
            Ter::TES_SUCCESS
        },
        |account| {
            account.balance -= 10;
            Ter::TES_SUCCESS
        },
        |account| account.account_txn_id.is_some(),
        || "tx-1",
        |account, tx_id| account.account_txn_id = Some(tx_id),
        |account| {
            assert_eq!(account.account_txn_id, Some("tx-1"));
            updated.set(true);
        },
        || {
            assert!(updated.get());
            Ter::TEC_CLAIM
        },
    );

    assert_eq!(result, Ter::TEC_CLAIM);
    assert_eq!(trans_token(result), "tecCLAIM");
    assert_eq!(
        account,
        Account {
            balance: 40,
            sequence: 8,
            account_txn_id: Some("tx-1"),
        }
    );
}

#[test]
fn tx_transactor_apply_skips_account_txn_id_update_when_field_is_absent() {
    let trace = std::cell::RefCell::new(Vec::new());
    let mut account = Account {
        balance: 50,
        sequence: 7,
        account_txn_id: None,
    };

    let result = run_transactor_apply(
        false,
        Some(&mut account),
        || trace.borrow_mut().push("pre_compute"),
        |account| account.balance,
        |balance| {
            trace.borrow_mut().push("capture_pre_fee");
            assert_eq!(balance, 50);
        },
        |account| {
            trace.borrow_mut().push("consume_seq_proxy");
            account.sequence = 8;
            Ter::TES_SUCCESS
        },
        |account| {
            trace.borrow_mut().push("pay_fee");
            account.balance -= 10;
            Ter::TES_SUCCESS
        },
        |account| {
            trace.borrow_mut().push("has_account_txn_id");
            account.account_txn_id.is_some()
        },
        || {
            panic!("missing account txn id should skip transaction-id extraction");
        },
        |_, _| {
            panic!("missing account txn id should skip account txn id writes");
        },
        |account| {
            trace.borrow_mut().push("update_account");
            assert_eq!(account.account_txn_id, None);
        },
        || {
            trace.borrow_mut().push("do_apply");
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
    assert_eq!(
        trace.into_inner(),
        vec![
            "pre_compute",
            "capture_pre_fee",
            "consume_seq_proxy",
            "pay_fee",
            "has_account_txn_id",
            "update_account",
            "do_apply"
        ]
    );
    assert_eq!(
        account,
        Account {
            balance: 40,
            sequence: 8,
            account_txn_id: None,
        }
    );
}
