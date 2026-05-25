//! Final success tail for the reference implementation.
//!
//! This module ports the deterministic outer success result once the earlier
//! preclaim checks have already passed.

use protocol::Ter;

pub fn run_loan_set_preclaim_success() -> Ter {
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::run_loan_set_preclaim_success;

    #[test]
    fn loan_set_preclaim_success_returns_tes_success() {
        let result = run_loan_set_preclaim_success();

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
    }
}
