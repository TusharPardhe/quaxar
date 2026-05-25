//! Integration tests that pin the narrowed Rust `PermissionedDomainDelete.cpp`
//! shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    PermissionedDomainDeleteApplySink, PermissionedDomainDeleteLoadedSink,
    PermissionedDomainDeletePreclaimFacts, run_permissioned_domain_delete_do_apply,
    run_permissioned_domain_delete_loaded, run_permissioned_domain_delete_preclaim,
    run_permissioned_domain_delete_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestLoadedSink {
    dir_remove: bool,
    owner_and_count_valid: bool,
    events: Vec<String>,
    owner_count_deltas: Vec<i32>,
    erased: bool,
}

impl TestLoadedSink {
    fn new() -> Self {
        Self {
            dir_remove: true,
            owner_and_count_valid: true,
            events: Vec::new(),
            owner_count_deltas: Vec::new(),
            erased: false,
        }
    }
}

impl PermissionedDomainDeleteLoadedSink for TestLoadedSink {
    fn dir_remove(&mut self) -> bool {
        self.events.push("dir_remove".to_string());
        self.dir_remove
    }

    fn owner_exists_with_nonzero_count(&mut self) -> bool {
        self.events.push("owner_and_count".to_string());
        self.owner_and_count_valid
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn erase_domain(&mut self) {
        self.events.push("erase".to_string());
        self.erased = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    loaded_exists: bool,
    delete_result: Ter,
    events: Vec<String>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            loaded_exists: true,
            delete_result: Ter::TES_SUCCESS,
            events: Vec::new(),
        }
    }
}

impl PermissionedDomainDeleteApplySink for TestApplySink {
    fn loaded_domain_exists(&mut self) -> bool {
        self.events.push("peek_loaded".to_string());
        self.loaded_exists
    }

    fn delete_loaded_domain(&mut self) -> Ter {
        self.events.push("delete_loaded".to_string());
        self.delete_result
    }
}

#[test]
fn permissioned_domain_delete_preflight_rejects_zero_domain() {
    let result = run_permissioned_domain_delete_preflight(true);

    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(result), "temMALFORMED");
}

#[test]
fn permissioned_domain_delete_preclaim_existence_and_owner_checks() {
    let missing = run_permissioned_domain_delete_preclaim(PermissionedDomainDeletePreclaimFacts {
        domain_exists: false,
        tx_account_matches_owner: false,
    });
    let mismatch = run_permissioned_domain_delete_preclaim(PermissionedDomainDeletePreclaimFacts {
        domain_exists: true,
        tx_account_matches_owner: false,
    });

    assert_eq!(missing, Ter::TEC_NO_ENTRY);
    assert_eq!(mismatch, Ter::TEC_NO_PERMISSION);
}

#[test]
fn permissioned_domain_delete_loaded_preserves_delete_order() {
    let mut sink = TestLoadedSink::new();

    let result = run_permissioned_domain_delete_loaded(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        ["dir_remove", "owner_and_count", "adjust:-1", "erase"]
    );
    assert_eq!(sink.owner_count_deltas, vec![-1]);
    assert!(sink.erased);
}

#[test]
fn permissioned_domain_delete_loaded_maps_dir_remove_failure() {
    let mut sink = TestLoadedSink::new();
    sink.dir_remove = false;

    let result = run_permissioned_domain_delete_loaded(&mut sink);

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(result), "tefBAD_LEDGER");
    assert_eq!(sink.events, ["dir_remove"]);
}

#[test]
fn permissioned_domain_delete_do_apply_requires_loaded_domain_then_delegates() {
    let mut missing = TestApplySink::new();
    missing.loaded_exists = false;
    assert_eq!(
        run_permissioned_domain_delete_do_apply(&mut missing),
        Ter::TEF_INTERNAL
    );
    assert_eq!(missing.events, ["peek_loaded"]);

    let mut present = TestApplySink::new();
    assert_eq!(
        run_permissioned_domain_delete_do_apply(&mut present),
        Ter::TES_SUCCESS
    );
    assert_eq!(present.events, ["peek_loaded", "delete_loaded"]);
}

#[test]
#[should_panic(
    expected = "PermissionedDomainDelete::doApply expects owner and nonzero owner count"
)]
fn permissioned_domain_delete_loaded_keeps_cpp_owner_assert_boundary() {
    let mut sink = TestLoadedSink::new();
    sink.owner_and_count_valid = false;

    let _ = run_permissioned_domain_delete_loaded(&mut sink);
}
