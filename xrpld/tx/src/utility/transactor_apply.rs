//! Current Rust helper mirroring `Transactor::apply()`.
//!
//! This module preserves the exact current outer sequencing:
//!
//! - always run `preCompute()` first,
//! - assert that a missing source account is only allowed for the zero-account
//!   transactor case,
//! - when the source account exists, capture `preFeeBalance_` before any
//!   sequence or fee mutations,
//! - return the first non-success result from `consumeSeqProxy(...)` or
//!   `payFee()` unchanged,
//! - only stamp `sfAccountTxnID` when the source account already has that
//!   field, and
//! - always finish with `doApply()` after the source-account shell succeeds.

use protocol::{Ter, is_tes_success};

pub const TRANSACTOR_APPLY_ASSERT_MESSAGE: &str =
    "xrpl::Transactor::apply : non-null SLE or zero account";

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_apply<AccountState, Balance, TxId>(
    account_is_zero: bool,
    account_state: Option<&mut AccountState>,
    pre_compute: impl FnOnce(),
    account_balance: impl FnOnce(&AccountState) -> Balance,
    set_pre_fee_balance: impl FnOnce(Balance),
    consume_seq_proxy: impl FnOnce(&mut AccountState) -> Ter,
    pay_fee: impl FnOnce(&mut AccountState) -> Ter,
    has_account_txn_id: impl FnOnce(&AccountState) -> bool,
    transaction_id: impl FnOnce() -> TxId,
    set_account_txn_id: impl FnOnce(&mut AccountState, TxId),
    update_account: impl FnOnce(&AccountState),
    do_apply: impl FnOnce() -> Ter,
) -> Ter {
    pre_compute();

    assert!(
        account_state.is_some() || account_is_zero,
        "{TRANSACTOR_APPLY_ASSERT_MESSAGE}"
    );

    if let Some(account_state) = account_state {
        let pre_fee_balance = account_balance(account_state);
        set_pre_fee_balance(pre_fee_balance);

        let result = consume_seq_proxy(account_state);
        if !is_tes_success(result) {
            return result;
        }

        let result = pay_fee(account_state);
        if !is_tes_success(result) {
            return result;
        }

        if has_account_txn_id(account_state) {
            let tx_id = transaction_id();
            set_account_txn_id(account_state, tx_id);
        }

        update_account(account_state);
    }

    do_apply()
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        panic::{AssertUnwindSafe, catch_unwind},
    };

    use protocol::{Ter, trans_token};

    use super::{TRANSACTOR_APPLY_ASSERT_MESSAGE, run_transactor_apply};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Account {
        balance: i64,
        sequence: u32,
        account_txn_id: Option<&'static str>,
    }

    #[test]
    fn transactor_apply_skips_account_shell_for_zero_account() {
        let trace = RefCell::new(Vec::new());

        let result = run_transactor_apply(
            true,
            None::<&mut Account>,
            || trace.borrow_mut().push("pre_compute"),
            |_| panic!("zero-account apply should skip balance capture"),
            |_| panic!("zero-account apply should skip pre-fee state"),
            |_| panic!("zero-account apply should skip sequence handling"),
            |_| panic!("zero-account apply should skip fee handling"),
            |_| panic!("zero-account apply should skip account-txn-id inspection"),
            || panic!("zero-account apply should skip tx id extraction"),
            |_, _| panic!("zero-account apply should skip account-txn-id writes"),
            |_| panic!("zero-account apply should skip view update"),
            || {
                trace.borrow_mut().push("do_apply");
                Ter::TEC_NO_ENTRY
            },
        );

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
        assert_eq!(trace.into_inner(), vec!["pre_compute", "do_apply"]);
    }

    #[test]
    fn transactor_apply_asserts_missing_nonzero_source_account() {
        let pre_compute_called = Cell::new(false);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = run_transactor_apply(
                false,
                None::<&mut Account>,
                || pre_compute_called.set(true),
                |_| panic!("assert should happen before balance capture"),
                |_| panic!("assert should happen before pre-fee state"),
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

        assert!(pre_compute_called.get());
        assert!(message.contains(TRANSACTOR_APPLY_ASSERT_MESSAGE));
    }

    #[test]
    fn transactor_apply_returns_sequence_failure_before_fee() {
        let trace = RefCell::new(Vec::new());
        let mut account = Account {
            balance: 50,
            sequence: 7,
            account_txn_id: Some("old"),
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
                Ter::TER_NO_ACCOUNT
            },
            |_| {
                trace.borrow_mut().push("pay_fee");
                Ter::TES_SUCCESS
            },
            |_| {
                trace.borrow_mut().push("has_account_txn_id");
                true
            },
            || {
                trace.borrow_mut().push("transaction_id");
                "tx-1"
            },
            |_, _| trace.borrow_mut().push("set_account_txn_id"),
            |_| trace.borrow_mut().push("update_account"),
            || {
                trace.borrow_mut().push("do_apply");
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
        assert_eq!(
            trace.into_inner(),
            vec!["pre_compute", "capture_pre_fee", "consume_seq_proxy"]
        );
        assert_eq!(account.sequence, 8);
        assert_eq!(account.account_txn_id, Some("old"));
    }

    #[test]
    fn transactor_apply_returns_fee_failure_before_account_txn_id_update() {
        let trace = RefCell::new(Vec::new());
        let mut account = Account {
            balance: 50,
            sequence: 7,
            account_txn_id: Some("old"),
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
                Ter::TEC_INSUFF_FEE
            },
            |_| {
                trace.borrow_mut().push("has_account_txn_id");
                true
            },
            || {
                trace.borrow_mut().push("transaction_id");
                "tx-1"
            },
            |_, _| trace.borrow_mut().push("set_account_txn_id"),
            |_| trace.borrow_mut().push("update_account"),
            || {
                trace.borrow_mut().push("do_apply");
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_INSUFF_FEE);
        assert_eq!(trans_token(result), "tecINSUFF_FEE");
        assert_eq!(
            trace.into_inner(),
            vec![
                "pre_compute",
                "capture_pre_fee",
                "consume_seq_proxy",
                "pay_fee"
            ]
        );
        assert_eq!(account.balance, 40);
        assert_eq!(account.account_txn_id, Some("old"));
    }

    #[test]
    fn transactor_apply_updates_account_txn_id_and_runs_do_apply_after_success() {
        let trace = RefCell::new(Vec::new());
        let mut account = Account {
            balance: 50,
            sequence: 7,
            account_txn_id: Some("old"),
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
                trace.borrow_mut().push("transaction_id");
                "tx-1"
            },
            |account, tx_id| {
                trace.borrow_mut().push("set_account_txn_id");
                account.account_txn_id = Some(tx_id);
            },
            |account| {
                trace
                    .borrow_mut()
                    .push(if account.account_txn_id == Some("tx-1") {
                        "update_account"
                    } else {
                        "update_account_before_txn_id"
                    });
            },
            || {
                trace.borrow_mut().push("do_apply");
                Ter::TEC_CLAIM
            },
        );

        assert_eq!(result, Ter::TEC_CLAIM);
        assert_eq!(trans_token(result), "tecCLAIM");
        assert_eq!(
            trace.into_inner(),
            vec![
                "pre_compute",
                "capture_pre_fee",
                "consume_seq_proxy",
                "pay_fee",
                "has_account_txn_id",
                "transaction_id",
                "set_account_txn_id",
                "update_account",
                "do_apply"
            ]
        );
        assert_eq!(
            account,
            Account {
                balance: 40,
                sequence: 8,
                account_txn_id: Some("tx-1"),
            }
        );
    }
}
