//! Integration tests that pin the loaded destination-account gate in
//! `PaymentChannelClaim.cpp`.

use protocol::Ter;
use tx::payment_channel_claim_destination::{
    PaymentChannelClaimDestinationFacts, run_payment_channel_claim_destination_gate,
};

#[test]
fn destination_branch_returns_no_dst_before_deposit_preauth() {
    let result = run_payment_channel_claim_destination_gate(PaymentChannelClaimDestinationFacts {
        destination_exists: false,
    });

    assert_eq!(result, Ter::TEC_NO_DST);
}

#[test]
fn destination_branch_passthroughs_success() {
    let result = run_payment_channel_claim_destination_gate(PaymentChannelClaimDestinationFacts {
        destination_exists: true,
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}
