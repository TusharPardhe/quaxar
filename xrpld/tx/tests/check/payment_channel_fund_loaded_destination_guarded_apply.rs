//! Integration tests that pin the higher loaded destination-read
//! `PaymentChannelFund.cpp` wrapper to the current C++ behavior.

use std::cell::Cell;

use protocol::Ter;
use tx::payment_channel_fund::PaymentChannelFundApplySink;
use tx::payment_channel_fund_loaded::{
    PaymentChannelFundLoadedChannelFacts, PaymentChannelFundLoadedTxFacts,
};
use tx::payment_channel_fund_loaded_destination_guarded_apply::run_payment_channel_fund_loaded_destination_guarded_do_apply;
use tx::payment_channel_fund_loaded_guarded_apply::run_payment_channel_fund_loaded_guarded_do_apply;
use tx::payment_channel_helpers::PaymentChannelCloseSink;

#[derive(Debug, Default)]
struct TestApplySink {
    source_dir_result: Ter,
    destination_dir_result: Ter,
    source_account_exists: bool,
    events: Vec<String>,
    refund_drops: Option<u64>,
    owner_count_deltas: Vec<i32>,
    updated_expiration: Option<u32>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            source_dir_result: Ter::TES_SUCCESS,
            destination_dir_result: Ter::TES_SUCCESS,
            source_account_exists: true,
            events: Vec::new(),
            refund_drops: None,
            owner_count_deltas: Vec::new(),
            updated_expiration: None,
        }
    }
}

impl PaymentChannelCloseSink for TestApplySink {
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
}

impl PaymentChannelFundApplySink<u32> for TestApplySink {
    fn update_expiration(&mut self, expiration: u32) {
        self.events.push("update_expiration".to_string());
        self.updated_expiration = Some(expiration);
    }

    fn set_channel_amount(&mut self, amount_drops: u64) {
        self.events
            .push(format!("set_channel_amount:{amount_drops}"));
    }

    fn persist_channel(&mut self) {
        self.events.push("persist_channel".to_string());
    }

    fn subtract_owner_balance(&mut self, amount_drops: u64) {
        self.events
            .push(format!("subtract_owner_balance:{amount_drops}"));
    }

    fn persist_owner(&mut self) {
        self.events.push("persist_owner".to_string());
    }
}

#[derive(Debug, Clone, Copy)]
struct OwnerFacts;

fn loaded_channel() -> PaymentChannelFundLoadedChannelFacts<u32, u32> {
    PaymentChannelFundLoadedChannelFacts {
        source_account: 7_u32,
        cancel_after: None,
        current_expiration: None,
        settle_delay: 30,
        channel_amount_drops: 1_000,
        channel_balance_drops: 250,
        destination_owner_directory_present: true,
    }
}

fn loaded_tx() -> PaymentChannelFundLoadedTxFacts<u32, u32> {
    PaymentChannelFundLoadedTxFacts {
        tx_account: 7_u32,
        extend_expiration: Some(90_u32),
        fund_amount_drops: 300,
    }
}

#[test]
fn loaded_destination_guarded_fund_apply_returns_no_entry_before_lookups() {
    let owner_called = Cell::new(false);
    let destination_called = Cell::new(false);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_destination_guarded_do_apply(
        None::<PaymentChannelFundLoadedChannelFacts<u32, u32>>,
        loaded_tx(),
        50_u32,
        || {
            owner_called.set(true);
            Some(OwnerFacts)
        },
        |_| true,
        |_| true,
        || {
            destination_called.set(true);
            true
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert!(!owner_called.get());
    assert!(!destination_called.get());
    assert!(sink.events.is_empty());
}

#[test]
fn loaded_destination_guarded_fund_apply_skips_destination_on_owner_error() {
    let destination_called = Cell::new(false);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_destination_guarded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        || Some(OwnerFacts),
        |_| true,
        |_| false,
        || {
            destination_called.set(true);
            true
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_UNFUNDED);
    assert!(!destination_called.get());
    assert!(sink.events.is_empty());
}

#[test]
fn loaded_destination_guarded_fund_apply_returns_no_dst_before_mutation() {
    let destination_called = Cell::new(false);
    let mut helper_sink = TestApplySink::new();

    let helper_result = run_payment_channel_fund_loaded_destination_guarded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        || Some(OwnerFacts),
        |_| true,
        |_| true,
        || {
            destination_called.set(true);
            false
        },
        &mut helper_sink,
    );

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_fund_loaded_guarded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        || Some(OwnerFacts),
        |_| true,
        |_| true,
        false,
        &mut direct_sink,
    );

    assert_eq!(helper_result, Ter::TEC_NO_DST);
    assert!(destination_called.get());
    assert_eq!(helper_result, direct_result);
    assert_eq!(helper_sink.events, direct_sink.events);
    assert_eq!(
        helper_sink.updated_expiration,
        direct_sink.updated_expiration
    );
}

#[test]
fn loaded_destination_guarded_fund_apply_matches_guarded_wrapper() {
    let mut helper_sink = TestApplySink::new();
    let helper_result = run_payment_channel_fund_loaded_destination_guarded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        || Some(OwnerFacts),
        |_| true,
        |_| true,
        || true,
        &mut helper_sink,
    );

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_fund_loaded_guarded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        || Some(OwnerFacts),
        |_| true,
        |_| true,
        true,
        &mut direct_sink,
    );

    assert_eq!(helper_result, direct_result);
    assert_eq!(helper_sink.events, direct_sink.events);
    assert_eq!(
        helper_sink.updated_expiration,
        direct_sink.updated_expiration
    );
}
