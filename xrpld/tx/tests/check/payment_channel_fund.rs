//! Integration tests that pin the narrowed Rust `PaymentChannelFund.cpp` shell
//! to the current C++ behavior.

use protocol::{SeqProxy, Ter, trans_token};
use tx::payment_channel_due::{PaymentChannelDueFacts, is_payment_channel_due};
use tx::payment_channel_fund::{
    PaymentChannelFundPreparedDoApplyFacts, build_payment_channel_fund_prepared_do_apply_facts,
    run_payment_channel_fund_prepared_do_apply,
};
use tx::payment_channel_helpers::{
    PaymentChannelCloseFacts, PaymentChannelCloseSink, run_payment_channel_close,
};
use tx::{
    PaymentChannelFundApplyFacts, PaymentChannelFundApplySink, TxConsequences,
    run_payment_channel_fund_do_apply, run_payment_channel_fund_make_tx_consequences,
    run_payment_channel_fund_min_extend_expiration, run_payment_channel_fund_preflight,
};

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

fn apply_facts() -> PaymentChannelFundApplyFacts<u32> {
    PaymentChannelFundApplyFacts {
        channel_exists: true,
        due_facts: PaymentChannelDueFacts {
            cancel_after: None,
            expiration: None,
            close_time: 0,
        },
        close_facts: PaymentChannelCloseFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
        },
        tx_account_is_owner: true,
        channel_amount_drops: 1_000,
        fund_amount_drops: 300,
        extend_expiration: None,
        min_extend_expiration: 80,
        owner_account_exists: true,
        owner_balance_covers_reserve: true,
        owner_balance_covers_reserve_plus_amount: true,
        destination_exists: true,
    }
}

fn prepared_do_apply_facts() -> PaymentChannelFundPreparedDoApplyFacts<u32> {
    build_payment_channel_fund_prepared_do_apply_facts(apply_facts())
}

#[test]
fn payment_channel_fund_make_tx_consequences_tracks_xrp_spend() {
    let consequences =
        run_payment_channel_fund_make_tx_consequences(12, SeqProxy::sequence(7), 400);

    assert_eq!(
        consequences,
        TxConsequences::with_potential_spend(12, SeqProxy::sequence(7), 400)
    );
}

#[test]
fn payment_channel_fund_preflight_rejects_non_xrp_or_non_positive_amount() {
    let not_xrp = run_payment_channel_fund_preflight(false, true);
    let not_positive = run_payment_channel_fund_preflight(true, false);

    assert_eq!(not_xrp, Ter::TEM_BAD_AMOUNT);
    assert_eq!(not_positive, Ter::TEM_BAD_AMOUNT);
    assert_eq!(trans_token(not_xrp), "temBAD_AMOUNT");
}

#[test]
fn payment_channel_fund_due_helper_rule() {
    assert!(is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: Some(10_u32),
        expiration: None,
        close_time: 10,
    }));
    assert!(is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: None,
        expiration: Some(10_u32),
        close_time: 11,
    }));
    assert!(!is_payment_channel_due(PaymentChannelDueFacts {
        cancel_after: Some(10_u32),
        expiration: Some(20),
        close_time: 9,
    }));
}

#[test]
fn payment_channel_fund_min_extend_expiration_caps_against_current_expiration() {
    assert_eq!(
        run_payment_channel_fund_min_extend_expiration(100_u32, 40, None),
        140
    );
    assert_eq!(
        run_payment_channel_fund_min_extend_expiration(100_u32, 40, Some(120)),
        120
    );
}

#[test]
fn payment_channel_fund_do_apply_maps_missing_channel() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            channel_exists: false,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert!(sink.events.is_empty());
}

#[test]
fn payment_channel_fund_do_apply_closes_past_cancel_after_before_permission() {
    let mut sink = TestApplySink::new();
    sink.destination_dir_result = Ter::TEC_NO_DST;

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            due_facts: PaymentChannelDueFacts {
                cancel_after: Some(1),
                expiration: None,
                close_time: 1,
            },
            tx_account_is_owner: false,
            ..apply_facts()
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
fn payment_channel_fund_do_apply_closes_past_expiration_before_permission() {
    let mut sink = TestApplySink::new();
    sink.destination_dir_result = Ter::TEC_NO_DST;

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            due_facts: PaymentChannelDueFacts {
                cancel_after: None,
                expiration: Some(1),
                close_time: 1,
            },
            tx_account_is_owner: false,
            ..apply_facts()
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
fn payment_channel_fund_close_helper_returns_sink_result_unchanged() {
    let mut sink = TestApplySink::new();
    sink.source_dir_result = Ter::TEM_BAD_SIGNATURE;

    let result = run_payment_channel_close(
        PaymentChannelCloseFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 1_234,
            channel_balance_drops: 234,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(sink.events, ["remove_source_owner_directory"]);
}

#[test]
fn payment_channel_fund_do_apply_rejects_non_owner() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            tx_account_is_owner: false,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert!(sink.events.is_empty());
}

#[test]
fn payment_channel_fund_do_apply_rejects_short_extension() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            extend_expiration: Some(79),
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEM_BAD_EXPIRATION);
    assert_eq!(trans_token(result), "temBAD_EXPIRATION");
    assert!(sink.events.is_empty());
}

#[test]
fn payment_channel_fund_do_apply_updates_expiration_before_owner_lookup() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            extend_expiration: Some(90),
            owner_account_exists: false,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(sink.events, ["update_expiration"]);
    assert_eq!(sink.updated_expiration, Some(90));
}

#[test]
fn payment_channel_fund_do_apply_checks_reserve_then_funds() {
    let mut reserve_sink = TestApplySink::new();
    let reserve = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            owner_balance_covers_reserve: false,
            ..apply_facts()
        },
        &mut reserve_sink,
    );

    let mut unfunded_sink = TestApplySink::new();
    let unfunded = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            owner_balance_covers_reserve_plus_amount: false,
            ..apply_facts()
        },
        &mut unfunded_sink,
    );

    assert_eq!(reserve, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(unfunded, Ter::TEC_UNFUNDED);
    assert!(reserve_sink.events.is_empty());
    assert!(unfunded_sink.events.is_empty());
}

#[test]
fn payment_channel_fund_do_apply_requires_destination_before_mutation() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            destination_exists: false,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_DST);
    assert!(sink.events.is_empty());
}

#[test]
fn payment_channel_fund_do_apply_runs_success_mutation_order() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_fund_do_apply(
        PaymentChannelFundApplyFacts {
            extend_expiration: Some(90),
            ..apply_facts()
        },
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
fn payment_channel_fund_prepared_do_apply_matches_direct_path() {
    let mut prepared_sink = TestApplySink::new();
    let prepared_result =
        run_payment_channel_fund_prepared_do_apply(prepared_do_apply_facts(), &mut prepared_sink);

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_fund_do_apply(apply_facts(), &mut direct_sink);

    assert_eq!(prepared_result, direct_result);
    assert_eq!(prepared_sink.events, direct_sink.events);
    assert_eq!(prepared_sink.refund_drops, direct_sink.refund_drops);
    assert_eq!(
        prepared_sink.owner_count_deltas,
        direct_sink.owner_count_deltas
    );
    assert_eq!(
        prepared_sink.updated_expiration,
        direct_sink.updated_expiration
    );
}
