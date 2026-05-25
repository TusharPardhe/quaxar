//! Narrow helper for shaping loaded the reference implementation state into the
//! current Rust apply facts.

#![allow(dead_code)]

use crate::payment_channel_claim::PaymentChannelClaimApplyFacts;
use crate::payment_channel_claim::PaymentChannelClaimApplySink;
use crate::payment_channel_claim_payment::{
    PaymentChannelClaimPaymentFacts, PaymentChannelClaimPaymentSink,
    run_payment_channel_claim_payment_guards, run_payment_channel_claim_payment_mutation,
};
#[cfg(test)]
#[path = "payment_channel_claim_loaded_destination_preauth_mutation_apply.rs"]
mod payment_channel_claim_loaded_destination_preauth_mutation_apply;
#[cfg(test)]
use self::payment_channel_claim_loaded_destination_preauth_mutation_apply::run_payment_channel_claim_loaded_prepared_destination_preauth_mutation_do_apply;
#[cfg(not(test))]
use crate::payment_channel_claim_loaded_destination_preauth_mutation_apply::run_payment_channel_claim_loaded_prepared_destination_preauth_mutation_do_apply;
use crate::payment_channel_due::PaymentChannelDueFacts;
use crate::payment_channel_helpers::PaymentChannelCloseFacts;
use protocol::Ter;
use std::ops::Add;

struct PaymentChannelClaimLoadedPaymentSinkAdapter<'a, Time, S> {
    sink: &'a mut S,
    _time: std::marker::PhantomData<Time>,
}

impl<'a, Time, S> PaymentChannelClaimPaymentSink
    for PaymentChannelClaimLoadedPaymentSinkAdapter<'a, Time, S>
