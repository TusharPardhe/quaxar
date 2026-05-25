//! Integration tests that pin the shared payment-channel close helper to the
//! current C++ `closeChannel(...)` behavior.

use std::cell::Cell;

use protocol::Ter;
use tx::{
    PaymentChannelCloseHelperFacts, PaymentChannelCloseHelperSink, run_payment_channel_close_helper,
};

#[derive(Debug, Default)]
struct TestSink {
    source_dir_result: Ter,
    destination_dir_result: Ter,
    source_account_exists: bool,
    events: Vec<String>,
    refund_drops: Option<u64>,
    owner_count_deltas: Vec<i32>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            source_dir_result: Ter::TES_SUCCESS,
            destination_dir_result: Ter::TES_SUCCESS,
            source_account_exists: true,
            events: Vec::new(),
            refund_drops: None,
            owner_count_deltas: Vec::new(),
        }
    }
}

impl PaymentChannelCloseHelperSink for TestSink {
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

#[test]
fn payment_channel_close_helper_ordering() {
    let mut sink = TestSink::new();

    let result = run_payment_channel_close_helper(
        PaymentChannelCloseHelperFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "remove_source_owner_directory",
            "remove_destination_owner_directory",
            "source_account_exists",
            "apply_refund_to_source_account",
            "adjust_source_owner_count:-1",
            "erase_channel"
        ]
    );
    assert_eq!(sink.refund_drops, Some(750));
    assert_eq!(sink.owner_count_deltas, [-1]);
}

#[test]
fn payment_channel_close_helper_skips_destination_removal_when_absent() {
    let mut sink = TestSink::new();

    let result = run_payment_channel_close_helper(
        PaymentChannelCloseHelperFacts {
            destination_owner_directory_present: false,
            channel_amount_drops: 100,
            channel_balance_drops: 40,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "remove_source_owner_directory",
            "source_account_exists",
            "apply_refund_to_source_account",
            "adjust_source_owner_count:-1",
            "erase_channel"
        ]
    );
    assert_eq!(sink.refund_drops, Some(60));
}

#[test]
fn payment_channel_close_helper_maps_directory_failures() {
    let mut source_failure = TestSink::new();
    source_failure.source_dir_result = Ter::TEC_NO_DST;

    let source_result = run_payment_channel_close_helper(
        PaymentChannelCloseHelperFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 100,
            channel_balance_drops: 10,
        },
        &mut source_failure,
    );

    assert_eq!(source_result, Ter::TEF_BAD_LEDGER);
    assert_eq!(source_failure.events, ["remove_source_owner_directory"]);

    let mut destination_failure = TestSink::new();
    destination_failure.destination_dir_result = Ter::TEC_NO_DST;

    let destination_result = run_payment_channel_close_helper(
        PaymentChannelCloseHelperFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 100,
            channel_balance_drops: 10,
        },
        &mut destination_failure,
    );

    assert_eq!(destination_result, Ter::TEF_BAD_LEDGER);
    assert_eq!(
        destination_failure.events,
        [
            "remove_source_owner_directory",
            "remove_destination_owner_directory"
        ]
    );
}

#[test]
fn payment_channel_close_helper_maps_missing_source_account() {
    let mut sink = TestSink::new();
    sink.source_account_exists = false;

    let result = run_payment_channel_close_helper(
        PaymentChannelCloseHelperFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 100,
            channel_balance_drops: 10,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(
        sink.events,
        [
            "remove_source_owner_directory",
            "remove_destination_owner_directory",
            "source_account_exists"
        ]
    );
}

#[test]
fn payment_channel_close_helper_refund_is_amount_minus_balance() {
    let mut sink = TestSink::new();

    let called = Cell::new(false);
    let result = run_payment_channel_close_helper(
        PaymentChannelCloseHelperFacts {
            destination_owner_directory_present: false,
            channel_amount_drops: 1_234,
            channel_balance_drops: 234,
        },
        &mut sink,
    );
    called.set(true);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(called.get());
    assert_eq!(sink.refund_drops, Some(1_000));
}
