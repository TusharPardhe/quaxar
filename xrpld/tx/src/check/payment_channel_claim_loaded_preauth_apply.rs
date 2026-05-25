//! Higher loaded-state wrapper for the preauth gate in the reference implementation.
//!
//! This wrapper preserves the current ordering above the already-landed
//! destination-aware loaded claim seam.

use crate::payment_channel_claim::PaymentChannelClaimApplySink;
use crate::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedPreparedApplyFacts,
    PaymentChannelClaimLoadedTxFacts, build_payment_channel_claim_loaded_prepared_apply_facts,
};
use crate::payment_channel_claim_loaded_destination_apply::run_payment_channel_claim_loaded_prepared_destination_do_apply;
use protocol::Ter;
use std::ops::Add;

struct PaymentChannelClaimLoadedPreauthSinkAdapter<'a, Time, S, VerifyDepositPreauth> {
    sink: &'a mut S,
    verify_deposit_preauth: Option<VerifyDepositPreauth>,
    _time: std::marker::PhantomData<Time>,
}

impl<'a, Time, S, VerifyDepositPreauth> PaymentChannelClaimApplySink<Time>
    for PaymentChannelClaimLoadedPreauthSinkAdapter<'a, Time, S, VerifyDepositPreauth>
where
    S: PaymentChannelClaimApplySink<Time>,
    VerifyDepositPreauth: FnOnce() -> Ter,
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
        self.sink.destination_exists()
    }

    fn verify_deposit_preauth(&mut self) -> Ter {
        let verify_deposit_preauth = self
            .verify_deposit_preauth
            .take()
            .expect("deposit-preauth callback should be called at most once");
        verify_deposit_preauth()
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

pub(crate) fn run_payment_channel_claim_loaded_prepared_preauth_do_apply<
    Time,
    S,
    VerifyDepositPreauth,
>(
    prepared: PaymentChannelClaimLoadedPreparedApplyFacts<Time>,
    verify_deposit_preauth: VerifyDepositPreauth,
    sink: &mut S,
) -> Ter
where
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
    VerifyDepositPreauth: FnOnce() -> Ter,
{
    run_payment_channel_claim_loaded_prepared_destination_do_apply(
        prepared,
        &mut PaymentChannelClaimLoadedPreauthSinkAdapter {
            sink,
            verify_deposit_preauth: Some(verify_deposit_preauth),
            _time: std::marker::PhantomData,
        },
    )
}

pub fn run_payment_channel_claim_loaded_preauth_do_apply<AccountId, Time, S, VerifyDepositPreauth>(
    channel: Option<PaymentChannelClaimLoadedChannelFacts<AccountId, Time>>,
    tx: PaymentChannelClaimLoadedTxFacts<AccountId>,
    close_time: Time,
    destination_exists: bool,
    verify_deposit_preauth: VerifyDepositPreauth,
    sink: &mut S,
) -> Ter
where
    AccountId: Copy + Eq,
    Time: Copy + Ord + Add<Output = Time>,
    S: PaymentChannelClaimApplySink<Time>,
    VerifyDepositPreauth: FnOnce() -> Ter,
{
    let Some(channel) = channel else {
        return Ter::TEC_NO_TARGET;
    };

    run_payment_channel_claim_loaded_prepared_preauth_do_apply(
        build_payment_channel_claim_loaded_prepared_apply_facts(
            channel,
            tx,
            close_time,
            destination_exists,
        ),
        verify_deposit_preauth,
        sink,
    )
}
