//! Higher loaded-state wrapper for the destination-aware the reference implementation seam.
//!
//! This wrapper short-circuits missing channels before any sink work and
//! returns the current `tecNO_DST` result before deposit-preauth when the
//! loaded destination is missing.

#![allow(dead_code)]

use crate::payment_channel_claim::{
    PaymentChannelClaimApplySink, run_payment_channel_claim_do_apply,
};
use crate::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedPreparedApplyFacts,
    PaymentChannelClaimLoadedTxFacts, build_payment_channel_claim_loaded_prepared_apply_facts,
};
use protocol::Ter;
use std::ops::Add;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelClaimLoadedPreparedDestinationApplyFacts<Time> {
    pub prepared_apply_facts: PaymentChannelClaimLoadedPreparedApplyFacts<Time>,
}

struct PaymentChannelClaimLoadedDestinationSinkAdapter<'a, Time, S> {
    sink: &'a mut S,
    destination_exists: bool,
    _time: std::marker::PhantomData<Time>,
}

impl<'a, Time, S> PaymentChannelClaimApplySink<Time>
    for PaymentChannelClaimLoadedDestinationSinkAdapter<'a, Time, S>
where
    S: PaymentChannelClaimApplySink<Time>,
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
        self.sink.apply_refund_to_source_account(refund_drops)
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.sink.adjust_source_owner_count(delta)
    }

    fn erase_channel(&mut self) {
        self.sink.erase_channel()
    }

    fn destination_exists(&mut self) -> bool {
        self.destination_exists
    }

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

    fn clear_expiration(&mut self) {
        self.sink.clear_expiration();
    }

    fn set_expiration(&mut self, expiration: Time) {
        self.sink.set_expiration(expiration);
    }
}

pub fn build_payment_channel_claim_loaded_prepared_destination_apply_facts<AccountId, Time>(
    channel: PaymentChannelClaimLoadedChannelFacts<AccountId, Time>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
    destination_exists: bool,
) -> PaymentChannelClaimLoadedPreparedDestinationApplyFacts<Time>
where
    AccountId: Copy + Eq,
    Time: Copy,
{
    PaymentChannelClaimLoadedPreparedDestinationApplyFacts {
        prepared_apply_facts: build_payment_channel_claim_loaded_prepared_apply_facts(
            channel,
            tx,
            close_time,
            destination_exists,
        ),
    }
}

fn run_payment_channel_claim_loaded_prepared_apply_destination_do_apply<Time, S>(
    prepared: PaymentChannelClaimLoadedPreparedApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    let destination_exists = prepared.destination_exists;
    let facts = prepared.into_apply_facts();
    run_payment_channel_claim_do_apply(
        facts,
        &mut PaymentChannelClaimLoadedDestinationSinkAdapter {
            sink,
            destination_exists,
            _time: std::marker::PhantomData,
        },
    )
}

pub fn run_payment_channel_claim_loaded_prepared_destination_carrier_do_apply<Time, S>(
    prepared: PaymentChannelClaimLoadedPreparedDestinationApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    run_payment_channel_claim_loaded_prepared_apply_destination_do_apply(
        prepared.prepared_apply_facts,
        sink,
    )
}

pub(crate) fn run_payment_channel_claim_loaded_prepared_destination_do_apply<Time, S>(
    prepared: PaymentChannelClaimLoadedPreparedApplyFacts<Time>,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
{
    run_payment_channel_claim_loaded_prepared_apply_destination_do_apply(prepared, sink)
}

pub fn run_payment_channel_claim_loaded_destination_do_apply<AccountId, Time, S>(
    channel: Option<PaymentChannelClaimLoadedChannelFacts<AccountId, Time>>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
    destination_exists: bool,
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

    run_payment_channel_claim_loaded_prepared_destination_carrier_do_apply(
        build_payment_channel_claim_loaded_prepared_destination_apply_facts(
            channel,
            tx,
            close_time,
            destination_exists,
        ),
        sink,
    )
}
