//! the reference implementation compatibility surface.
//!
//! This ports the exact current deterministic behavior around:
//!
//! - `preflight(...)` rejecting a zero `sfOfferSequence`,
//! - `preclaim(...)` checking account existence and current sequence ordering,
//! - and the `doApply()` owner lookup plus optional offer-delete shell.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OfferCancelPreclaimFacts {
    pub account_exists: bool,
    pub account_sequence: u32,
    pub offer_sequence: u32,
}

pub trait OfferCancelApplySink {
    fn account_exists(&mut self) -> bool;
    fn offer_exists(&mut self) -> bool;
    fn delete_offer(&mut self) -> Ter;
}

pub fn run_offer_cancel_preflight(offer_sequence: u32) -> NotTec {
    if offer_sequence == 0 {
        return Ter::TEM_BAD_SEQUENCE;
    }

    Ter::TES_SUCCESS
}

pub fn run_offer_cancel_preclaim(facts: OfferCancelPreclaimFacts) -> Ter {
    if !facts.account_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if facts.account_sequence <= facts.offer_sequence {
        return Ter::TEM_BAD_SEQUENCE;
    }

    Ter::TES_SUCCESS
}

pub fn run_offer_cancel_do_apply<S: OfferCancelApplySink>(sink: &mut S) -> Ter {
    if !sink.account_exists() {
        return Ter::TEF_INTERNAL;
    }

    if sink.offer_exists() {
        return sink.delete_offer();
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        OfferCancelApplySink, OfferCancelPreclaimFacts, run_offer_cancel_do_apply,
        run_offer_cancel_preclaim, run_offer_cancel_preflight,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestApplySink {
        account_exists: bool,
        offer_exists: bool,
        delete_result: Ter,
        events: Vec<String>,
    }

    impl TestApplySink {
        fn new() -> Self {
            Self {
                account_exists: true,
                offer_exists: true,
                delete_result: Ter::TES_SUCCESS,
                events: Vec::new(),
            }
        }
    }

    impl OfferCancelApplySink for TestApplySink {
        fn account_exists(&mut self) -> bool {
            self.events.push("account".to_string());
            self.account_exists
        }

        fn offer_exists(&mut self) -> bool {
            self.events.push("offer".to_string());
            self.offer_exists
        }

        fn delete_offer(&mut self) -> Ter {
            self.events.push("delete".to_string());
            self.delete_result
        }
    }

    #[test]
    fn offer_cancel_preflight_rejects_zero_offer_sequence() {
        let result = run_offer_cancel_preflight(0);

        assert_eq!(result, Ter::TEM_BAD_SEQUENCE);
        assert_eq!(trans_token(result), "temBAD_SEQUENCE");
    }

    #[test]
    fn offer_cancel_preflight_accepts_non_zero_offer_sequence() {
        assert_eq!(run_offer_cancel_preflight(1), Ter::TES_SUCCESS);
    }

    #[test]
    fn offer_cancel_preclaim_requires_account() {
        let result = run_offer_cancel_preclaim(OfferCancelPreclaimFacts {
            account_exists: false,
            account_sequence: 9,
            offer_sequence: 8,
        });

        assert_eq!(result, Ter::TER_NO_ACCOUNT);
        assert_eq!(trans_token(result), "terNO_ACCOUNT");
    }

    #[test]
    fn offer_cancel_preclaim_rejects_current_or_future_offer_sequence() {
        let equal = run_offer_cancel_preclaim(OfferCancelPreclaimFacts {
            account_exists: true,
            account_sequence: 7,
            offer_sequence: 7,
        });
        let future = run_offer_cancel_preclaim(OfferCancelPreclaimFacts {
            account_exists: true,
            account_sequence: 7,
            offer_sequence: 8,
        });

        assert_eq!(equal, Ter::TEM_BAD_SEQUENCE);
        assert_eq!(future, Ter::TEM_BAD_SEQUENCE);
    }

    #[test]
    fn offer_cancel_preclaim_accepts_strictly_prior_offer_sequence() {
        let result = run_offer_cancel_preclaim(OfferCancelPreclaimFacts {
            account_exists: true,
            account_sequence: 7,
            offer_sequence: 6,
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn offer_cancel_do_apply_returns_tef_internal_for_missing_account() {
        let mut sink = TestApplySink::new();
        sink.account_exists = false;

        let result = run_offer_cancel_do_apply(&mut sink);

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(sink.events, ["account"]);
    }

    #[test]
    fn offer_cancel_do_apply_returns_success_when_offer_is_absent() {
        let mut sink = TestApplySink::new();
        sink.offer_exists = false;

        let result = run_offer_cancel_do_apply(&mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.events, ["account", "offer"]);
    }

    #[test]
    fn offer_cancel_do_apply_delegates_offer_delete_result() {
        let mut sink = TestApplySink::new();
        sink.delete_result = Ter::TEF_BAD_LEDGER;

        let result = run_offer_cancel_do_apply(&mut sink);

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(result), "tefBAD_LEDGER");
        assert_eq!(sink.events, ["account", "offer", "delete"]);
    }
}
