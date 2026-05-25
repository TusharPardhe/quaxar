//! Integration tests that pin the dedicated `PaymentChannelClaim.cpp` renew
//! and close-settle tail to the current C++ ordering.

use protocol::Ter;
use tx::payment_channel_claim_settle::{
    PaymentChannelClaimSettleFacts, PaymentChannelClaimSettleSink, run_payment_channel_claim_settle,
};

#[derive(Debug)]
struct TestSink {
    close_result: Ter,
    events: Vec<String>,
    expirations: Vec<u32>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            close_result: Ter::TES_SUCCESS,
            events: Vec::new(),
            expirations: Vec::new(),
        }
    }
}

impl PaymentChannelClaimSettleSink<u32> for TestSink {
    fn clear_expiration(&mut self) {
        self.events.push("clear_expiration".to_string());
    }

    fn set_expiration(&mut self, expiration: u32) {
        self.events.push("set_expiration".to_string());
        self.expirations.push(expiration);
    }

    fn close_channel(&mut self) -> Ter {
        self.events.push("close_channel".to_string());
        self.close_result
    }
}

fn facts() -> PaymentChannelClaimSettleFacts<u32> {
    PaymentChannelClaimSettleFacts {
        tx_account_is_source: true,
        renew_flag: false,
        close_flag: false,
        tx_account_is_destination: false,
        channel_fully_paid: false,
        current_expiration: None,
        close_time: 10,
        settle_delay: 20,
    }
}

#[test]
fn renew_clears_expiration_only_for_source() {
    let mut sink = TestSink::new();

    let result = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            renew_flag: true,
            ..facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["clear_expiration"]);

    let mut denied_sink = TestSink::new();
    let denied = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            tx_account_is_source: false,
            renew_flag: true,
            ..facts()
        },
        &mut denied_sink,
    );

    assert_eq!(denied, Ter::TEC_NO_PERMISSION);
    assert!(denied_sink.events.is_empty());
}

#[test]
fn close_short_circuits_through_callback_for_destination_or_paid() {
    let mut destination_sink = TestSink::new();
    destination_sink.close_result = Ter::TEC_NO_PERMISSION;

    let destination_result = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            close_flag: true,
            tx_account_is_destination: true,
            ..facts()
        },
        &mut destination_sink,
    );

    assert_eq!(destination_result, Ter::TEC_NO_PERMISSION);
    assert_eq!(destination_sink.events, ["close_channel"]);

    let mut paid_sink = TestSink::new();
    paid_sink.close_result = Ter::TEF_INTERNAL;

    let paid_result = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            close_flag: true,
            channel_fully_paid: true,
            ..facts()
        },
        &mut paid_sink,
    );

    assert_eq!(paid_result, Ter::TEF_INTERNAL);
    assert_eq!(paid_sink.events, ["close_channel"]);
}

#[test]
fn close_sets_settle_expiration_only_when_needed() {
    let mut sink = TestSink::new();

    let result = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            close_flag: true,
            current_expiration: None,
            ..facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["set_expiration"]);
    assert_eq!(sink.expirations, [30]);

    let mut greater_sink = TestSink::new();
    let greater_result = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            close_flag: true,
            current_expiration: Some(31),
            ..facts()
        },
        &mut greater_sink,
    );

    assert_eq!(greater_result, Ter::TES_SUCCESS);
    assert_eq!(greater_sink.events, ["set_expiration"]);
    assert_eq!(greater_sink.expirations, [30]);

    let mut lower_sink = TestSink::new();
    let lower_result = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            close_flag: true,
            current_expiration: Some(30),
            ..facts()
        },
        &mut lower_sink,
    );

    assert_eq!(lower_result, Ter::TES_SUCCESS);
    assert!(lower_sink.events.is_empty());
    assert!(lower_sink.expirations.is_empty());
}

#[test]
fn renew_happens_before_close() {
    let mut sink = TestSink::new();

    let result = run_payment_channel_claim_settle(
        PaymentChannelClaimSettleFacts {
            renew_flag: true,
            close_flag: true,
            current_expiration: Some(31),
            ..facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["clear_expiration", "set_expiration"]);
    assert_eq!(sink.expirations, [30]);
}
