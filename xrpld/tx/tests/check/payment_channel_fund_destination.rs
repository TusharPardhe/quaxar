//! Integration tests that pin the narrow destination-existence gate in
//! `PaymentChannelFund.cpp` to the current C++ behavior.

use protocol::Ter;
use tx::payment_channel_fund_destination::{
    PaymentChannelFundDestinationFacts, run_payment_channel_fund_destination_gate,
};

#[test]
fn destination_gate_returns_no_dst() {
    let result = run_payment_channel_fund_destination_gate(PaymentChannelFundDestinationFacts {
        destination_exists: false,
    });

    assert_eq!(result, Ter::TEC_NO_DST);
}

#[test]
fn destination_gate_passes_through_when_destination_exists() {
    let result = run_payment_channel_fund_destination_gate(PaymentChannelFundDestinationFacts {
        destination_exists: true,
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}
