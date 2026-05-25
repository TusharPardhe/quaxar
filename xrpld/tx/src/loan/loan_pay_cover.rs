//! Current Rust helper mirroring the the reference implementation broker-fee destination
//! decision.
//!
//! This preserves the deterministic short-circuit decision:
//!
//! - send the broker fee to the owner only when cover is sufficient,
//! - otherwise fall back to the broker pseudo-account when the owner is
//!   deep-frozen or lacks strong auth.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoanPayBrokerFeeDestinationFacts {
    pub cover_is_sufficient: bool,
    pub owner_is_deep_frozen: bool,
    pub owner_requires_auth: bool,
}

pub fn decide_loan_pay_broker_fee_destination(facts: LoanPayBrokerFeeDestinationFacts) -> bool {
    facts.cover_is_sufficient && !facts.owner_is_deep_frozen && !facts.owner_requires_auth
}

#[cfg(test)]
mod tests {
    use super::{LoanPayBrokerFeeDestinationFacts, decide_loan_pay_broker_fee_destination};

    #[test]
    fn loan_pay_cover_sends_fee_to_owner_only_when_all_checks_pass() {
        assert!(decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: true,
                owner_is_deep_frozen: false,
                owner_requires_auth: false,
            }
        ));
    }

    #[test]
    fn loan_pay_cover_rejects_frozen_or_unauthorized_owner() {
        assert!(!decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: true,
                owner_is_deep_frozen: true,
                owner_requires_auth: false,
            }
        ));
        assert!(!decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: true,
                owner_is_deep_frozen: false,
                owner_requires_auth: true,
            }
        ));
    }

    #[test]
    fn loan_pay_cover_requires_sufficient_cover() {
        assert!(!decide_loan_pay_broker_fee_destination(
            LoanPayBrokerFeeDestinationFacts {
                cover_is_sufficient: false,
                owner_is_deep_frozen: false,
                owner_requires_auth: false,
            }
        ));
    }
}
