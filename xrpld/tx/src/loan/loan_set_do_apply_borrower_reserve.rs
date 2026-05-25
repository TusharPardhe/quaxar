//! Current Rust helper mirroring the isolated
//! the LoanSet transactor borrower-reserve guard.
//!
//! This module preserves the deterministic behavior around:
//!
//! - reading the already-updated borrower owner count,
//! - using `preFeeBalance_` only when `account_ == borrower`,
//! - otherwise using the borrower account's ledger XRP balance, and
//! - rejecting only when that chosen balance is strictly below the computed
//!   account reserve.

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetDoApplyBorrowerReserveFailure {
    InsufficientReserve,
}

impl LoanSetDoApplyBorrowerReserveFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::InsufficientReserve => Ter::TEC_INSUFFICIENT_RESERVE,
        }
    }
}

pub fn check_loan_set_do_apply_borrower_reserve<Balance, OwnerCount, ComputeAccountReserve>(
    account_is_borrower: bool,
    pre_fee_balance: &Balance,
    borrower_balance: &Balance,
    owner_count: OwnerCount,
    compute_account_reserve: ComputeAccountReserve,
) -> Result<(), LoanSetDoApplyBorrowerReserveFailure>
where
    Balance: PartialOrd,
    OwnerCount: Copy,
    ComputeAccountReserve: FnOnce(OwnerCount) -> Balance,
{
    let balance = if account_is_borrower {
        pre_fee_balance
    } else {
        borrower_balance
    };
    let required_reserve = compute_account_reserve(owner_count);

    if balance < &required_reserve {
        return Err(LoanSetDoApplyBorrowerReserveFailure::InsufficientReserve);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::trans_token;

    use super::{LoanSetDoApplyBorrowerReserveFailure, check_loan_set_do_apply_borrower_reserve};

    #[test]
    fn loan_set_do_apply_borrower_reserve_uses_pre_fee_balance_for_borrower_signed_tx() {
        let result =
            check_loan_set_do_apply_borrower_reserve(true, &30_u32, &1_u32, 4_u32, |_| 30_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_borrower_reserve_uses_borrower_ledger_balance_for_counterparty() {
        let result =
            check_loan_set_do_apply_borrower_reserve(false, &100_u32, &29_u32, 4_u32, |_| 30_u32);

        assert_eq!(
            result,
            Err(LoanSetDoApplyBorrowerReserveFailure::InsufficientReserve)
        );
    }

    #[test]
    fn loan_set_do_apply_borrower_reserve_allows_exact_reserve_equality() {
        let result =
            check_loan_set_do_apply_borrower_reserve(false, &1_u32, &30_u32, 4_u32, |_| 30_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_borrower_reserve_returns_insufficient_reserve() {
        let result =
            check_loan_set_do_apply_borrower_reserve(true, &29_u32, &100_u32, 4_u32, |_| 30_u32);

        assert_eq!(
            result,
            Err(LoanSetDoApplyBorrowerReserveFailure::InsufficientReserve)
        );
        let err = result.expect_err("borrower reserve shortage should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(trans_token(err.ter()), "tecINSUFFICIENT_RESERVE");
    }

    #[test]
    fn loan_set_do_apply_borrower_reserve_computes_reserve_once_from_updated_owner_count() {
        let seen = RefCell::new(Vec::new());

        let result = check_loan_set_do_apply_borrower_reserve(
            true,
            &29_u32,
            &100_u32,
            4_u32,
            |owner_count| {
                seen.borrow_mut().push(owner_count);
                30_u32
            },
        );

        assert_eq!(
            result,
            Err(LoanSetDoApplyBorrowerReserveFailure::InsufficientReserve)
        );
        assert_eq!(*seen.borrow(), vec![4_u32]);
    }
}
