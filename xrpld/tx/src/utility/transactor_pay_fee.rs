//! Current Rust helper mirroring `Transactor::payFee()`.
//!
//! This module preserves the exact current fee-payment behavior:
//!
//! - read the fee paid from the transaction,
//! - load the fee payer account from the current view,
//! - return `tefINTERNAL` if the fee payer account is unexpectedly missing,
//! - deduct the fee from the fee payer balance,
//! - immediately update the view only when the fee payer differs from the
//!   source account.

use std::ops::Sub;

use protocol::Ter;

pub trait TransactorPayFeeTx {
    type AccountId;
    type Amount;

    fn fee_paid(&self) -> Self::Amount;
    fn fee_payer(&self) -> Self::AccountId;
}

pub fn run_transactor_pay_fee<
    Tx,
    AccountState,
    PeekAccount,
    AccountBalance,
    SetAccountBalance,
    UpdateAccount,
>(
    tx: &Tx,
    account: &Tx::AccountId,
    mut peek_account: PeekAccount,
    mut account_balance: AccountBalance,
    mut set_account_balance: SetAccountBalance,
    mut update_account: UpdateAccount,
) -> Result<AccountState, Ter>
where
    Tx: TransactorPayFeeTx,
    Tx::AccountId: PartialEq,
    Tx::Amount: Copy + Sub<Output = Tx::Amount>,
    PeekAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    AccountBalance: FnMut(&AccountState) -> Tx::Amount,
    SetAccountBalance: FnMut(&mut AccountState, Tx::Amount),
    UpdateAccount: FnMut(&AccountState),
{
    let fee_paid = tx.fee_paid();
    let fee_payer = tx.fee_payer();
    let Some(mut account_state) = peek_account(&fee_payer) else {
        return Err(Ter::TEF_INTERNAL);
    };

    let new_balance = account_balance(&account_state) - fee_paid;
    set_account_balance(&mut account_state, new_balance);

    if &fee_payer != account {
        update_account(&account_state);
    }

    Ok(account_state)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{Ter, trans_token};

    use super::{TransactorPayFeeTx, run_transactor_pay_fee};

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
    fn transactor_pay_fee_returns_internal_when_fee_payer_is_missing() {
        let update_called = Cell::new(false);

        let result = run_transactor_pay_fee(
            &FeeTx {
                fee_paid: 10,
                fee_payer: "alice",
            },
            &"alice",
            |_| None::<Account>,
            |_| panic!("missing payer should skip balance read"),
            |_, _| panic!("missing payer should skip balance update"),
            |_| update_called.set(true),
        );

        assert_eq!(result, Err(Ter::TEF_INTERNAL));
        assert_eq!(trans_token(result.unwrap_err()), "tefINTERNAL");
        assert!(!update_called.get());
    }

    #[test]
    fn transactor_pay_fee_deducts_source_balance_without_immediate_update() {
        let update_called = Cell::new(false);

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
            |_| update_called.set(true),
        );

        assert_eq!(result.unwrap(), Account { balance: 40 });
        assert!(!update_called.get());
    }

    #[test]
    fn transactor_pay_fee_updates_delegate_fee_payer_immediately() {
        let updates = RefCell::new(Vec::new());

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
            |account| updates.borrow_mut().push(account.balance),
        );

        assert_eq!(result.unwrap(), Account { balance: 75 });
        assert_eq!(updates.into_inner(), vec![75]);
    }
}
