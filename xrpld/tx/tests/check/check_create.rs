//! Integration tests that pin the narrowed Rust `CheckCreate.cpp` shell to
//! the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    CheckCreateApplyFacts, CheckCreateApplySink, CheckCreateMutation, CheckCreatePreclaimFacts,
    CheckCreatePreflightFacts, run_check_create_do_apply, run_check_create_preclaim,
    run_check_create_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    source_account_exists: bool,
    reserve_sufficient: bool,
    destination_dir_page: Option<u64>,
    owner_dir_page: Option<u64>,
    created: Option<CheckCreateMutation>,
    owner_count_deltas: Vec<i32>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            source_account_exists: true,
            reserve_sufficient: true,
            destination_dir_page: Some(11),
            owner_dir_page: Some(22),
            created: None,
            owner_count_deltas: Vec::new(),
        }
    }
}

impl CheckCreateApplySink for TestApplySink {
    fn source_account_exists(&mut self) -> bool {
        self.source_account_exists
    }

    fn reserve_sufficient(&mut self) -> bool {
        self.reserve_sufficient
    }

    fn insert_destination_dir(&mut self) -> Option<u64> {
        self.destination_dir_page
    }

    fn insert_owner_dir(&mut self) -> Option<u64> {
        self.owner_dir_page
    }

    fn create_check(&mut self, mutation: CheckCreateMutation) {
        self.created = Some(mutation);
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.owner_count_deltas.push(delta);
    }
}

fn preflight_facts() -> CheckCreatePreflightFacts {
    CheckCreatePreflightFacts {
        tx_account_is_destination: false,
        send_max_is_legal: true,
        send_max_signum_positive: true,
        send_max_currency_is_bad: false,
        expiration: None,
    }
}

fn preclaim_facts() -> CheckCreatePreclaimFacts {
    CheckCreatePreclaimFacts {
        destination_exists: true,
        destination_disallow_incoming_check: false,
        destination_is_pseudo_account: false,
        destination_require_dest_tag: false,
        tx_has_destination_tag: false,
        send_max_is_native: true,
        send_max_issuer_is_source: false,
        send_max_issuer_is_destination: false,
        send_max_issuer_globally_frozen: false,
        source_to_issuer_trustline_frozen: false,
        issuer_to_destination_trustline_frozen: false,
        tx_expired: false,
    }
}

fn apply_facts() -> CheckCreateApplyFacts {
    CheckCreateApplyFacts {
        source_account: "alice".to_string(),
        destination_account: "bob".to_string(),
        sequence: 7,
        destination_equals_source: false,
        send_max: "USD:50".to_string(),
        source_tag: Some(2),
        destination_tag: Some(3),
        invoice_id: Some([4; 32]),
        expiration: Some(9),
    }
}

#[test]
fn check_create_preflight_rejects_check_to_self() {
    let result = run_check_create_preflight(CheckCreatePreflightFacts {
        tx_account_is_destination: true,
        send_max_is_legal: false,
        send_max_signum_positive: false,
        send_max_currency_is_bad: true,
        expiration: Some(0),
    });

    assert_eq!(result, Ter::TEM_REDUNDANT);
    assert_eq!(trans_token(result), "temREDUNDANT");
}

#[test]
fn check_create_preflight_rejects_bad_amount_before_currency() {
    let result = run_check_create_preflight(CheckCreatePreflightFacts {
        send_max_is_legal: false,
        send_max_currency_is_bad: true,
        ..preflight_facts()
    });

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
    assert_eq!(trans_token(result), "temBAD_AMOUNT");
}

#[test]
fn check_create_preflight_rejects_bad_currency() {
    let result = run_check_create_preflight(CheckCreatePreflightFacts {
        send_max_currency_is_bad: true,
        ..preflight_facts()
    });

    assert_eq!(result, Ter::TEM_BAD_CURRENCY);
    assert_eq!(trans_token(result), "temBAD_CURRENCY");
}

