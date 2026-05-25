//! Loaded-state helper for the next the reference implementation step.
//!
//! This helper short-circuits missing channels before any sink work and
//! delegates the loaded path to the existing apply-facts builder plus
//! `doApply()` shell.

use crate::payment_channel_claim::{
    PaymentChannelClaimApplyFacts, PaymentChannelClaimApplySink, run_payment_channel_claim_do_apply,
};
use crate::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedTxFacts,
    build_payment_channel_claim_apply_facts,
};
use protocol::Ter;
use std::ops::Add;

pub fn run_payment_channel_claim_loaded_do_apply<AccountId, Time, S>(
    channel: Option<PaymentChannelClaimLoadedChannelFacts<AccountId, Time>>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
    sink: &mut S,
) -> Ter
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    let Some(channel) = channel else {
        return Ter::TEC_NO_TARGET;
    };

    let facts = build_payment_channel_claim_apply_facts(channel, tx, close_time);
    run_payment_channel_claim_prepared_loaded_do_apply(facts, sink)
}

pub fn run_payment_channel_claim_prepared_loaded_do_apply<Time, S>(
    facts: PaymentChannelClaimApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    run_payment_channel_claim_do_apply(facts, sink)
}
