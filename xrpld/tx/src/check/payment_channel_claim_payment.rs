//! Payment-branch helper for the `sfBalance` path in the reference implementation.
//!
//! This helper stays pure over the branch decisions, consumes the precomputed
//! facts, and drives the required callback ordering.

#![allow(dead_code)]

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimPaymentFacts {
    pub tx_account_is_destination: bool,
    pub signature_present: bool,
    pub provided_public_key_matches_channel: bool,
    pub requested_balance_exceeds_channel_funds: bool,
    pub requested_balance_not_above_channel_balance: bool,
    pub channel_balance_drops: u64,
    pub requested_balance_drops: u64,
}

pub trait PaymentChannelClaimPaymentSink {
    fn verify_deposit_preauth(&mut self) -> Ter;
    fn set_channel_balance(&mut self, balance_drops: u64);
    fn add_destination_balance(&mut self, delta_drops: u64);
    fn persist_destination_balance(&mut self);
    fn persist_channel_balance(&mut self);
}

const fn validate_payment_channel_claim_payment(facts: PaymentChannelClaimPaymentFacts) -> Ter {
    if facts.tx_account_is_destination && !facts.signature_present {
        return Ter::TEM_BAD_SIGNATURE;
    }

    if facts.signature_present && !facts.provided_public_key_matches_channel {
        return Ter::TEM_BAD_SIGNER;
    }

    if facts.requested_balance_exceeds_channel_funds {
        return Ter::TEC_UNFUNDED_PAYMENT;
    }

    if facts.requested_balance_not_above_channel_balance {
        return Ter::TEC_UNFUNDED_PAYMENT;
    }

    Ter::TES_SUCCESS
}

pub const fn run_payment_channel_claim_payment_guards(
    facts: PaymentChannelClaimPaymentFacts,
) -> Ter {
    validate_payment_channel_claim_payment(facts)
}

pub fn run_payment_channel_claim_payment_mutation<S>(
    facts: PaymentChannelClaimPaymentFacts,
    sink: &mut S,
) -> Ter
where
    S: PaymentChannelClaimPaymentSink,
{
    let delta_drops = facts
        .requested_balance_drops
        .checked_sub(facts.channel_balance_drops)
        .expect("branch guards keep requested balance above channel balance");
    sink.set_channel_balance(facts.requested_balance_drops);
    sink.add_destination_balance(delta_drops);
    sink.persist_destination_balance();
    sink.persist_channel_balance();
    Ter::TES_SUCCESS
}

pub fn run_payment_channel_claim_payment_branch<S>(
    facts: PaymentChannelClaimPaymentFacts,
    sink: &mut S,
) -> Ter
where
    S: PaymentChannelClaimPaymentSink,
{
    let err = validate_payment_channel_claim_payment(facts);
    if err != Ter::TES_SUCCESS {
        return err;
    }

    let err = sink.verify_deposit_preauth();
    if err != Ter::TES_SUCCESS {
        return err;
    }

    run_payment_channel_claim_payment_mutation(facts, sink)
}