where
    S: PaymentChannelClaimApplySink<Time>,
{
    fn verify_deposit_preauth(&mut self) -> Ter {
        self.sink.verify_deposit_preauth()
    }

    fn set_channel_balance(&mut self, balance_drops: u64) {
        self.sink.set_channel_balance(balance_drops);
    }

    fn add_destination_balance(&mut self, delta_drops: u64) {
        self.sink.add_destination_balance(delta_drops);
    }

    fn persist_destination_balance(&mut self) {
        self.sink.persist_destination_balance();
    }

    fn persist_channel_balance(&mut self) {
        self.sink.persist_channel_balance();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimLoadedChannelFacts<AccountId, Time> {
    pub source_account: AccountId,
    pub destination_account: AccountId,
    pub cancel_after: Option<Time>,
    pub current_expiration: Option<Time>,
    pub settle_delay: Time,
    pub channel_amount_drops: u64,
    pub channel_balance_drops: u64,
    pub destination_owner_directory_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimLoadedTxFacts<AccountId> {
    pub tx_account: AccountId,
    pub balance_present: bool,
    pub signature_present: bool,
    pub provided_public_key_matches_channel: bool,
    pub requested_balance_drops: u64,
    pub renew_flag: bool,
    pub close_flag: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimLoadedPreparedApplyFacts<Time> {
    pub apply_facts: PaymentChannelClaimApplyFacts<Time>,
    pub destination_exists: bool,
}

impl<Time> PaymentChannelClaimLoadedPreparedApplyFacts<Time> {
    pub fn into_apply_facts(self) -> PaymentChannelClaimApplyFacts<Time> {
        self.apply_facts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimLoadedPreparedPaymentFacts<Time> {
    pub apply_facts: PaymentChannelClaimApplyFacts<Time>,
    pub payment_facts: PaymentChannelClaimPaymentFacts,
}

pub fn build_payment_channel_claim_apply_facts<AccountId, Time>(
    channel: PaymentChannelClaimLoadedChannelFacts<AccountId, Time>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
) -> PaymentChannelClaimApplyFacts<Time>
where
    AccountId: Copy + Eq,
    Time: Copy,
{
    PaymentChannelClaimApplyFacts {
        channel_exists: true,
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
        tx_account_is_source: tx.tx_account == channel.source_account,
        tx_account_is_destination: tx.tx_account == channel.destination_account,
        balance_present: tx.balance_present,
        signature_present: tx.signature_present,
        provided_public_key_matches_channel: tx.provided_public_key_matches_channel,
        requested_balance_exceeds_channel_funds: tx.requested_balance_drops
            > channel.channel_amount_drops,
        requested_balance_not_above_channel_balance: tx.requested_balance_drops
            <= channel.channel_balance_drops,
        channel_balance_drops: channel.channel_balance_drops,
        requested_balance_drops: tx.requested_balance_drops,
        renew_flag: tx.renew_flag,
        close_flag: tx.close_flag,
        channel_fully_paid: channel.channel_balance_drops == channel.channel_amount_drops,
        current_expiration: channel.current_expiration,
        close_time,
        settle_delay: channel.settle_delay,
    }
}

pub fn build_payment_channel_claim_loaded_prepared_apply_facts<AccountId, Time>(
    channel: PaymentChannelClaimLoadedChannelFacts<AccountId, Time>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
    destination_exists: bool,
) -> PaymentChannelClaimLoadedPreparedApplyFacts<Time>
where
    AccountId: Copy + Eq,
    Time: Copy,
{
    PaymentChannelClaimLoadedPreparedApplyFacts {
        apply_facts: build_payment_channel_claim_apply_facts(channel, tx, close_time),
        destination_exists,
    }
}

pub fn build_payment_channel_claim_loaded_prepared_payment_facts<Time>(
    facts: PaymentChannelClaimApplyFacts<Time>,
) -> PaymentChannelClaimLoadedPreparedPaymentFacts<Time>
where
    Time: Copy,
{
    PaymentChannelClaimLoadedPreparedPaymentFacts {
        payment_facts: PaymentChannelClaimPaymentFacts {
            tx_account_is_destination: facts.tx_account_is_destination,
            signature_present: facts.signature_present,
            provided_public_key_matches_channel: facts.provided_public_key_matches_channel,
            requested_balance_exceeds_channel_funds: facts.requested_balance_exceeds_channel_funds,
            requested_balance_not_above_channel_balance: facts
                .requested_balance_not_above_channel_balance,
            channel_balance_drops: facts.channel_balance_drops,
            requested_balance_drops: facts.requested_balance_drops,
        },
        apply_facts: facts,
    }
}

pub fn run_payment_channel_claim_prepared_loaded_destination_preauth_mutation_do_apply<
    Time,
    S,
    ReadDestination,
    VerifyDepositPreauth,
    RunMutation,
>(
    facts: PaymentChannelClaimApplyFacts<Time>,
    read_destination: ReadDestination,
    verify_deposit_preauth: VerifyDepositPreauth,
    run_mutation: RunMutation,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
    ReadDestination: FnOnce() -> bool,
    VerifyDepositPreauth: FnOnce() -> Ter,
    RunMutation: FnOnce(PaymentChannelClaimApplyFacts<Time>, &mut S) -> Ter,
{
    run_payment_channel_claim_loaded_prepared_destination_preauth_mutation_do_apply(
        PaymentChannelClaimLoadedPreparedApplyFacts {
            apply_facts: facts,
            destination_exists: read_destination(),
        },
        verify_deposit_preauth,
        run_mutation,
        sink,
    )
}

pub fn run_payment_channel_claim_prepared_loaded_payment_do_apply<Time, S>(
    facts: PaymentChannelClaimApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    run_payment_channel_claim_loaded_prepared_payment_do_apply(
        build_payment_channel_claim_loaded_prepared_payment_facts(facts),
        sink,
    )
}

pub fn run_payment_channel_claim_loaded_prepared_payment_do_apply<Time, S>(
    prepared: PaymentChannelClaimLoadedPreparedPaymentFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    let err = run_payment_channel_claim_payment_guards(prepared.payment_facts);
    if err != Ter::TES_SUCCESS {
        return err;
    }

    let destination_exists = sink.destination_exists();
    if !destination_exists {
        return Ter::TEC_NO_DST;
    }

    let verify_deposit_preauth = sink.verify_deposit_preauth();
    run_payment_channel_claim_loaded_prepared_destination_preauth_mutation_do_apply(
        PaymentChannelClaimLoadedPreparedApplyFacts {
            apply_facts: prepared.apply_facts,
            destination_exists,
        },
        || verify_deposit_preauth,
        |_, sink| {
            run_payment_channel_claim_payment_mutation(
                prepared.payment_facts,
                &mut PaymentChannelClaimLoadedPaymentSinkAdapter {
                    sink,
                    _time: std::marker::PhantomData,
                },
            )
        },
        sink,
    )
}