#[test]
fn check_create_preflight_rejects_zero_expiration() {
    let result = run_check_create_preflight(CheckCreatePreflightFacts {
        expiration: Some(0),
        ..preflight_facts()
    });

    assert_eq!(result, Ter::TEM_BAD_EXPIRATION);
    assert_eq!(trans_token(result), "temBAD_EXPIRATION");
}

#[test]
fn check_create_preclaim_rejects_missing_destination() {
    let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
        destination_exists: false,
        destination_disallow_incoming_check: true,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_DST);
    assert_eq!(trans_token(result), "tecNO_DST");
}

#[test]
fn check_create_preclaim_rejects_disallowed_destination() {
    let disallow = run_check_create_preclaim(CheckCreatePreclaimFacts {
        destination_disallow_incoming_check: true,
        ..preclaim_facts()
    });
    let pseudo = run_check_create_preclaim(CheckCreatePreclaimFacts {
        destination_is_pseudo_account: true,
        ..preclaim_facts()
    });

    assert_eq!(disallow, Ter::TEC_NO_PERMISSION);
    assert_eq!(pseudo, Ter::TEC_NO_PERMISSION);
}

#[test]
fn check_create_preclaim_rejects_missing_destination_tag() {
    let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
        destination_require_dest_tag: true,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
    assert_eq!(trans_token(result), "tecDST_TAG_NEEDED");
}

#[test]
fn check_create_preclaim_rejects_frozen_cases() {
    let global = run_check_create_preclaim(CheckCreatePreclaimFacts {
        send_max_is_native: false,
        send_max_issuer_globally_frozen: true,
        ..preclaim_facts()
    });
    let source_line = run_check_create_preclaim(CheckCreatePreclaimFacts {
        send_max_is_native: false,
        source_to_issuer_trustline_frozen: true,
        ..preclaim_facts()
    });
    let dest_line = run_check_create_preclaim(CheckCreatePreclaimFacts {
        send_max_is_native: false,
        issuer_to_destination_trustline_frozen: true,
        ..preclaim_facts()
    });

    assert_eq!(global, Ter::TEC_FROZEN);
    assert_eq!(source_line, Ter::TEC_FROZEN);
    assert_eq!(dest_line, Ter::TEC_FROZEN);
}

#[test]
fn check_create_preclaim_rejects_expired_check() {
    let result = run_check_create_preclaim(CheckCreatePreclaimFacts {
        tx_expired: true,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_EXPIRED);
    assert_eq!(trans_token(result), "tecEXPIRED");
}

#[test]
fn check_create_do_apply_maps_missing_source() {
    let mut sink = TestApplySink::new();
    sink.source_account_exists = false;

    let result = run_check_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
}

#[test]
fn check_create_do_apply_maps_insufficient_reserve() {
    let mut sink = TestApplySink::new();
    sink.reserve_sufficient = false;

    let result = run_check_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
}

#[test]
fn check_create_do_apply_maps_destination_dir_failure() {
    let mut sink = TestApplySink::new();
    sink.destination_dir_page = None;

    let result = run_check_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(result), "tecDIR_FULL");
}

#[test]
fn check_create_do_apply_maps_owner_dir_failure() {
    let mut sink = TestApplySink::new();
    sink.owner_dir_page = None;

    let result = run_check_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEC_DIR_FULL);
}

#[test]
fn check_create_do_apply_preserves_optional_fields() {
    let mut sink = TestApplySink::new();

    let result = run_check_create_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.owner_count_deltas, vec![1]);
    let created = sink.created.expect("created check");
    assert_eq!(created.destination_node, Some(11));
    assert_eq!(created.owner_node, 22);
    assert_eq!(created.source_tag, Some(2));
    assert_eq!(created.destination_tag, Some(3));
    assert_eq!(created.invoice_id, Some([4; 32]));
    assert_eq!(created.expiration, Some(9));
}
