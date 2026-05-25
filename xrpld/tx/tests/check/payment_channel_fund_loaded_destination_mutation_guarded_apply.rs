//! Integration tests that pin the higher loaded destination-read plus mutation
//! `PaymentChannelFund.cpp` wrapper to the current C++ behavior.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use protocol::Ter;
use tx::payment_channel_fund::run_payment_channel_fund_do_apply;
use tx::payment_channel_fund_loaded::{
    PaymentChannelFundLoadedChannelFacts, PaymentChannelFundLoadedPreparedApplyFacts,
    PaymentChannelFundLoadedTxFacts, build_payment_channel_fund_loaded_prepared_apply_facts,
};
use tx::payment_channel_fund_loaded_destination_mutation_guarded_apply::{
    PaymentChannelFundLoadedDestinationMutationApplySink,
    build_payment_channel_fund_loaded_destination_mutation_apply_facts,
    run_payment_channel_fund_loaded_destination_mutation_guarded_do_apply,
    run_payment_channel_fund_loaded_prepared_destination_mutation_guarded_do_apply,
};
use tx::payment_channel_helpers::PaymentChannelCloseSink;

#[derive(Debug, Default)]
struct TestApplySink {
    source_dir_result: Ter,
    destination_dir_result: Ter,
    source_account_exists: bool,
    events: Rc<RefCell<Vec<String>>>,
    updated_expiration: Option<u32>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            source_dir_result: Ter::TES_SUCCESS,
            destination_dir_result: Ter::TES_SUCCESS,
            source_account_exists: true,
            events: Rc::new(RefCell::new(Vec::new())),
            updated_expiration: None,
        }
    }

    fn events(&self) -> Vec<String> {
        self.events.borrow().clone()
    }
}

impl PaymentChannelCloseSink for TestApplySink {
    fn remove_source_owner_directory(&mut self) -> Ter {
        self.events
            .borrow_mut()
            .push("remove_source_owner_directory".to_string());
        self.source_dir_result
    }

    fn remove_destination_owner_directory(&mut self) -> Ter {
        self.events
            .borrow_mut()
            .push("remove_destination_owner_directory".to_string());
        self.destination_dir_result
    }

    fn source_account_exists(&mut self) -> bool {
        self.events
            .borrow_mut()
            .push("source_account_exists".to_string());
        self.source_account_exists
    }

    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        self.events
            .borrow_mut()
            .push(format!("apply_refund_to_source_account:{refund_drops}"));
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.events
            .borrow_mut()
            .push(format!("adjust_source_owner_count:{delta}"));
    }

    fn erase_channel(&mut self) {
        self.events.borrow_mut().push("erase_channel".to_string());
    }
}

impl PaymentChannelFundLoadedDestinationMutationApplySink<u32> for TestApplySink {
    fn update_expiration(&mut self, expiration: u32) {
        self.events
            .borrow_mut()
            .push(format!("update_expiration:{expiration}"));
        self.updated_expiration = Some(expiration);
    }

