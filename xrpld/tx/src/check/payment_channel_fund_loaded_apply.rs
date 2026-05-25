//! Higher loaded-state wrapper for the reference implementation.
//!
//! This wrapper moves the already-loaded channel-plus-guard handoff into one
//! Rust seam above the current loaded-facts builder and apply shell.

use crate::payment_channel_fund::{
    PaymentChannelFundApplyFacts, PaymentChannelFundApplySink, run_payment_channel_fund_core_apply,
};
use crate::payment_channel_fund_loaded::{
    PaymentChannelFundLoadedApplyFacts, PaymentChannelFundLoadedChannelFacts,
    PaymentChannelFundLoadedPreparedApplyFacts, PaymentChannelFundLoadedTxFacts,
    build_payment_channel_fund_loaded_apply_facts,
};
use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundLoadedGuardFacts {
    pub owner_account_exists: bool,
    pub owner_balance_covers_reserve: bool,
    pub owner_balance_covers_reserve_plus_amount: bool,
    pub destination_exists: bool,
}

pub fn run_payment_channel_fund_loaded_do_apply<AccountId, Time, S>(
    channel: Option<PaymentChannelFundLoadedChannelFacts<AccountId, Time>>,
    tx: PaymentChannelFundLoadedTxFacts<AccountId, Time>,
    close_time: Time,
    guard_facts: PaymentChannelFundLoadedGuardFacts,
    sink: &mut S,
) -> Ter
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + std::ops::Add<Output = Time>,
    S: PaymentChannelFundApplySink<Time>,
{
    let Some(channel) = channel else {
        return Ter::TEC_NO_ENTRY;
    };

    run_payment_channel_fund_loaded_apply_facts_do_apply(
        build_payment_channel_fund_loaded_apply_facts(channel, tx, close_time),
        guard_facts,
        sink,
    )
}

pub fn run_payment_channel_fund_loaded_apply_facts_do_apply<Time, S>(
    facts: PaymentChannelFundLoadedApplyFacts<Time>,
    guard_facts: PaymentChannelFundLoadedGuardFacts,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundApplySink<Time>,
{
    run_payment_channel_fund_core_apply(
        facts.into_apply_facts(
            guard_facts.owner_account_exists,
            guard_facts.owner_balance_covers_reserve,
            guard_facts.owner_balance_covers_reserve_plus_amount,
            guard_facts.destination_exists,
        ),
        sink,
    )
}

pub fn run_payment_channel_fund_prepared_loaded_do_apply<Time, S>(
    facts: PaymentChannelFundApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundApplySink<Time>,
{
    run_payment_channel_fund_loaded_apply_facts_do_apply(
        PaymentChannelFundLoadedApplyFacts {
            due_facts: facts.due_facts,
            close_facts: facts.close_facts,
            tx_account_is_owner: facts.tx_account_is_owner,
            channel_amount_drops: facts.channel_amount_drops,
            fund_amount_drops: facts.fund_amount_drops,
            extend_expiration: facts.extend_expiration,
            min_extend_expiration: facts.min_extend_expiration,
        },
        PaymentChannelFundLoadedGuardFacts {
            owner_account_exists: facts.owner_account_exists,
            owner_balance_covers_reserve: facts.owner_balance_covers_reserve,
            owner_balance_covers_reserve_plus_amount: facts
                .owner_balance_covers_reserve_plus_amount,
            destination_exists: facts.destination_exists,
        },
        sink,
    )
}

pub fn run_payment_channel_fund_prepared_loaded_apply_do_apply<Time, S>(
    prepared: PaymentChannelFundLoadedPreparedApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundApplySink<Time>,
{
    run_payment_channel_fund_loaded_apply_facts_do_apply(
        prepared.apply_facts,
        prepared.guard_facts,
        sink,
    )
}
