//! Integration tests that pin the shared payment-channel due-time helper to
//! the current C++ `PaymentChannelClaim.cpp` and `PaymentChannelFund.cpp`
//! behavior.

use tx::payment_channel_due::{PaymentChannelDueFacts, is_payment_channel_due};

#[test]
fn payment_channel_due_when_cancel_after_is_reached() {
    assert!(is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: Some(20_u32),
        expiration: None,
        close_time: 20,
    }));
    assert!(is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: Some(20_u32),
        expiration: Some(40),
        close_time: 21,
    }));
}

#[test]
fn payment_channel_due_when_expiration_is_reached() {
    assert!(is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: None,
        expiration: Some(20_u32),
        close_time: 20,
    }));
    assert!(is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: Some(30_u32),
        expiration: Some(20),
        close_time: 25,
    }));
}

#[test]
fn payment_channel_due_when_only_the_future_deadline_exists() {
    assert!(!is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: Some(20_u32),
        expiration: None,
        close_time: 19,
    }));
    assert!(!is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: None,
        expiration: Some(20_u32),
        close_time: 19,
    }));
}

#[test]
fn payment_channel_due_when_no_deadlines_are_present() {
    assert!(!is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: None,
        expiration: None,
        close_time: 20_u32,
    }));
}
