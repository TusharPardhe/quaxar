//! Integration tests that pin the narrowed Rust `PaymentChannelCreate.cpp`
//! shell to the current C++ behavior.

use protocol::{SeqProxy, Ter, trans_token};
use tx::{
    PaymentChannelCreateApplyFacts, PaymentChannelCreateApplySink,
    PaymentChannelCreatePreclaimFacts, PaymentChannelCreatePreflightFacts,
    PaymentChannelCreatePreparedApplyFacts, TxConsequences,
    build_payment_channel_create_prepared_apply_facts, run_payment_channel_create_do_apply,
    run_payment_channel_create_make_tx_consequences, run_payment_channel_create_preclaim,
    run_payment_channel_create_preflight, run_payment_channel_create_prepared_do_apply,
};

#[derive(Debug, Default)]
struct TestApplySink {
    owner_dir_page: Option<u64>,
    destination_dir_page: Option<u64>,
    events: Vec<String>,
    owner_count_deltas: Vec<i32>,
    include_sequence_field: Option<bool>,
    owner_nodes: Vec<u64>,
    destination_nodes: Vec<u64>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            owner_dir_page: Some(11),
            destination_dir_page: Some(22),
            events: Vec::new(),
            owner_count_deltas: Vec::new(),
            include_sequence_field: None,
            owner_nodes: Vec::new(),
            destination_nodes: Vec::new(),
        }
    }
}

impl PaymentChannelCreateApplySink for TestApplySink {
    fn create_payment_channel_entry(&mut self, include_sequence_field: bool) {
        self.events.push("create".to_string());
        self.include_sequence_field = Some(include_sequence_field);
    }

    fn insert_owner_directory(&mut self) -> Option<u64> {
        self.events.push("owner_dir".to_string());
        self.owner_dir_page
    }

    fn set_owner_node(&mut self, page: u64) {
        self.events.push("set_owner_node".to_string());
        self.owner_nodes.push(page);
    }

    fn insert_destination_directory(&mut self) -> Option<u64> {
        self.events.push("destination_dir".to_string());
        self.destination_dir_page
    }

    fn set_destination_node(&mut self, page: u64) {
        self.events.push("set_destination_node".to_string());
        self.destination_nodes.push(page);
    }

