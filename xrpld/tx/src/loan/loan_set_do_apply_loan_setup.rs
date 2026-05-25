//! Post-transfer loan-creation setup facts for the LoanSet transactor.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - reading `getStartDate(view)` first,
//! - then reading `brokerSle->at(sfLoanSequence)`, and
//! - returning those two values unchanged for the following loan-creation step.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetDoApplyLoanSetup<StartDate, LoanSequence> {
    pub start_date: StartDate,
    pub loan_sequence: LoanSequence,
}

pub fn load_loan_set_do_apply_loan_setup<StartDate, LoanSequence, GetStartDate, GetLoanSequence>(
    get_start_date: GetStartDate,
    get_loan_sequence: GetLoanSequence,
) -> LoanSetDoApplyLoanSetup<StartDate, LoanSequence>
where
    GetStartDate: FnOnce() -> StartDate,
    GetLoanSequence: FnOnce() -> LoanSequence,
{
    let start_date = get_start_date();
    let loan_sequence = get_loan_sequence();

    LoanSetDoApplyLoanSetup {
        start_date,
        loan_sequence,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::{LoanSetDoApplyLoanSetup, load_loan_set_do_apply_loan_setup};

    #[test]
    fn loan_set_do_apply_loan_setup_reads_start_date_before_loan_sequence() {
        let steps = RefCell::new(Vec::new());

        let result = load_loan_set_do_apply_loan_setup(
            || {
                steps.borrow_mut().push("start_date");
                123_u32
            },
            || {
                steps.borrow_mut().push("loan_sequence");
                7_u32
            },
        );

        assert_eq!(
            result,
            LoanSetDoApplyLoanSetup {
                start_date: 123_u32,
                loan_sequence: 7_u32,
            }
        );
        assert_eq!(steps.into_inner(), vec!["start_date", "loan_sequence"]);
    }

    #[test]
    fn loan_set_do_apply_loan_setup_reads_each_value_once() {
        let start_date_calls = Cell::new(0_u32);
        let loan_sequence_calls = Cell::new(0_u32);

        let result = load_loan_set_do_apply_loan_setup(
            || {
                start_date_calls.set(start_date_calls.get() + 1);
                55_u32
            },
            || {
                loan_sequence_calls.set(loan_sequence_calls.get() + 1);
                9_u32
            },
        );

        assert_eq!(result.start_date, 55_u32);
        assert_eq!(result.loan_sequence, 9_u32);
        assert_eq!(start_date_calls.get(), 1);
        assert_eq!(loan_sequence_calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_loan_setup_keeps_zero_sequence_unchanged() {
        let result = load_loan_set_do_apply_loan_setup(|| 88_u32, || 0_u32);

        assert_eq!(
            result,
            LoanSetDoApplyLoanSetup {
                start_date: 88_u32,
                loan_sequence: 0_u32,
            }
        );
    }
}
