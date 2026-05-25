//! Current Rust helper mirroring the post-broker-update
//! the LoanSet transactor broker-pseudo `dirLink(...)` wrapper.
//!
//! This module preserves the deterministic behavior around:
//!
//! - invoking the broker-pseudo directory-link attempt exactly
//!   once, and
//! - returning that `TER` unchanged.

use protocol::Ter;

pub fn run_loan_set_do_apply_broker_pseudo_dir_link<DirLink>(dir_link: DirLink) -> Ter
where
    DirLink: FnOnce() -> Ter,
{
    dir_link()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_broker_pseudo_dir_link;

    #[test]
    fn loan_set_do_apply_broker_pseudo_dir_link_returns_success_unchanged() {
        let result = run_loan_set_do_apply_broker_pseudo_dir_link(|| Ter::TES_SUCCESS);

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_broker_pseudo_dir_link_returns_dir_full_unchanged() {
        let result = run_loan_set_do_apply_broker_pseudo_dir_link(|| Ter::TEC_DIR_FULL);

        assert_eq!(result, Ter::TEC_DIR_FULL);
        assert_eq!(trans_token(result), "tecDIR_FULL");
    }

    #[test]
    fn loan_set_do_apply_broker_pseudo_dir_link_calls_the_helper_once() {
        let calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_broker_pseudo_dir_link(|| {
            calls.set(calls.get() + 1);
            Ter::TEC_DIR_FULL
        });

        assert_eq!(result, Ter::TEC_DIR_FULL);
        assert_eq!(calls.get(), 1);
    }
}
