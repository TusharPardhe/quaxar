//! Destination-account gate for the loaded the reference implementation
//! flow.
//!
//! This helper makes the current `tecNO_DST` gate explicit after the local
//! payment guards and before deposit-preauth and payment mutations.

#![allow(dead_code)]

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimDestinationFacts {
    pub destination_exists: bool,
}

pub fn run_payment_channel_claim_destination_gate(
    facts: PaymentChannelClaimDestinationFacts,
) -> Ter {
    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    Ter::TES_SUCCESS
}
