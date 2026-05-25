//! Higher loaded-state wrapper for the destination lookup, preauth, and final
//! mutation slice in the reference implementation.
//!
//! This wrapper preserves the current ordering above the already-landed
//! destination+preauth seam.

#![allow(dead_code)]

use crate::payment_channel_claim::{PaymentChannelClaimApplyFacts, PaymentChannelClaimApplySink};
use crate::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedPreparedApplyFacts,
    PaymentChannelClaimLoadedTxFacts, build_payment_channel_claim_loaded_prepared_apply_facts,
};
use protocol::Ter;
use std::ops::Add;

pub(crate) fn run_payment_channel_claim_loaded_prepared_destination_preauth_mutation_do_apply<
    Time,
    S,
    VerifyDepositPreauth,
    RunMutation,
>(
    prepared: PaymentChannelClaimLoadedPreparedApplyFacts<Time>,
    verify_deposit_preauth: VerifyDepositPreauth,
    run_mutation: RunMutation,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
    VerifyDepositPreauth: FnOnce() -> Ter,
    RunMutation: FnOnce(PaymentChannelClaimApplyFacts<Time>, &mut S) -> Ter,
{
    if !prepared.destination_exists {
        return Ter::TEC_NO_DST;
    }

    let err = verify_deposit_preauth();
    if err != Ter::TES_SUCCESS {
        return err;
    }

    run_mutation(prepared.into_apply_facts(), sink)
}

pub fn run_payment_channel_claim_loaded_destination_preauth_mutation_do_apply<
    AccountId,
    Time,
    S,
    ReadDestination,
    VerifyDepositPreauth,
    RunMutation,
>(
    channel: Option<PaymentChannelClaimLoadedChannelFacts<AccountId, Time>>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
    read_destination: ReadDestination,
    verify_deposit_preauth: VerifyDepositPreauth,
    run_mutation: RunMutation,
    sink: &mut S,
) -> Ter
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
    ReadDestination: FnOnce() -> bool,
    VerifyDepositPreauth: FnOnce() -> Ter,
    RunMutation: FnOnce(PaymentChannelClaimApplyFacts<Time>, &mut S) -> Ter,
{
    let Some(channel) = channel else {
        return Ter::TEC_NO_TARGET;
    };

    let destination_exists = read_destination();
    if !destination_exists {
        return Ter::TEC_NO_DST;
    }

    run_payment_channel_claim_loaded_prepared_destination_preauth_mutation_do_apply(
        build_payment_channel_claim_loaded_prepared_apply_facts(
            channel,
            tx,
            close_time,
            destination_exists,
        ),
        verify_deposit_preauth,
        run_mutation,
        sink,
    )
}
