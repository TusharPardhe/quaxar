//! Higher loaded-state wrapper for the final mutation slice in
//! the reference implementation.
//!
//! This wrapper preserves the current ordering above the already-landed loaded
//! destination-read seam.

use crate::payment_channel_due::is_payment_channel_due;
use crate::payment_channel_fund::{PaymentChannelFundApplyFacts, PaymentChannelFundApplySink};
use crate::payment_channel_fund_loaded::{
    PaymentChannelFundLoadedApplyFacts, PaymentChannelFundLoadedChannelFacts,
    PaymentChannelFundLoadedPreparedApplyFacts, PaymentChannelFundLoadedTxFacts,
};
use crate::payment_channel_fund_loaded_apply::{
    PaymentChannelFundLoadedGuardFacts, run_payment_channel_fund_loaded_apply_facts_do_apply,
    run_payment_channel_fund_prepared_loaded_do_apply,
};
use crate::payment_channel_fund_loaded_destination_guarded_apply::run_payment_channel_fund_loaded_destination_guarded_do_apply;
use crate::payment_channel_helpers::{PaymentChannelCloseSink, run_payment_channel_close};
use protocol::Ter;
use std::marker::PhantomData;

pub trait PaymentChannelFundLoadedDestinationMutationApplySink<Time>:
    PaymentChannelCloseSink
{
    fn update_expiration(&mut self, expiration: Time);
    fn apply_fund_mutation(&mut self, new_channel_amount: u64, fund_amount_drops: u64);
}

struct PaymentChannelFundApplySinkAdapter<'a, Time, S> {
    sink: &'a mut S,
    _time: PhantomData<Time>,
}

impl<'a, Time, S> PaymentChannelCloseSink for PaymentChannelFundApplySinkAdapter<'a, Time, S>
where
    S: PaymentChannelFundApplySink<Time>,
{
    fn remove_source_owner_directory(&mut self) -> Ter {
        self.sink.remove_source_owner_directory()
    }

    fn remove_destination_owner_directory(&mut self) -> Ter {
        self.sink.remove_destination_owner_directory()
    }

    fn source_account_exists(&mut self) -> bool {
        self.sink.source_account_exists()
    }

    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        self.sink.apply_refund_to_source_account(refund_drops);
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.sink.adjust_source_owner_count(delta);
    }

    fn erase_channel(&mut self) {
        self.sink.erase_channel();
    }
}

impl<'a, Time, S> PaymentChannelFundLoadedDestinationMutationApplySink<Time>
    for PaymentChannelFundApplySinkAdapter<'a, Time, S>
where
    S: PaymentChannelFundApplySink<Time>,
{
    fn update_expiration(&mut self, expiration: Time) {
        self.sink.update_expiration(expiration);
    }

    fn apply_fund_mutation(&mut self, new_channel_amount: u64, fund_amount_drops: u64) {
        self.sink.set_channel_amount(new_channel_amount);
        self.sink.persist_channel();
        self.sink.subtract_owner_balance(fund_amount_drops);
        self.sink.persist_owner();
    }
}

impl<'a, Time, S> PaymentChannelFundApplySink<Time>
    for PaymentChannelFundApplySinkAdapter<'a, Time, S>
where
    S: PaymentChannelFundApplySink<Time>,
{
    fn update_expiration(&mut self, expiration: Time) {
        self.sink.update_expiration(expiration);
    }

    fn set_channel_amount(&mut self, amount_drops: u64) {
        self.sink.set_channel_amount(amount_drops);
    }

    fn persist_channel(&mut self) {
        self.sink.persist_channel();
    }

    fn subtract_owner_balance(&mut self, amount_drops: u64) {
        self.sink.subtract_owner_balance(amount_drops);
    }

    fn persist_owner(&mut self) {
        self.sink.persist_owner();
    }
}

struct PaymentChannelFundLoadedDestinationMutationSinkAdapter<'a, Time, S> {
    sink: &'a mut S,
    pending_channel_amount: Option<u64>,
    pending_fund_amount: Option<u64>,
    _time: std::marker::PhantomData<Time>,
}

impl<'a, Time, S> PaymentChannelCloseSink
    for PaymentChannelFundLoadedDestinationMutationSinkAdapter<'a, Time, S>
where
    S: PaymentChannelFundLoadedDestinationMutationApplySink<Time>,
{
    fn remove_source_owner_directory(&mut self) -> Ter {
        self.sink.remove_source_owner_directory()
    }

    fn remove_destination_owner_directory(&mut self) -> Ter {
        self.sink.remove_destination_owner_directory()
    }

    fn source_account_exists(&mut self) -> bool {
        self.sink.source_account_exists()
    }

    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        self.sink.apply_refund_to_source_account(refund_drops);
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.sink.adjust_source_owner_count(delta);
    }

    fn erase_channel(&mut self) {
        self.sink.erase_channel();
    }
}

