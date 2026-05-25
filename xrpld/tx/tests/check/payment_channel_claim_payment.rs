//! Integration tests that pin the dedicated `sfBalance` payment branch helper
//! to the current `PaymentChannelClaim.cpp` ordering.

use protocol::Ter;
use tx::payment_channel_claim_payment::{
    PaymentChannelClaimPaymentFacts, PaymentChannelClaimPaymentSink,
    run_payment_channel_claim_payment_branch, run_payment_channel_claim_payment_guards,
};

#[derive(Debug, Default)]
struct TestSink {
    verify_deposit_preauth_result: Ter,
    events: Vec<String>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            verify_deposit_preauth_result: Ter::TES_SUCCESS,
            events: Vec::new(),
        }
    }
}

impl PaymentChannelClaimPaymentSink for TestSink {
    fn verify_deposit_preauth(&mut self) -> Ter {
        self.events.push("verify_deposit_preauth".to_string());
        self.verify_deposit_preauth_result
    }

    fn set_channel_balance(&mut self, balance_drops: u64) {
        self.events
            .push(format!("set_channel_balance:{balance_drops}"));
    }

    fn add_destination_balance(&mut self, delta_drops: u64) {
        self.events
            .push(format!("add_destination_balance:{delta_drops}"));
    }

    fn persist_destination_balance(&mut self) {
        self.events.push("persist_destination_balance".to_string());
    }

    fn persist_channel_balance(&mut self) {
        self.events.push("persist_channel_balance".to_string());
    }
}

fn base_facts() -> PaymentChannelClaimPaymentFacts {
    PaymentChannelClaimPaymentFacts {
        tx_account_is_destination: false,
        signature_present: false,
        provided_public_key_matches_channel: true,
        requested_balance_exceeds_channel_funds: false,
        requested_balance_not_above_channel_balance: false,
        channel_balance_drops: 250,
        requested_balance_drops: 400,
    }
}

#[test]
fn payment_branch_returns_bad_signature_before_any_other_guard() {
    let sink = TestSink::new();

    let result = run_payment_channel_claim_payment_guards(PaymentChannelClaimPaymentFacts {
        tx_account_is_destination: true,
        ..base_facts()
    });

    assert_eq!(result, Ter::TEM_BAD_SIGNATURE);
    assert!(sink.events.is_empty());
}

#[test]
fn payment_branch_returns_bad_signer_after_public_key_check() {
    let sink = TestSink::new();

    let result = run_payment_channel_claim_payment_guards(PaymentChannelClaimPaymentFacts {
        signature_present: true,
        provided_public_key_matches_channel: false,
        requested_balance_exceeds_channel_funds: true,
        requested_balance_not_above_channel_balance: true,
        ..base_facts()
    });

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
    assert!(sink.events.is_empty());
}

#[test]
fn payment_branch_returns_unfunded_payment_when_requested_balance_exceeds_channel_funds() {
    let sink = TestSink::new();

    let result = run_payment_channel_claim_payment_guards(PaymentChannelClaimPaymentFacts {
        requested_balance_exceeds_channel_funds: true,
        ..base_facts()
    });

    assert_eq!(result, Ter::TEC_UNFUNDED_PAYMENT);
    assert!(sink.events.is_empty());
}

#[test]
fn payment_branch_returns_unfunded_payment_when_requested_balance_is_not_above_paid_amount() {
    let sink = TestSink::new();

    let result = run_payment_channel_claim_payment_guards(PaymentChannelClaimPaymentFacts {
        requested_balance_not_above_channel_balance: true,
        ..base_facts()
    });

    assert_eq!(result, Ter::TEC_UNFUNDED_PAYMENT);
    assert!(sink.events.is_empty());
}

#[test]
fn payment_branch_passthroughs_deposit_preauth_failure_before_update() {
    let mut sink = TestSink::new();
    sink.verify_deposit_preauth_result = Ter::TEC_NO_AUTH;

    let result = run_payment_channel_claim_payment_branch(base_facts(), &mut sink);

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(sink.events, ["verify_deposit_preauth"]);
}

#[test]
fn payment_branch_success_runs_cpp_mutation_order() {
    let mut sink = TestSink::new();

    let result = run_payment_channel_claim_payment_branch(base_facts(), &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "verify_deposit_preauth",
            "set_channel_balance:400",
            "add_destination_balance:150",
            "persist_destination_balance",
            "persist_channel_balance"
        ]
    );
}
