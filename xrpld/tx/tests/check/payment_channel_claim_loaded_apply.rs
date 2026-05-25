//! Integration tests that pin the higher loaded `PaymentChannelClaim.cpp`
//! apply seam to the current Rust behavior.

use protocol::Ter;
use tx::payment_channel_claim::{PaymentChannelClaimApplySink, run_payment_channel_claim_do_apply};
use tx::payment_channel_claim_loaded::run_payment_channel_claim_loaded_prepared_payment_do_apply;
use tx::payment_channel_claim_loaded::{
    PaymentChannelClaimLoadedChannelFacts, PaymentChannelClaimLoadedPreparedApplyFacts,
    PaymentChannelClaimLoadedTxFacts, build_payment_channel_claim_apply_facts,
    build_payment_channel_claim_loaded_prepared_payment_facts,
};
use tx::payment_channel_claim_loaded_apply::{
    run_payment_channel_claim_loaded_do_apply, run_payment_channel_claim_prepared_loaded_do_apply,
};

#[derive(Debug, Default)]
struct TestApplySink {
    deposit_preauth_result: Ter,
    source_dir_result: Ter,
    destination_dir_result: Ter,
    source_account_exists: bool,
    events: Vec<String>,
    expirations: Vec<u32>,
    refund_drops: Option<u64>,
    owner_count_deltas: Vec<i32>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            deposit_preauth_result: Ter::TES_SUCCESS,
            source_dir_result: Ter::TES_SUCCESS,
            destination_dir_result: Ter::TES_SUCCESS,
            source_account_exists: true,
            events: Vec::new(),
            expirations: Vec::new(),
            refund_drops: None,
            owner_count_deltas: Vec::new(),
        }
    }
}

impl PaymentChannelClaimApplySink<u32> for TestApplySink {
    fn remove_source_owner_directory(&mut self) -> Ter {
        self.events
            .push("remove_source_owner_directory".to_string());
        self.source_dir_result
    }

    fn remove_destination_owner_directory(&mut self) -> Ter {
        self.events
            .push("remove_destination_owner_directory".to_string());
        self.destination_dir_result
    }

    fn source_account_exists(&mut self) -> bool {
        self.events.push("source_account_exists".to_string());
        self.source_account_exists
    }

    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        self.events
            .push("apply_refund_to_source_account".to_string());
        self.refund_drops = Some(refund_drops);
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.events
            .push(format!("adjust_source_owner_count:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn erase_channel(&mut self) {
        self.events.push("erase_channel".to_string());
    }

    fn destination_exists(&mut self) -> bool {
        self.events.push("destination_exists".to_string());
        true
    }

    fn verify_deposit_preauth(&mut self) -> Ter {
        self.events.push("verify_deposit_preauth".to_string());
        self.deposit_preauth_result
    }

    fn set_channel_balance(&mut self, balance_drops: u64) {
        self.events
            .push(format!("set_channel_balance:{balance_drops}"));
    }

    fn add_destination_balance(&mut self, delta_drops: u64) {
        self.events
            .push(format!("add_destination_balance:{delta_drops}"));
    }

    fn persist_destination_balance(&mut self) {
        self.events.push("persist_destination_balance".to_string());
    }

    fn persist_channel_balance(&mut self) {
        self.events.push("persist_channel_balance".to_string());
    }

    fn clear_expiration(&mut self) {
        self.events.push("clear_expiration".to_string());
    }

    fn set_expiration(&mut self, expiration: u32) {
        self.events.push("set_expiration".to_string());
        self.expirations.push(expiration);
    }
}

fn loaded_channel() -> PaymentChannelClaimLoadedChannelFacts<u32, u32> {
    PaymentChannelClaimLoadedChannelFacts {
        source_account: 1,
        destination_account: 2,
        cancel_after: None,
        current_expiration: None,
        settle_delay: 30,
        channel_amount_drops: 1_000,
        channel_balance_drops: 250,
        destination_owner_directory_present: true,
    }
}

fn loaded_tx() -> PaymentChannelClaimLoadedTxFacts<u32> {
    PaymentChannelClaimLoadedTxFacts {
        tx_account: 1,
        balance_present: true,
        signature_present: true,
        provided_public_key_matches_channel: true,
        requested_balance_drops: 400,
        renew_flag: false,
        close_flag: true,
    }
}

fn prepared_apply_facts() -> PaymentChannelClaimLoadedPreparedApplyFacts<u32> {
    PaymentChannelClaimLoadedPreparedApplyFacts {
        apply_facts: build_payment_channel_claim_apply_facts(loaded_channel(), loaded_tx(), 60_u32),
        destination_exists: true,
    }
}

fn prepared_payment_facts()
-> tx::payment_channel_claim_loaded::PaymentChannelClaimLoadedPreparedPaymentFacts<u32> {
    build_payment_channel_claim_loaded_prepared_payment_facts(
        build_payment_channel_claim_apply_facts(loaded_channel(), loaded_tx(), 60_u32),
    )
}

#[test]
fn loaded_claim_apply_returns_no_target_before_sink_work() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_claim_loaded_do_apply(None, loaded_tx(), 60_u32, &mut sink);

    assert_eq!(result, Ter::TEC_NO_TARGET);
    assert!(sink.events.is_empty());
}

#[test]
fn loaded_claim_apply_delegates_loaded_facts_to_claim_do_apply() {
    let mut helper_sink = TestApplySink::new();
    let helper_result = run_payment_channel_claim_loaded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        60_u32,
        &mut helper_sink,
    );

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_claim_do_apply(
        build_payment_channel_claim_apply_facts(loaded_channel(), loaded_tx(), 60_u32),
        &mut direct_sink,
    );

    assert_eq!(helper_result, direct_result);
    assert_eq!(helper_sink.events, direct_sink.events);
    assert_eq!(
        helper_sink.events,
        [
            "destination_exists",
            "verify_deposit_preauth",
            "set_channel_balance:400",
            "add_destination_balance:150",
            "persist_destination_balance",
            "persist_channel_balance",
            "set_expiration"
        ]
    );
    assert_eq!(helper_sink.expirations, vec![90]);
}

#[test]
fn prepared_loaded_claim_apply_matches_direct_claim_do_apply() {
    let mut helper_sink = TestApplySink::new();
    let helper_result = run_payment_channel_claim_prepared_loaded_do_apply(
        prepared_apply_facts().apply_facts,
        &mut helper_sink,
    );

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_claim_do_apply(
        build_payment_channel_claim_apply_facts(loaded_channel(), loaded_tx(), 60_u32),
        &mut direct_sink,
    );

    assert_eq!(helper_result, direct_result);
    assert_eq!(helper_sink.events, direct_sink.events);
    assert_eq!(helper_sink.expirations, direct_sink.expirations);
}

#[test]
fn prepared_loaded_claim_payment_apply_matches_direct_claim_do_apply() {
    let mut helper_sink = TestApplySink::new();
    let helper_result = run_payment_channel_claim_loaded_prepared_payment_do_apply(
        prepared_payment_facts(),
        &mut helper_sink,
    );

    assert_eq!(helper_result, Ter::TES_SUCCESS);
    assert_eq!(
        helper_sink.events,
        [
            "destination_exists",
            "verify_deposit_preauth",
            "set_channel_balance:400",
            "add_destination_balance:150",
            "persist_destination_balance",
            "persist_channel_balance"
        ]
    );
    assert!(helper_sink.expirations.is_empty());
}
