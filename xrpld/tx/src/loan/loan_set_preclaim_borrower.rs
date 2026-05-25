//! Borrower-account existence branch for the reference implementation.
//!
//! This module ports the deterministic behavior around:
//!
//! - attempting exactly one borrower account lookup, and
//! - mapping a missing borrower to `terNO_ACCOUNT` with the current warning
//!   text.

use protocol::Ter;

pub const LOAN_SET_BORROWER_DOES_NOT_EXIST_WARNING: &str = "Borrower does not exist.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetPreclaimBorrowerFailure {
    BorrowerDoesNotExist,
}

impl LoanSetPreclaimBorrowerFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::BorrowerDoesNotExist => Ter::TER_NO_ACCOUNT,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::BorrowerDoesNotExist => LOAN_SET_BORROWER_DOES_NOT_EXIST_WARNING,
        }
    }
}

pub fn check_loan_set_preclaim_borrower<BorrowerId, Borrower, ReadBorrower>(
    borrower: &BorrowerId,
    read_borrower: ReadBorrower,
) -> Result<Borrower, LoanSetPreclaimBorrowerFailure>
where
    ReadBorrower: FnOnce(&BorrowerId) -> Option<Borrower>,
{
    read_borrower(borrower).ok_or(LoanSetPreclaimBorrowerFailure::BorrowerDoesNotExist)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, trans_token};

    use super::{
        LOAN_SET_BORROWER_DOES_NOT_EXIST_WARNING, LoanSetPreclaimBorrowerFailure,
        check_loan_set_preclaim_borrower,
    };

    #[test]
    fn loan_set_preclaim_borrower_returns_loaded_account_unchanged() {
        let result = check_loan_set_preclaim_borrower(&"borrower", |borrower| {
            assert_eq!(*borrower, "borrower");
            Some("borrower-sle")
        });

        assert_eq!(result, Ok("borrower-sle"));
    }

    #[test]
    fn loan_set_preclaim_borrower_returns_no_account_when_missing() {
        let result =
            check_loan_set_preclaim_borrower(&"missing-borrower", |_| None::<&'static str>);

        assert_eq!(
            result,
            Err(LoanSetPreclaimBorrowerFailure::BorrowerDoesNotExist)
        );
        let err = result.unwrap_err();
        assert_eq!(err.ter(), Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(err.ter()), "terNO_ACCOUNT");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_BORROWER_DOES_NOT_EXIST_WARNING
        );
    }

    #[test]
    fn loan_set_preclaim_borrower_reads_account_exactly_once() {
        let seen = RefCell::new(Vec::new());

        let result = check_loan_set_preclaim_borrower(&"borrower", |borrower| {
            seen.borrow_mut().push(*borrower);
            Some("borrower-sle")
        });

        assert_eq!(result, Ok("borrower-sle"));
        assert_eq!(*seen.borrow(), vec!["borrower"]);
    }
}
