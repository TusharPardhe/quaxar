//! Payment transactor dispatch and logic.

use crate::{ApplyResult, UnknownTransactionType};
use protocol::TxType;

pub fn run_payment_invoke_apply(
    txn_type: TxType,
    run_payment_do_apply: impl FnOnce() -> Result<ApplyResult, UnknownTransactionType<TxType>>,
) -> Result<ApplyResult, UnknownTransactionType<TxType>> {
    if txn_type == TxType::PAYMENT {
        run_payment_do_apply()
    } else {
        Err(UnknownTransactionType::new(txn_type))
    }
}
