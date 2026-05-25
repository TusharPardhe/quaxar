//! Integration tests that pin the narrowed Rust `CheckCancel.cpp` shell to the
//! current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    CheckCancelApplySink, CheckCancelPreclaimFacts, run_check_cancel_do_apply,
    run_check_cancel_preclaim, run_check_cancel_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    check_exists: bool,
    source_matches_destination: bool,
    remove_destination_dir: bool,
    remove_owner_dir: bool,
    events: Vec<String>,
    owner_count_delta: Vec<i32>,
    erased: bool,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            check_exists: true,
            source_matches_destination: false,
            remove_destination_dir: true,
            remove_owner_dir: true,
            events: Vec::new(),
            owner_count_delta: Vec::new(),
            erased: false,
        }
    }
}

impl CheckCancelApplySink for TestApplySink {
    fn check_exists(&mut self) -> bool {
        self.events.push("check_exists".to_string());
        self.check_exists
    }

    fn check_source_matches_destination(&mut self) -> bool {
        self.events.push("same_account".to_string());
        self.source_matches_destination
    }

    fn remove_destination_dir(&mut self) -> bool {
        self.events.push("remove_destination".to_string());
        self.remove_destination_dir
    }

    fn remove_owner_dir(&mut self) -> bool {
        self.events.push("remove_owner".to_string());
        self.remove_owner_dir
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_delta.push(delta);
    }

    fn erase_check(&mut self) {
        self.events.push("erase".to_string());
        self.erased = true;
    }
}

#[test]
fn check_cancel_preflight_is_noop() {
    assert_eq!(run_check_cancel_preflight(), Ter::TES_SUCCESS);
}

#[test]
fn check_cancel_preclaim_maps_missing_check() {
    let result = run_check_cancel_preclaim(CheckCancelPreclaimFacts {
        check_exists: false,
        check_expired: false,
        tx_account_is_check_source: false,
        tx_account_is_check_destination: false,
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn check_cancel_preclaim_rejects_unexpired_outsider() {
    let result = run_check_cancel_preclaim(CheckCancelPreclaimFacts {
        check_exists: true,
        check_expired: false,
        tx_account_is_check_source: false,
        tx_account_is_check_destination: false,
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
}

#[test]
fn check_cancel_preclaim_allows_expired_outsider() {
    let result = run_check_cancel_preclaim(CheckCancelPreclaimFacts {
        check_exists: true,
        check_expired: true,
        tx_account_is_check_source: false,
        tx_account_is_check_destination: false,
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn check_cancel_do_apply_maps_missing_check() {
    let mut sink = TestApplySink::new();
    sink.check_exists = false;

    let result = run_check_cancel_do_apply(&mut sink);

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(sink.events, ["check_exists"]);
}

#[test]
fn check_cancel_do_apply_maps_destination_remove_failure() {
    let mut sink = TestApplySink::new();
    sink.remove_destination_dir = false;

    let result = run_check_cancel_do_apply(&mut sink);

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(result), "tefBAD_LEDGER");
    assert_eq!(
        sink.events,
        ["check_exists", "same_account", "remove_destination"]
    );
}

#[test]
fn check_cancel_do_apply_skips_destination_remove_for_self_check() {
    let mut sink = TestApplySink::new();
    sink.source_matches_destination = true;

    let result = run_check_cancel_do_apply(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "check_exists",
            "same_account",
            "remove_owner",
            "adjust:-1",
            "erase"
        ]
    );
}

#[test]
fn check_cancel_do_apply_maps_owner_remove_failure() {
    let mut sink = TestApplySink::new();
    sink.remove_owner_dir = false;

    let result = run_check_cancel_do_apply(&mut sink);

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(
        sink.events,
        [
            "check_exists",
            "same_account",
            "remove_destination",
            "remove_owner"
        ]
    );
}

#[test]
fn check_cancel_do_apply_preserves_current_success_order() {
    let mut sink = TestApplySink::new();

    let result = run_check_cancel_do_apply(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "check_exists",
            "same_account",
            "remove_destination",
            "remove_owner",
            "adjust:-1",
            "erase"
        ]
    );
    assert_eq!(sink.owner_count_delta, vec![-1]);
    assert!(sink.erased);
}
