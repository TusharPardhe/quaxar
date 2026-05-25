//! Integration tests that pin the narrowed Rust `MPTokenIssuanceDestroy.cpp`
//! shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    MPTokenIssuanceDestroyApplySink, MPTokenIssuanceDestroyPreclaimFacts,
    run_mp_token_issuance_destroy_do_apply, run_mp_token_issuance_destroy_preclaim,
    run_mp_token_issuance_destroy_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestSink {
    loaded_issuance_exists: bool,
    account_matches_loaded_issuer: bool,
    dir_remove: bool,
    erased: bool,
    owner_count_deltas: Vec<i32>,
    events: Vec<String>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            loaded_issuance_exists: true,
            account_matches_loaded_issuer: true,
            dir_remove: true,
            erased: false,
            owner_count_deltas: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl MPTokenIssuanceDestroyApplySink for TestSink {
    fn loaded_issuance_exists(&mut self) -> bool {
        self.events.push("loaded_exists".to_string());
        self.loaded_issuance_exists
    }

    fn account_matches_loaded_issuer(&mut self) -> bool {
        self.events.push("issuer_matches".to_string());
        self.account_matches_loaded_issuer
    }

    fn remove_from_owner_dir(&mut self) -> bool {
        self.events.push("dir_remove".to_string());
        self.dir_remove
    }

    fn erase_issuance(&mut self) {
        self.events.push("erase".to_string());
        self.erased = true;
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("owner_count:{delta}"));
        self.owner_count_deltas.push(delta);
    }
}

#[test]
fn mp_token_issuance_destroy_preflight_is_trivial() {
    assert_eq!(run_mp_token_issuance_destroy_preflight(), Ter::TES_SUCCESS);
}

#[test]
fn mp_token_issuance_destroy_preclaim_ordered_guards() {
    let missing = run_mp_token_issuance_destroy_preclaim(MPTokenIssuanceDestroyPreclaimFacts {
        issuance_exists: false,
        issuer_matches: true,
        outstanding_amount_is_zero: true,
        locked_amount_is_zero: true,
    });
    let no_permission =
        run_mp_token_issuance_destroy_preclaim(MPTokenIssuanceDestroyPreclaimFacts {
            issuance_exists: true,
            issuer_matches: false,
            outstanding_amount_is_zero: true,
            locked_amount_is_zero: true,
        });
    let obligations = run_mp_token_issuance_destroy_preclaim(MPTokenIssuanceDestroyPreclaimFacts {
        issuance_exists: true,
        issuer_matches: true,
        outstanding_amount_is_zero: false,
        locked_amount_is_zero: true,
    });
    let locked = run_mp_token_issuance_destroy_preclaim(MPTokenIssuanceDestroyPreclaimFacts {
        issuance_exists: true,
        issuer_matches: true,
        outstanding_amount_is_zero: true,
        locked_amount_is_zero: false,
    });

    assert_eq!(missing, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(no_permission, Ter::TEC_NO_PERMISSION);
    assert_eq!(obligations, Ter::TEC_HAS_OBLIGATIONS);
    assert_eq!(locked, Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn mp_token_issuance_destroy_do_apply_preserves_cpp_delete_order() {
    let mut sink = TestSink::new();

    let result = run_mp_token_issuance_destroy_do_apply(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "loaded_exists",
            "issuer_matches",
            "dir_remove",
            "erase",
            "owner_count:-1"
        ]
    );
    assert!(sink.erased);
    assert_eq!(sink.owner_count_deltas, vec![-1]);
}

#[test]
fn mp_token_issuance_destroy_do_apply_maps_cpp_failures() {
    let mut missing = TestSink::new();
    missing.loaded_issuance_exists = false;
    assert_eq!(
        run_mp_token_issuance_destroy_do_apply(&mut missing),
        Ter::TEC_INTERNAL
    );

    let mut mismatch = TestSink::new();
    mismatch.account_matches_loaded_issuer = false;
    assert_eq!(
        run_mp_token_issuance_destroy_do_apply(&mut mismatch),
        Ter::TEC_INTERNAL
    );

    let mut dir_fail = TestSink::new();
    dir_fail.dir_remove = false;
    let result = run_mp_token_issuance_destroy_do_apply(&mut dir_fail);
    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(result), "tefBAD_LEDGER");
}
