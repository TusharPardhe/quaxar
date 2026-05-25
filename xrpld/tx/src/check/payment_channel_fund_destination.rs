//! Destination-existence gate for the reference implementation.
//!
//! This helper preserves the the reference implementation ordering: destination existence is
//! checked after the owner-account guards and before any mutation work.

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundDestinationFacts {
    pub destination_exists: bool,
}

pub fn run_payment_channel_fund_destination_gate(facts: PaymentChannelFundDestinationFacts) -> Ter {
    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    Ter::TES_SUCCESS
}
