//! Integration tests that pin the narrowed Rust `OracleDelete.cpp` shell to
//! the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    OracleDeleteApplySink, OracleDeleteLoadedSink, OracleDeletePreclaimFacts,
    run_oracle_delete_do_apply, run_oracle_delete_loaded, run_oracle_delete_preclaim,
    run_oracle_delete_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestLoadedSink {
    oracle_exists: bool,
    remove_owner_dir: bool,
    owner_exists: bool,
    price_series_exceeds_five: bool,
    events: Vec<String>,
    owner_count_delta: Vec<i32>,
    erased: bool,
}

impl TestLoadedSink {
    fn new() -> Self {
        Self {
            oracle_exists: true,
            remove_owner_dir: true,
            owner_exists: true,
            price_series_exceeds_five: false,
            events: Vec::new(),
            owner_count_delta: Vec::new(),
            erased: false,
        }
    }
}

impl OracleDeleteLoadedSink for TestLoadedSink {
    fn oracle_exists(&mut self) -> bool {
        self.events.push("oracle_exists".to_string());
        self.oracle_exists
    }

    fn remove_owner_dir(&mut self) -> bool {
        self.events.push("remove_owner_dir".to_string());
        self.remove_owner_dir
    }

    fn owner_exists(&mut self) -> bool {
        self.events.push("owner_exists".to_string());
        self.owner_exists
    }

    fn price_series_exceeds_five(&mut self) -> bool {
        self.events.push("series_gt_five".to_string());
        self.price_series_exceeds_five
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_delta.push(delta);
    }

    fn erase_oracle(&mut self) {
        self.events.push("erase".to_string());
        self.erased = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    oracle_exists: bool,
    delete_result: Ter,
    events: Vec<String>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            oracle_exists: true,
            delete_result: Ter::TES_SUCCESS,
            events: Vec::new(),
        }
    }
}

impl OracleDeleteApplySink for TestApplySink {
    fn oracle_exists(&mut self) -> bool {
        self.events.push("peek_oracle".to_string());
        self.oracle_exists
    }

    fn delete_loaded_oracle(&mut self) -> Ter {
        self.events.push("delete_loaded".to_string());
        self.delete_result
    }
}

#[test]
fn oracle_delete_preflight_is_noop() {
    assert_eq!(run_oracle_delete_preflight(), Ter::TES_SUCCESS);
}

#[test]
fn oracle_delete_preclaim_rejects_missing_account() {
    let result = run_oracle_delete_preclaim(OracleDeletePreclaimFacts {
        account_exists: false,
        oracle_exists: false,
        tx_account_matches_owner: false,
    });

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
}

#[test]
fn oracle_delete_preclaim_rejects_missing_oracle() {
    let result = run_oracle_delete_preclaim(OracleDeletePreclaimFacts {
        account_exists: true,
        oracle_exists: false,
        tx_account_matches_owner: false,
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn oracle_delete_preclaim_rejects_owner_mismatch() {
    let result = run_oracle_delete_preclaim(OracleDeletePreclaimFacts {
        account_exists: true,
        oracle_exists: true,
        tx_account_matches_owner: false,
    });

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
}

#[test]
fn oracle_delete_loaded_rejects_missing_loaded_oracle() {
    let mut sink = TestLoadedSink::new();
    sink.oracle_exists = false;

    let result = run_oracle_delete_loaded(&mut sink);

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(sink.events, ["oracle_exists"]);
}

#[test]
fn oracle_delete_loaded_maps_dir_remove_failure() {
    let mut sink = TestLoadedSink::new();
    sink.remove_owner_dir = false;

    let result = run_oracle_delete_loaded(&mut sink);

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(result), "tefBAD_LEDGER");
    assert_eq!(sink.events, ["oracle_exists", "remove_owner_dir"]);
}

#[test]
fn oracle_delete_loaded_maps_missing_owner() {
    let mut sink = TestLoadedSink::new();
    sink.owner_exists = false;

    let result = run_oracle_delete_loaded(&mut sink);

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(
        sink.events,
        ["oracle_exists", "remove_owner_dir", "owner_exists"]
    );
}

#[test]
fn oracle_delete_loaded_preserves_owner_count_rule() {
    let mut sink = TestLoadedSink::new();
    sink.price_series_exceeds_five = true;

    let result = run_oracle_delete_loaded(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "oracle_exists",
            "remove_owner_dir",
            "owner_exists",
            "series_gt_five",
            "adjust:-2",
            "erase"
        ]
    );
    assert_eq!(sink.owner_count_delta, vec![-2]);
    assert!(sink.erased);
}

#[test]
fn oracle_delete_do_apply_delegates_when_oracle_exists() {
    let mut sink = TestApplySink::new();

    let result = run_oracle_delete_do_apply(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["peek_oracle", "delete_loaded"]);
}

#[test]
fn oracle_delete_do_apply_maps_missing_oracle_to_internal() {
    let mut sink = TestApplySink::new();
    sink.oracle_exists = false;

    let result = run_oracle_delete_do_apply(&mut sink);

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(sink.events, ["peek_oracle"]);
}
