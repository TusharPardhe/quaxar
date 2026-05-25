//! Renew and close-settle tail for the reference implementation after the balance
//! branch.
//!
//! This helper matches the the reference implementation tail ordering for source-only renew
//! permission checks, close callback invocation, and expiration updates.

use protocol::Ter;
use std::ops::Add;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimSettleFacts<Time> {
    pub tx_account_is_source: bool,
    pub renew_flag: bool,
    pub close_flag: bool,
    pub tx_account_is_destination: bool,
    pub channel_fully_paid: bool,
    pub current_expiration: Option<Time>,
    pub close_time: Time,
    pub settle_delay: Time,
}

pub trait PaymentChannelClaimSettleSink<Time> {
    fn clear_expiration(&mut self);
    fn set_expiration(&mut self, expiration: Time);
    fn close_channel(&mut self) -> Ter;
}

pub fn run_payment_channel_claim_settle<Time, S>(
    facts: PaymentChannelClaimSettleFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimSettleSink<Time>,
{
    if facts.renew_flag {
        if !facts.tx_account_is_source {
            return Ter::TEC_NO_PERMISSION;
        }

        sink.clear_expiration();
    }

    if facts.close_flag {
        if facts.tx_account_is_destination || facts.channel_fully_paid {
            return sink.close_channel();
        }

        let settle_expiration = facts.close_time + facts.settle_delay;
        if facts
            .current_expiration
            .is_none_or(|expiration| expiration > settle_expiration)
        {
            sink.set_expiration(settle_expiration);
        }
    }

    Ter::TES_SUCCESS
}
