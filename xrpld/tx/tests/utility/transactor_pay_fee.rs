//! Integration tests that pin the narrowed Rust `Transactor::payFee()` shell
//! to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{TransactorPayFeeTx, run_transactor_pay_fee};

struct FeeTx {
    fee_paid: i64,
    fee_payer: &'static str,
}

impl TransactorPayFeeTx for FeeTx {
    type AccountId = &'static str;
    type Amount = i64;

    fn fee_paid(&self) -> Self::Amount {
        self.fee_paid
    }

    fn fee_payer(&self) -> Self::AccountId {
        self.fee_payer
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Account {
    balance: i64,
}

#[test]
fn tx_transactor_pay_fee_returns_internal_when_fee_payer_is_missing() {
    let result = run_transactor_pay_fee(
        &FeeTx {
            fee_paid: 10,
            fee_payer: "alice",
        },
        &"alice",
        |_| None::<Account>,
        |_| panic!("missing payer should skip balance read"),
        |_, _| panic!("missing payer should skip balance update"),
        |_| panic!("missing payer should skip view update"),
    );

    assert_eq!(result, Err(Ter::TEF_INTERNAL));
    assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
}

#[test]
fn tx_transactor_pay_fee_deducts_source_balance_without_immediate_update() {
    let result = run_transactor_pay_fee(
        &FeeTx {
            fee_paid: 10,
            fee_payer: "alice",
        },
        &"alice",
        |account| {
            assert_eq!(*account, "alice");
            Some(Account { balance: 50 })
        },
        |account| account.balance,
        |account, new_balance| account.balance = new_balance,
        |_| panic!("source-account payer should skip immediate update"),
    );

    assert_eq!(result.unwrap(), Account { balance: 40 });
}

#[test]
fn tx_transactor_pay_fee_updates_delegate_fee_payer_immediately() {
    let mut updated = Vec::new();

    let result = run_transactor_pay_fee(
        &FeeTx {
            fee_paid: 15,
            fee_payer: "delegate",
        },
        &"source",
        |account| {
            assert_eq!(*account, "delegate");
            Some(Account { balance: 90 })
        },
        |account| account.balance,
        |account, new_balance| account.balance = new_balance,
        |account| updated.push(account.balance),
    );

    assert_eq!(result.unwrap(), Account { balance: 75 });
    assert_eq!(updated, vec![75]);
}