    fn apply_fund_mutation(&mut self, new_channel_amount: u64, fund_amount_drops: u64) {
        self.events.borrow_mut().push(format!(
            "apply_fund_mutation:{new_channel_amount}:{fund_amount_drops}"
        ));
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

#[derive(Debug, Clone, Copy)]
struct OwnerFacts;

fn prepared_apply_facts() -> PaymentChannelFundLoadedPreparedApplyFacts<u32> {
    build_payment_channel_fund_loaded_prepared_apply_facts(
        loaded_channel(),
        loaded_tx(),
        50_u32,
        true,
        true,
        true,
        true,
    )
}

#[test]
fn loaded_destination_mutation_guarded_fund_apply_returns_no_entry_before_lookups() {
    let owner_called = Cell::new(false);
    let destination_called = Cell::new(false);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_destination_mutation_guarded_do_apply(
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
    assert!(sink.events().is_empty());
}

#[test]
fn loaded_destination_mutation_guarded_fund_apply_skips_destination_on_owner_error() {
    let destination_called = Cell::new(false);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_destination_mutation_guarded_do_apply(
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
    assert!(sink.events().is_empty());
}

#[test]
fn loaded_destination_mutation_guarded_fund_apply_returns_no_dst_before_mutation() {
    let destination_called = Cell::new(false);
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_loaded_destination_mutation_guarded_do_apply(
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
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_DST);
    assert!(destination_called.get());
    assert_eq!(sink.events(), ["update_expiration:90"]);
}

#[test]
fn loaded_destination_mutation_guarded_fund_apply_matches_direct_mutation_order() {
    let mut sink = TestApplySink::new();

    let helper_result = run_payment_channel_fund_loaded_destination_mutation_guarded_do_apply(
        Some(loaded_channel()),
        loaded_tx(),
        50_u32,
        || Some(OwnerFacts),
        |_| true,
        |_| true,
        || true,
        &mut sink,
    );

    let facts = build_payment_channel_fund_loaded_destination_mutation_apply_facts(
        loaded_channel(),
        loaded_tx(),
        50_u32,
        true,
    );

    #[derive(Debug, Default)]
    struct DirectSink {
        events: Vec<String>,
    }

    impl PaymentChannelCloseSink for DirectSink {
        fn remove_source_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_source_owner_directory".to_string());
            Ter::TES_SUCCESS
        }
        fn remove_destination_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_destination_owner_directory".to_string());
            Ter::TES_SUCCESS
        }
        fn source_account_exists(&mut self) -> bool {
            self.events.push("source_account_exists".to_string());
            true
        }
        fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
            self.events
                .push(format!("apply_refund_to_source_account:{refund_drops}"));
        }
        fn adjust_source_owner_count(&mut self, delta: i32) {
            self.events
                .push(format!("adjust_source_owner_count:{delta}"));
        }
        fn erase_channel(&mut self) {
            self.events.push("erase_channel".to_string());
        }
    }

    impl tx::payment_channel_fund::PaymentChannelFundApplySink<u32> for DirectSink {
        fn update_expiration(&mut self, expiration: u32) {
            self.events.push(format!("update_expiration:{expiration}"));
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

    let mut direct_sink = DirectSink::default();
    let direct_result = run_payment_channel_fund_do_apply(facts, &mut direct_sink);

    assert_eq!(helper_result, direct_result);
    assert_eq!(
        sink.events(),
        ["update_expiration:90", "apply_fund_mutation:1300:300"]
    );
    assert_eq!(
        direct_sink.events,
        [
            "update_expiration:90",
            "set_channel_amount:1300",
            "persist_channel",
            "subtract_owner_balance:300",
            "persist_owner"
        ]
    );
}

#[test]
fn prepared_loaded_destination_mutation_guarded_fund_apply_matches_direct() {
    let mut prepared_sink = TestApplySink::new();
    let prepared_result =
        run_payment_channel_fund_loaded_prepared_destination_mutation_guarded_do_apply(
            prepared_apply_facts(),
            &mut prepared_sink,
        );

    let facts = build_payment_channel_fund_loaded_destination_mutation_apply_facts(
        loaded_channel(),
        loaded_tx(),
        50_u32,
        true,
    );

    #[derive(Debug, Default)]
    struct DirectSink {
        events: Vec<String>,
    }

    impl PaymentChannelCloseSink for DirectSink {
        fn remove_source_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_source_owner_directory".to_string());
            Ter::TES_SUCCESS
        }
        fn remove_destination_owner_directory(&mut self) -> Ter {
            self.events
                .push("remove_destination_owner_directory".to_string());
            Ter::TES_SUCCESS
        }
        fn source_account_exists(&mut self) -> bool {
            self.events.push("source_account_exists".to_string());
            true
        }
        fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
            self.events
                .push(format!("apply_refund_to_source_account:{refund_drops}"));
        }
        fn adjust_source_owner_count(&mut self, delta: i32) {
            self.events
                .push(format!("adjust_source_owner_count:{delta}"));
        }
        fn erase_channel(&mut self) {
            self.events.push("erase_channel".to_string());
        }
    }

    impl tx::payment_channel_fund::PaymentChannelFundApplySink<u32> for DirectSink {
        fn update_expiration(&mut self, expiration: u32) {
            self.events.push(format!("update_expiration:{expiration}"));
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

    let mut direct_sink = DirectSink::default();
    let direct_result = run_payment_channel_fund_do_apply(facts, &mut direct_sink);

    assert_eq!(prepared_result, direct_result);
    assert_eq!(
        prepared_sink.events(),
        ["update_expiration:90", "apply_fund_mutation:1300:300"]
    );
    assert_eq!(
        direct_sink.events,
        [
            "update_expiration:90",
            "set_channel_amount:1300",
            "persist_channel",
            "subtract_owner_balance:300",
            "persist_owner"
        ]
    );
}