impl<'a, Time, S> PaymentChannelFundApplySink<Time>
    for PaymentChannelFundLoadedDestinationMutationSinkAdapter<'a, Time, S>
where
    S: PaymentChannelFundLoadedDestinationMutationApplySink<Time>,
{
    fn update_expiration(&mut self, expiration: Time) {
        self.sink.update_expiration(expiration);
    }

    fn set_channel_amount(&mut self, amount_drops: u64) {
        self.pending_channel_amount = Some(amount_drops);
    }

    fn persist_channel(&mut self) {}

    fn subtract_owner_balance(&mut self, amount_drops: u64) {
        self.pending_fund_amount = Some(amount_drops);
    }

    fn persist_owner(&mut self) {
        let new_channel_amount = self
            .pending_channel_amount
            .take()
            .expect("channel amount should be prepared before owner persist");
        let fund_amount_drops = self
            .pending_fund_amount
            .take()
            .expect("fund amount should be prepared before owner persist");
        self.sink
            .apply_fund_mutation(new_channel_amount, fund_amount_drops);
    }
}

pub fn run_payment_channel_fund_loaded_destination_mutation_guarded_do_apply<
    AccountId,
    Time,
    Owner,
    S,
    ReadOwner,
    HasReserve,
    HasFunds,
    DestinationExists,
>(
    channel: Option<PaymentChannelFundLoadedChannelFacts<AccountId, Time>>,
    tx: PaymentChannelFundLoadedTxFacts<AccountId, Time>,
    close_time: Time,
    read_owner: ReadOwner,
    has_reserve: HasReserve,
    has_funds: HasFunds,
    destination_exists: DestinationExists,
    sink: &mut S,
) -> Ter
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + std::ops::Add<Output = Time>,
    S: PaymentChannelFundLoadedDestinationMutationApplySink<Time>,
    ReadOwner: FnOnce() -> Option<Owner>,
    HasReserve: FnOnce(&Owner) -> bool,
    HasFunds: FnOnce(&Owner) -> bool,
    DestinationExists: FnOnce() -> bool,
{
    run_payment_channel_fund_loaded_destination_guarded_do_apply(
        channel,
        tx,
        close_time,
        read_owner,
        has_reserve,
        has_funds,
        destination_exists,
        &mut PaymentChannelFundLoadedDestinationMutationSinkAdapter {
            sink,
            pending_channel_amount: None,
            pending_fund_amount: None,
            _time: std::marker::PhantomData,
        },
    )
}

pub fn run_payment_channel_fund_prepared_destination_mutation_guarded_do_apply<Time, S>(
    facts: PaymentChannelFundLoadedApplyFacts<Time>,
    guard_facts: PaymentChannelFundLoadedGuardFacts,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundLoadedDestinationMutationApplySink<Time>,
{
    run_payment_channel_fund_loaded_prepared_destination_mutation_guarded_do_apply(
        PaymentChannelFundLoadedPreparedApplyFacts {
            apply_facts: facts,
            guard_facts,
        },
        sink,
    )
}

pub fn run_payment_channel_fund_loaded_prepared_destination_mutation_guarded_do_apply<Time, S>(
    prepared: PaymentChannelFundLoadedPreparedApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundLoadedDestinationMutationApplySink<Time>,
{
    run_payment_channel_fund_loaded_apply_facts_do_apply(
        prepared.apply_facts,
        prepared.guard_facts,
        &mut PaymentChannelFundLoadedDestinationMutationSinkAdapter {
            sink,
            pending_channel_amount: None,
            pending_fund_amount: None,
            _time: std::marker::PhantomData,
        },
    )
}

pub fn run_payment_channel_fund_apply_destination_mutation_guarded_do_apply<Time, S>(
    facts: PaymentChannelFundApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord,
    S: PaymentChannelFundApplySink<Time>,
{
    if !facts.channel_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if is_payment_channel_due(facts.due_facts) {
        return run_payment_channel_close(facts.close_facts, sink);
    }

    run_payment_channel_fund_prepared_loaded_do_apply(
        facts,
        &mut PaymentChannelFundApplySinkAdapter {
            sink,
            _time: PhantomData,
        },
    )
}

pub fn build_payment_channel_fund_loaded_destination_mutation_apply_facts<AccountId, Time>(
    channel: PaymentChannelFundLoadedChannelFacts<AccountId, Time>,
    tx: PaymentChannelFundLoadedTxFacts<AccountId, Time>,
    close_time: Time,
    destination_exists: bool,
) -> PaymentChannelFundApplyFacts<Time>
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + std::ops::Add<Output = Time>,
{
    use crate::payment_channel_fund_loaded::build_payment_channel_fund_loaded_apply_facts;

    build_payment_channel_fund_loaded_apply_facts(channel, tx, close_time).into_apply_facts(
        true,
        true,
        true,
        destination_exists,
    )
}
