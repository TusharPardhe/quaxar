//! Higher loaded-state wrapper for the destination lookup in the reference implementation.
//!
//! This wrapper preserves the current ordering above the already-landed loaded
//! owner-guard seam.

use crate::payment_channel_fund::PaymentChannelFundApplySink;
use crate::payment_channel_fund_loaded::{
    PaymentChannelFundLoadedChannelFacts, PaymentChannelFundLoadedTxFacts,
    build_payment_channel_fund_loaded_prepared_apply_facts,
};
use crate::payment_channel_fund_loaded_apply::run_payment_channel_fund_prepared_loaded_apply_do_apply;
use crate::payment_channel_fund_owner::load_payment_channel_fund_owner_guard_facts;
use protocol::Ter;

pub fn run_payment_channel_fund_loaded_destination_guarded_do_apply<
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
    S: PaymentChannelFundApplySink<Time>,
    ReadOwner: FnOnce() -> Option<Owner>,
    HasReserve: FnOnce(&Owner) -> bool,
    HasFunds: FnOnce(&Owner) -> bool,
    DestinationExists: FnOnce() -> bool,
{
    let Some(channel) = channel else {
        return Ter::TEC_NO_ENTRY;
    };

    if let Err(err) =
        load_payment_channel_fund_owner_guard_facts(read_owner, has_reserve, has_funds)
    {
        return err;
    }

    run_payment_channel_fund_prepared_loaded_apply_do_apply(
        build_payment_channel_fund_loaded_prepared_apply_facts(
            channel,
            tx,
            close_time,
            true,
            true,
            true,
            destination_exists(),
        ),
        sink,
    )
}
