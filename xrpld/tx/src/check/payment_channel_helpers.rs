//! Shared helper surface for the reference implementation.
//!
//! This module keeps the shared close-channel ordering explicit while leaving
//! the sink-driven keylet, journal, and ledger updates at the helper edge.

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentChannelCloseHelperFacts {
    pub destination_owner_directory_present: bool,
    pub channel_amount_drops: u64,
    pub channel_balance_drops: u64,
}

pub trait PaymentChannelCloseHelperSink {
    fn remove_source_owner_directory(&mut self) -> Ter;
    fn remove_destination_owner_directory(&mut self) -> Ter;
    fn source_account_exists(&mut self) -> bool;
    fn apply_refund_to_source_account(&mut self, refund_drops: u64);
    fn adjust_source_owner_count(&mut self, delta: i32);
    fn erase_channel(&mut self);
}

pub use PaymentChannelCloseHelperFacts as PaymentChannelCloseFacts;
pub use PaymentChannelCloseHelperSink as PaymentChannelCloseSink;

pub fn run_payment_channel_close_helper<S: PaymentChannelCloseHelperSink>(
    facts: PaymentChannelCloseHelperFacts,
    sink: &mut S,
) -> Ter {
    if sink.remove_source_owner_directory() != Ter::TES_SUCCESS {
        return Ter::TEF_BAD_LEDGER;
    }

    if facts.destination_owner_directory_present
        && sink.remove_destination_owner_directory() != Ter::TES_SUCCESS
    {
        return Ter::TEF_BAD_LEDGER;
    }

    if !sink.source_account_exists() {
        return Ter::TEF_INTERNAL;
    }

    debug_assert!(
        facts.channel_amount_drops >= facts.channel_balance_drops,
        "payment channel close refund must not underflow"
    );
    let refund_drops = facts
        .channel_amount_drops
        .saturating_sub(facts.channel_balance_drops);

    sink.apply_refund_to_source_account(refund_drops);
    sink.adjust_source_owner_count(-1);
    sink.erase_channel();
    Ter::TES_SUCCESS
}

pub use run_payment_channel_close_helper as run_payment_channel_close;

#[cfg(test)]
mod tests {
    use super::{
        PaymentChannelCloseHelperFacts, PaymentChannelCloseHelperSink,
        run_payment_channel_close_helper,
    };
    use protocol::Ter;

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
    fn close_channel_removes_directories_before_account_lookup_and_refund() {
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
    fn close_channel_skips_destination_removal_when_absent() {
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
    fn close_channel_maps_source_directory_failure_to_bad_ledger() {
        let mut sink = TestSink::new();
        sink.source_dir_result = Ter::TEC_NO_DST;

        let result = run_payment_channel_close_helper(
            PaymentChannelCloseHelperFacts {
                destination_owner_directory_present: true,
                channel_amount_drops: 100,
                channel_balance_drops: 10,
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(sink.events, ["remove_source_owner_directory"]);
    }

    #[test]
    fn close_channel_maps_destination_directory_failure_to_bad_ledger() {
        let mut sink = TestSink::new();
        sink.destination_dir_result = Ter::TEC_NO_DST;

        let result = run_payment_channel_close_helper(
            PaymentChannelCloseHelperFacts {
                destination_owner_directory_present: true,
                channel_amount_drops: 100,
                channel_balance_drops: 10,
            },
            &mut sink,
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(
            sink.events,
            [
                "remove_source_owner_directory",
                "remove_destination_owner_directory"
            ]
        );
    }

    #[test]
    fn close_channel_maps_missing_source_account_to_internal_failure() {
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
}
