//! Higher loaded-state wrapper for the destination lookup plus preauth slice
//! in the reference implementation.
//!
//! This wrapper preserves the current ordering above the already-landed loaded
//! preauth seam.

use crate::payment_channel_claim::PaymentChannelClaimApplySink;
use crate::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedTxFacts,
    build_payment_channel_claim_loaded_prepared_apply_facts,
};
use crate::payment_channel_claim_loaded_preauth_apply::run_payment_channel_claim_loaded_prepared_preauth_do_apply;
use protocol::Ter;
use std::ops::Add;

pub fn run_payment_channel_claim_loaded_destination_preauth_do_apply<
    AccountId,
    Time,
    S,
    ReadDestination,
    VerifyDepositPreauth,
>(
    channel: Option<PaymentChannelClaimLoadedChannelFacts<AccountId, Time>>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
    read_destination: ReadDestination,
    verify_deposit_preauth: VerifyDepositPreauth,
    sink: &mut S,
) -> Ter
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
    ReadDestination: FnOnce() -> bool,
    VerifyDepositPreauth: FnOnce() -> Ter,
{
    let Some(channel) = channel else {
        return Ter::TEC_NO_TARGET;
    };

    if !read_destination() {
        return Ter::TEC_NO_DST;
    }

    run_payment_channel_claim_loaded_prepared_preauth_do_apply(
        build_payment_channel_claim_loaded_prepared_apply_facts(channel, tx, close_time, true),
        verify_deposit_preauth,
        sink,
    )
}
