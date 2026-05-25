//! Integration tests that pin the higher loaded-state `PaymentChannelFund.cpp`
//! wrapper to the current C++ behavior.

use protocol::Ter;
use tx::payment_channel_due::PaymentChannelDueFacts;
use tx::payment_channel_fund::{PaymentChannelFundApplyFacts, PaymentChannelFundApplySink};
use tx::payment_channel_fund_loaded::{
    PaymentChannelFundLoadedChannelFacts, PaymentChannelFundLoadedTxFacts,
};
use tx::payment_channel_fund_loaded_apply::{
    PaymentChannelFundLoadedGuardFacts, run_payment_channel_fund_loaded_do_apply,
    run_payment_channel_fund_prepared_loaded_do_apply,
};
use tx::payment_channel_helpers::PaymentChannelCloseFacts;
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

fn guard_facts() -> PaymentChannelFundLoadedGuardFacts {
    PaymentChannelFundLoadedGuardFacts {
        owner_account_exists: true,
        owner_balance_covers_reserve: true,
        owner_balance_covers_reserve_plus_amount: true,
        destination_exists: true,
    }
}

fn prepared_apply_facts() -> PaymentChannelFundApplyFacts<u32> {
    PaymentChannelFundApplyFacts {
        channel_exists: true,
        due_facts: PaymentChannelDueFacts {
            cancel_after: None,
            expiration: None,
            close_time: 50,
        },
        close_facts: PaymentChannelCloseFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
        },
        tx_account_is_owner: true,
        channel_amount_drops: 1_000,
        fund_amount_drops: 300,
        extend_expiration: Some(90),
        min_extend_expiration: 80,
        owner_account_exists: true,
        owner_balance_covers_reserve: true,
        owner_balance_covers_reserve_plus_amount: true,
        destination_exists: true,
    }
}

#[test]
fn loaded_fund_do_apply_returns_no_entry_before_sink_work() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_do_apply(
        None::<PaymentChannelFundLoadedChannelFacts<u32, u32>>,
        loaded_tx(),
        50_u32,
        guard_facts(),
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert!(sink.events.is_empty());
}

#[test]
fn loaded_fund_do_apply_delegates_owner_guard_errors() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        PaymentChannelFundLoadedGuardFacts {
            owner_balance_covers_reserve_plus_amount: false,
            ..guard_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_UNFUNDED);
    assert_eq!(sink.events, ["update_expiration"]);
    assert_eq!(sink.updated_expiration, Some(90));
}

#[test]
fn loaded_fund_do_apply_preserves_success_order() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        guard_facts(),
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "update_expiration",
            "set_channel_amount:1300",
            "persist_channel",
            "subtract_owner_balance:300",
            "persist_owner"
        ]
    );
    assert_eq!(sink.updated_expiration, Some(90));
}

#[test]
fn prepared_loaded_fund_do_apply_delegates_guard_failures() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_prepared_loaded_do_apply(
        PaymentChannelFundApplyFacts {
            owner_balance_covers_reserve_plus_amount: false,
            ..prepared_apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_UNFUNDED);
    assert_eq!(sink.events, ["update_expiration"]);
    assert_eq!(sink.updated_expiration, Some(90));
}

#[test]
fn prepared_loaded_fund_do_apply_preserves_success_order() {
    let mut sink = TestApplySink::new();

    let result =
        run_payment_channel_fund_prepared_loaded_do_apply(prepared_apply_facts(), &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "update_expiration",
            "set_channel_amount:1300",
            "persist_channel",
            "subtract_owner_balance:300",
            "persist_owner"
        ]
    );
    assert_eq!(sink.updated_expiration, Some(90));
}