    fn deduct_owner_balance(&mut self) {
        self.events.push("deduct_owner_balance".to_string());
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn update_owner_account(&mut self) {
        self.events.push("update_owner".to_string());
    }
}

fn preclaim_facts() -> PaymentChannelCreatePreclaimFacts {
    PaymentChannelCreatePreclaimFacts {
        source_account_exists: true,
        source_balance_covers_reserve: true,
        source_balance_covers_reserve_plus_amount: true,
        destination_exists: true,
        destination_disallow_incoming_pay_chan: false,
        destination_requires_dest_tag: false,
        destination_has_dest_tag: false,
        destination_is_pseudo_account: false,
    }
}

fn apply_facts() -> PaymentChannelCreateApplyFacts {
    PaymentChannelCreateApplyFacts {
        account_exists: true,
        fix_paychan_cancel_after_enabled: false,
        cancel_after_expired: false,
        include_sequence_field: true,
    }
}

fn prepared_apply_facts() -> PaymentChannelCreatePreparedApplyFacts {
    build_payment_channel_create_prepared_apply_facts(apply_facts())
}

#[test]
fn payment_channel_create_make_tx_consequences_tracks_xrp_spend() {
    let consequences =
        run_payment_channel_create_make_tx_consequences(12, SeqProxy::sequence(9), 444);

    assert_eq!(
        consequences,
        TxConsequences::with_potential_spend(12, SeqProxy::sequence(9), 444)
    );
}

#[test]
fn payment_channel_create_preflight_ordering() {
    assert_eq!(
        run_payment_channel_create_preflight(PaymentChannelCreatePreflightFacts {
            amount_is_xrp: false,
            amount_positive: true,
            tx_account_is_destination: false,
            public_key_valid: true,
        }),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_payment_channel_create_preflight(PaymentChannelCreatePreflightFacts {
            amount_is_xrp: true,
            amount_positive: true,
            tx_account_is_destination: true,
            public_key_valid: true,
        }),
        Ter::TEM_DST_IS_SRC
    );
    assert_eq!(
        run_payment_channel_create_preflight(PaymentChannelCreatePreflightFacts {
            amount_is_xrp: true,
            amount_positive: true,
            tx_account_is_destination: false,
            public_key_valid: false,
        }),
        Ter::TEM_MALFORMED
    );
}

#[test]
fn payment_channel_create_preclaim_guards() {
    assert_eq!(
        run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
            source_account_exists: false,
            ..preclaim_facts()
        }),
        Ter::TER_NO_ACCOUNT
    );
    assert_eq!(
        run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
            source_balance_covers_reserve: false,
            ..preclaim_facts()
        }),
        Ter::TEC_INSUFFICIENT_RESERVE
    );
    assert_eq!(
        run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
            source_balance_covers_reserve_plus_amount: false,
            ..preclaim_facts()
        }),
        Ter::TEC_UNFUNDED
    );
    assert_eq!(
        run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
            destination_exists: false,
            ..preclaim_facts()
        }),
        Ter::TEC_NO_DST
    );
    assert_eq!(
        run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
            destination_disallow_incoming_pay_chan: true,
            ..preclaim_facts()
        }),
        Ter::TEC_NO_PERMISSION
    );
    assert_eq!(
        run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
            destination_requires_dest_tag: true,
            ..preclaim_facts()
        }),
        Ter::TEC_DST_TAG_NEEDED
    );
    assert_eq!(
        run_payment_channel_create_preclaim(PaymentChannelCreatePreclaimFacts {
            destination_is_pseudo_account: true,
            ..preclaim_facts()
        }),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn payment_channel_create_do_apply_honors_fix_cancel_after_gate() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_create_do_apply(
        PaymentChannelCreateApplyFacts {
            fix_paychan_cancel_after_enabled: true,
            cancel_after_expired: true,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_EXPIRED);
    assert_eq!(trans_token(result), "tecEXPIRED");
    assert!(sink.events.is_empty());
}

#[test]
fn payment_channel_create_do_apply_skips_cancel_after_check_when_fix_disabled() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_create_do_apply(
        PaymentChannelCreateApplyFacts {
            fix_paychan_cancel_after_enabled: false,
            cancel_after_expired: true,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "create",
            "owner_dir",
            "set_owner_node",
            "destination_dir",
            "set_destination_node",
            "deduct_owner_balance",
            "adjust:1",
            "update_owner",
        ]
    );
}

#[test]
fn payment_channel_create_do_apply_maps_owner_dir_failure_before_destination() {
    let mut sink = TestApplySink::new();
    sink.owner_dir_page = None;

    let result = run_payment_channel_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(sink.events, ["create", "owner_dir"]);
}

#[test]
fn prepared_payment_channel_create_do_apply_matches_direct_path() {
    let mut prepared_sink = TestApplySink::new();
    let prepared_result =
        run_payment_channel_create_prepared_do_apply(prepared_apply_facts(), &mut prepared_sink);

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_create_do_apply(apply_facts(), &mut direct_sink);

    assert_eq!(prepared_result, direct_result);
    assert_eq!(prepared_sink.events, direct_sink.events);
    assert_eq!(
        prepared_sink.include_sequence_field,
        direct_sink.include_sequence_field
    );
    assert_eq!(
        prepared_sink.owner_count_deltas,
        direct_sink.owner_count_deltas
    );
    assert_eq!(prepared_sink.owner_nodes, direct_sink.owner_nodes);
    assert_eq!(
        prepared_sink.destination_nodes,
        direct_sink.destination_nodes
    );
}

#[test]
fn payment_channel_create_do_apply_maps_destination_dir_failure_before_tail() {
    let mut sink = TestApplySink::new();
    sink.destination_dir_page = None;

    let result = run_payment_channel_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(
        sink.events,
        ["create", "owner_dir", "set_owner_node", "destination_dir"]
    );
    assert_eq!(sink.owner_nodes, vec![11]);
}

#[test]
fn payment_channel_create_do_apply_preserves_success_order() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "create",
            "owner_dir",
            "set_owner_node",
            "destination_dir",
            "set_destination_node",
            "deduct_owner_balance",
            "adjust:1",
            "update_owner",
        ]
    );
    assert_eq!(sink.owner_nodes, vec![11]);
    assert_eq!(sink.destination_nodes, vec![22]);
    assert_eq!(sink.include_sequence_field, Some(true));
    assert_eq!(sink.owner_count_deltas, vec![1]);
}
