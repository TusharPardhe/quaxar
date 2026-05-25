//! Narrow helper for shaping loaded the reference implementation state into the
//! current Rust apply facts.

use crate::payment_channel_due::PaymentChannelDueFacts;
use crate::payment_channel_fund::{
    PaymentChannelFundApplyFacts, run_payment_channel_fund_min_extend_expiration,
};
use crate::payment_channel_fund_loaded_apply::PaymentChannelFundLoadedGuardFacts;
use crate::payment_channel_helpers::PaymentChannelCloseFacts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundLoadedChannelFacts<AccountId, Time> {
    pub source_account: AccountId,
    pub cancel_after: Option<Time>,
    pub current_expiration: Option<Time>,
    pub settle_delay: Time,
    pub channel_amount_drops: u64,
    pub channel_balance_drops: u64,
    pub destination_owner_directory_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundLoadedTxFacts<AccountId, Time> {
    pub tx_account: AccountId,
    pub extend_expiration: Option<Time>,
    pub fund_amount_drops: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundLoadedApplyFacts<Time> {
    pub due_facts: PaymentChannelDueFacts<Time>,
    pub close_facts: PaymentChannelCloseFacts,
    pub tx_account_is_owner: bool,
    pub channel_amount_drops: u64,
    pub fund_amount_drops: u64,
    pub extend_expiration: Option<Time>,
    pub min_extend_expiration: Time,
}

impl<Time> PaymentChannelFundLoadedApplyFacts<Time>
where
    Time: Copy,
{
    pub fn into_apply_facts(
        self,
        owner_account_exists: bool,
        owner_balance_covers_reserve: bool,
        owner_balance_covers_reserve_plus_amount: bool,
        destination_exists: bool,
    ) -> PaymentChannelFundApplyFacts<Time> {
        PaymentChannelFundApplyFacts {
            channel_exists: true,
            due_facts: self.due_facts,
            close_facts: self.close_facts,
            tx_account_is_owner: self.tx_account_is_owner,
            channel_amount_drops: self.channel_amount_drops,
            fund_amount_drops: self.fund_amount_drops,
            extend_expiration: self.extend_expiration,
            min_extend_expiration: self.min_extend_expiration,
            owner_account_exists,
            owner_balance_covers_reserve,
            owner_balance_covers_reserve_plus_amount,
            destination_exists,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelFundLoadedPreparedApplyFacts<Time> {
    pub apply_facts: PaymentChannelFundLoadedApplyFacts<Time>,
    pub guard_facts: PaymentChannelFundLoadedGuardFacts,
}

pub fn build_payment_channel_fund_loaded_apply_facts<AccountId, Time>(
    channel: PaymentChannelFundLoadedChannelFacts<AccountId, Time>,
    tx: PaymentChannelFundLoadedTxFacts<AccountId, Time>,
    close_time: Time,
) -> PaymentChannelFundLoadedApplyFacts<Time>
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + std::ops::Add<Output = Time>,
{
    PaymentChannelFundLoadedApplyFacts {
        due_facts: PaymentChannelDueFacts {
            cancel_after: channel.cancel_after,
            expiration: channel.current_expiration,
            close_time,
        },
        close_facts: PaymentChannelCloseFacts {
            destination_owner_directory_present: channel.destination_owner_directory_present,
            channel_amount_drops: channel.channel_amount_drops,
            channel_balance_drops: channel.channel_balance_drops,
        },
        tx_account_is_owner: tx.tx_account == channel.source_account,
        channel_amount_drops: channel.channel_amount_drops,
        fund_amount_drops: tx.fund_amount_drops,
        extend_expiration: tx.extend_expiration,
        min_extend_expiration: run_payment_channel_fund_min_extend_expiration(
            close_time,
            channel.settle_delay,
            channel.current_expiration,
        ),
    }
}

pub fn build_payment_channel_fund_loaded_prepared_apply_facts<AccountId, Time>(
    channel: PaymentChannelFundLoadedChannelFacts<AccountId, Time>,
    tx: PaymentChannelFundLoadedTxFacts<AccountId, Time>,
    close_time: Time,
    owner_account_exists: bool,
    owner_balance_covers_reserve: bool,
    owner_balance_covers_reserve_plus_amount: bool,
    destination_exists: bool,
) -> PaymentChannelFundLoadedPreparedApplyFacts<Time>
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + std::ops::Add<Output = Time>,
{
    PaymentChannelFundLoadedPreparedApplyFacts {
        apply_facts: build_payment_channel_fund_loaded_apply_facts(channel, tx, close_time),
        guard_facts: PaymentChannelFundLoadedGuardFacts {
            owner_account_exists,
            owner_balance_covers_reserve,
            owner_balance_covers_reserve_plus_amount,
            destination_exists,
        },
    }
}
