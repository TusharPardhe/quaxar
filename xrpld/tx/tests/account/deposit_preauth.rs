//! Integration tests that pin the narrowed Rust
//! `DepositPreauth.cpp` feature-gate, `preflight(...)`, `preclaim(...)`,
//! `doApply()`, and `removeFromLedger(...)` shells
//! to the current C++ behavior.

use std::cell::Cell;

use protocol::{Ter, trans_token};
use tx::{
    DepositPreauthCredentialPreclaimFact, DepositPreauthDoApplyAccountFacts,
    DepositPreauthDoApplyAccountPath, DepositPreauthDoApplyAccountSink,
    DepositPreauthDoApplyCredentialFacts, DepositPreauthDoApplyCredentialSink,
    DepositPreauthPreclaimFacts, DepositPreauthPreflightFacts,
    deposit_preauth_check_extra_features, run_deposit_preauth_do_apply,
    run_deposit_preauth_do_apply_account_paths, run_deposit_preauth_do_apply_credential_paths,
    run_deposit_preauth_preclaim, run_deposit_preauth_preflight,
    run_deposit_preauth_remove_from_ledger,
};

fn empty_preclaim_facts() -> DepositPreauthPreclaimFacts<&'static str, &'static str> {
    DepositPreauthPreclaimFacts {
        authorize: None,
        unauthorize: None,
        authorize_target_exists: false,
        authorize_preauth_exists: false,
        unauthorize_preauth_exists: false,
        authorize_credentials_present: false,
        authorize_credentials: Vec::new(),
        authorize_credentials_preauth_exists: false,
        unauthorize_credentials_present: false,
        unauthorize_credentials_preauth_exists: false,
    }
}

#[derive(Debug, Clone)]
struct TestDoApplySink {
    owner_exists: bool,
    has_reserve: bool,
    dir_page: Option<u64>,
    remove_result: Ter,
    owner_node: Option<u64>,
    adjusted: bool,
    credential_owner_exists: bool,
    credential_has_reserve: bool,
    credential_create_ok: bool,
    credential_dir_page: Option<u64>,
    credential_remove_result: Ter,
    credential_owner_node: Option<u64>,
    credential_adjusted: bool,
    events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
}

impl TestDoApplySink {
    fn new(events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>) -> Self {
        Self {
            owner_exists: true,
            has_reserve: true,
            dir_page: Some(11),
            remove_result: Ter::TES_SUCCESS,
            owner_node: None,
            adjusted: false,
            credential_owner_exists: true,
            credential_has_reserve: true,
            credential_create_ok: true,
            credential_dir_page: Some(17),
            credential_remove_result: Ter::TES_SUCCESS,
            credential_owner_node: None,
            credential_adjusted: false,
            events,
        }
    }
}

impl DepositPreauthDoApplyAccountSink for TestDoApplySink {
    type OwnerNode = u64;

    fn authorize_owner_exists(&mut self) -> bool {
        self.events.borrow_mut().push("owner");
        self.owner_exists
    }

    fn authorize_has_reserve(&mut self) -> bool {
        self.events.borrow_mut().push("reserve");
        self.has_reserve
    }

    fn create_authorize_preauth(&mut self) {
        self.events.borrow_mut().push("create");
    }

    fn dir_insert_authorize_preauth(&mut self) -> Option<Self::OwnerNode> {
        self.events.borrow_mut().push("dir");
        self.dir_page
    }

    fn set_authorize_owner_node(&mut self, page: Self::OwnerNode) {
        self.events.borrow_mut().push("owner_node");
        self.owner_node = Some(page);
    }

    fn adjust_authorize_owner_count(&mut self) {
        self.events.borrow_mut().push("adjust");
        self.adjusted = true;
    }

    fn remove_unauthorize_preauth(&mut self) -> Ter {
        self.events.borrow_mut().push("remove");
        self.remove_result
    }
}

impl DepositPreauthDoApplyCredentialSink for TestDoApplySink {
    type OwnerNode = u64;

    fn authorize_credentials_owner_exists(&mut self) -> bool {
        self.events.borrow_mut().push("cred_owner");
        self.credential_owner_exists
    }

    fn authorize_credentials_has_reserve(&mut self) -> bool {
        self.events.borrow_mut().push("cred_reserve");
        self.credential_has_reserve
    }

    fn sort_authorize_credentials(&mut self) {
        self.events.borrow_mut().push("cred_sort");
    }

    fn create_authorize_credentials_preauth(&mut self) -> bool {
        self.events.borrow_mut().push("cred_create");
        self.credential_create_ok
    }

    fn dir_insert_authorize_credentials_preauth(&mut self) -> Option<Self::OwnerNode> {
        self.events.borrow_mut().push("cred_dir");
        self.credential_dir_page
    }

    fn set_authorize_credentials_owner_node(&mut self, page: Self::OwnerNode) {
        self.events.borrow_mut().push("cred_owner_node");
        self.credential_owner_node = Some(page);
    }

    fn adjust_authorize_credentials_owner_count(&mut self) {
        self.events.borrow_mut().push("cred_adjust");
        self.credential_adjusted = true;
    }

    fn remove_unauthorize_credentials_preauth(&mut self) -> Ter {
        self.events.borrow_mut().push("cred_remove");
        self.credential_remove_result
    }
}

#[test]
fn deposit_preauth_check_extra_features_gate() {
    assert!(deposit_preauth_check_extra_features(false, false, false));
    assert!(deposit_preauth_check_extra_features(true, false, true));
    assert!(deposit_preauth_check_extra_features(false, true, true));
    assert!(!deposit_preauth_check_extra_features(true, false, false));
    assert!(!deposit_preauth_check_extra_features(false, true, false));
}

#[test]
fn deposit_preauth_preflight_rejects_invalid_field_combinations() {
    let none = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: None,
            unauthorize: None,
            authorize_is_zero: false,
            unauthorize_is_zero: false,
            authorize_credentials_present: false,
            unauthorize_credentials_present: false,
        },
        || Ter::TES_SUCCESS,
    );
    let both_accounts = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: Some("becky"),
            unauthorize: Some("carol"),
            authorize_is_zero: false,
            unauthorize_is_zero: false,
            authorize_credentials_present: false,
            unauthorize_credentials_present: false,
        },
        || Ter::TES_SUCCESS,
    );
    let account_and_credentials = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: Some("becky"),
            unauthorize: None,
            authorize_is_zero: false,
            unauthorize_is_zero: false,
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(none, Ter::TEM_MALFORMED);
    assert_eq!(both_accounts, Ter::TEM_MALFORMED);
    assert_eq!(account_and_credentials, Ter::TEM_MALFORMED);
}

#[test]
fn deposit_preauth_preflight_rejects_zero_authorize_target() {
    let result = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: Some("zero"),
            unauthorize: None,
            authorize_is_zero: true,
            unauthorize_is_zero: false,
            authorize_credentials_present: false,
            unauthorize_credentials_present: false,
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_INVALID_ACCOUNT_ID);
    assert_eq!(trans_token(result), "temINVALID_ACCOUNT_ID");
}

#[test]
fn deposit_preauth_preflight_rejects_zero_unauthorize_target() {
    let result = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: None,
            unauthorize: Some("zero"),
            authorize_is_zero: false,
            unauthorize_is_zero: true,
            authorize_credentials_present: false,
            unauthorize_credentials_present: false,
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_INVALID_ACCOUNT_ID);
}

#[test]
fn deposit_preauth_preflight_rejects_self_authorize() {
    let result = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: Some("alice"),
            unauthorize: None,
            authorize_is_zero: false,
            unauthorize_is_zero: false,
            authorize_credentials_present: false,
            unauthorize_credentials_present: false,
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_CANNOT_PREAUTH_SELF);
    assert_eq!(trans_token(result), "temCANNOT_PREAUTH_SELF");
}

#[test]
fn deposit_preauth_preflight_allows_unauthorize_self() {
    let result = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: None,
            unauthorize: Some("alice"),
            authorize_is_zero: false,
            unauthorize_is_zero: false,
            authorize_credentials_present: false,
            unauthorize_credentials_present: false,
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn deposit_preauth_preflight_runs_credential_array_check_only_for_array_paths() {
    let called = Cell::new(false);

    let result = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: None,
            unauthorize: None,
            authorize_is_zero: false,
            unauthorize_is_zero: false,
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        || {
            called.set(true);
            Ter::TEM_ARRAY_TOO_LARGE
        },
    );

    assert!(called.get());
    assert_eq!(result, Ter::TEM_ARRAY_TOO_LARGE);
}

#[test]
fn deposit_preauth_preflight_skips_credential_array_check_for_account_paths() {
    let called = Cell::new(false);

    let result = run_deposit_preauth_preflight(
        DepositPreauthPreflightFacts {
            account: "alice",
            authorize: Some("becky"),
            unauthorize: None,
            authorize_is_zero: false,
            unauthorize_is_zero: false,
            authorize_credentials_present: false,
            unauthorize_credentials_present: false,
        },
        || {
            called.set(true);
            Ter::TEM_ARRAY_EMPTY
        },
    );

    assert!(!called.get());
    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn deposit_preauth_preclaim_rejects_missing_authorize_target() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        authorize: Some("carol"),
        authorize_target_exists: false,
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_TARGET);
    assert_eq!(trans_token(result), "tecNO_TARGET");
}

#[test]
fn deposit_preauth_preclaim_rejects_duplicate_authorize_entry() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        authorize: Some("becky"),
        authorize_target_exists: true,
        authorize_preauth_exists: true,
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_DUPLICATE);
    assert_eq!(trans_token(result), "tecDUPLICATE");
}

#[test]
fn deposit_preauth_preclaim_rejects_missing_unauthorize_entry() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        unauthorize: Some("becky"),
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn deposit_preauth_preclaim_rejects_missing_credential_issuer() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        authorize_credentials_present: true,
        authorize_credentials: vec![DepositPreauthCredentialPreclaimFact {
            issuer: "rick",
            credential_type: "cred-a",
            issuer_exists: false,
        }],
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_ISSUER);
    assert_eq!(trans_token(result), "tecNO_ISSUER");
}

#[test]
fn deposit_preauth_preclaim_rejects_duplicate_credential_pair() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        authorize_credentials_present: true,
        authorize_credentials: vec![
            DepositPreauthCredentialPreclaimFact {
                issuer: "issuer",
                credential_type: "cred-a",
                issuer_exists: true,
            },
            DepositPreauthCredentialPreclaimFact {
                issuer: "issuer",
                credential_type: "cred-a",
                issuer_exists: true,
            },
        ],
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
}

#[test]
fn deposit_preauth_preclaim_rejects_duplicate_credential_preauth() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        authorize_credentials_present: true,
        authorize_credentials: vec![DepositPreauthCredentialPreclaimFact {
            issuer: "issuer",
            credential_type: "cred-a",
            issuer_exists: true,
        }],
        authorize_credentials_preauth_exists: true,
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_DUPLICATE);
}

#[test]
fn deposit_preauth_preclaim_rejects_missing_credential_unauthorize_entry() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        unauthorize_credentials_present: true,
        unauthorize_credentials_preauth_exists: false,
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

#[test]
fn deposit_preauth_preclaim_uses_current_cpp_branch_priority() {
    let result = run_deposit_preauth_preclaim(DepositPreauthPreclaimFacts {
        authorize: Some("becky"),
        authorize_target_exists: false,
        authorize_credentials_present: true,
        authorize_credentials: vec![DepositPreauthCredentialPreclaimFact {
            issuer: "issuer",
            credential_type: "cred-a",
            issuer_exists: false,
        }],
        authorize_credentials_preauth_exists: true,
        ..empty_preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_TARGET);
}

#[test]
fn deposit_preauth_do_apply_authorize_returns_tefinternal_for_missing_owner() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.owner_exists = false;

    let result = run_deposit_preauth_do_apply_account_paths(
        DepositPreauthDoApplyAccountFacts {
            authorize_present: true,
            unauthorize_present: false,
        },
        &mut sink,
    );

    assert_eq!(
        result,
        DepositPreauthDoApplyAccountPath::Return(Ter::TEF_INTERNAL)
    );
    assert_eq!(trans_token(Ter::TEF_INTERNAL), "tefINTERNAL");
    assert_eq!(events.borrow().as_slice(), ["owner"]);
}

#[test]
fn deposit_preauth_do_apply_authorize_checks_reserve_before_create() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.has_reserve = false;

    let result = run_deposit_preauth_do_apply_account_paths(
        DepositPreauthDoApplyAccountFacts {
            authorize_present: true,
            unauthorize_present: false,
        },
        &mut sink,
    );

    assert_eq!(
        result,
        DepositPreauthDoApplyAccountPath::Return(Ter::TEC_INSUFFICIENT_RESERVE)
    );
    assert_eq!(events.borrow().as_slice(), ["owner", "reserve"]);
}

#[test]
fn deposit_preauth_do_apply_authorize_maps_missing_dir_page_to_tecdir_full() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.dir_page = None;

    let result = run_deposit_preauth_do_apply_account_paths(
        DepositPreauthDoApplyAccountFacts {
            authorize_present: true,
            unauthorize_present: false,
        },
        &mut sink,
    );

    assert_eq!(
        result,
        DepositPreauthDoApplyAccountPath::Return(Ter::TEC_DIR_FULL)
    );
    assert_eq!(trans_token(Ter::TEC_DIR_FULL), "tecDIR_FULL");
    assert_eq!(
        events.borrow().as_slice(),
        ["owner", "reserve", "create", "dir"]
    );
}

#[test]
fn deposit_preauth_do_apply_authorize_preserves_current_on_success() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));

    let result = run_deposit_preauth_do_apply_account_paths(
        DepositPreauthDoApplyAccountFacts {
            authorize_present: true,
            unauthorize_present: false,
        },
        &mut sink,
    );

    assert_eq!(
        result,
        DepositPreauthDoApplyAccountPath::Return(Ter::TES_SUCCESS)
    );
    assert_eq!(
        events.borrow().as_slice(),
        ["owner", "reserve", "create", "dir", "owner_node", "adjust"]
    );
    assert_eq!(sink.owner_node, Some(11));
    assert!(sink.adjusted);
}

#[test]
fn deposit_preauth_do_apply_unauthorize_returns_remove_result_unchanged() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.remove_result = Ter::TEC_NO_ENTRY;

    let result = run_deposit_preauth_do_apply_account_paths(
        DepositPreauthDoApplyAccountFacts {
            authorize_present: false,
            unauthorize_present: true,
        },
        &mut sink,
    );

    assert_eq!(
        result,
        DepositPreauthDoApplyAccountPath::Return(Ter::TEC_NO_ENTRY)
    );
    assert_eq!(trans_token(Ter::TEC_NO_ENTRY), "tecNO_ENTRY");
    assert_eq!(events.borrow().as_slice(), ["remove"]);
}

#[test]
fn deposit_preauth_do_apply_account_paths_continue_to_credentials_when_no_account_path_exists() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));

    let result = run_deposit_preauth_do_apply_account_paths(
        DepositPreauthDoApplyAccountFacts::default(),
        &mut sink,
    );

    assert_eq!(
        result,
        DepositPreauthDoApplyAccountPath::ContinueToCredentialPaths
    );
    assert!(events.borrow().is_empty());
}

#[test]
fn deposit_preauth_do_apply_account_paths_use_authorize_branch_first() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.owner_exists = false;
    sink.remove_result = Ter::TEC_NO_ENTRY;

    let result = run_deposit_preauth_do_apply_account_paths(
        DepositPreauthDoApplyAccountFacts {
            authorize_present: true,
            unauthorize_present: true,
        },
        &mut sink,
    );

    assert_eq!(
        result,
        DepositPreauthDoApplyAccountPath::Return(Ter::TEF_INTERNAL)
    );
    assert_eq!(events.borrow().as_slice(), ["owner"]);
}

#[test]
fn deposit_preauth_do_apply_credentials_return_tefinternal_for_missing_owner() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.credential_owner_exists = false;

    let result = run_deposit_preauth_do_apply_credential_paths(
        DepositPreauthDoApplyCredentialFacts {
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(events.borrow().as_slice(), ["cred_owner"]);
}

#[test]
fn deposit_preauth_do_apply_credentials_check_reserve_before_sort() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.credential_has_reserve = false;

    let result = run_deposit_preauth_do_apply_credential_paths(
        DepositPreauthDoApplyCredentialFacts {
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(events.borrow().as_slice(), ["cred_owner", "cred_reserve"]);
}

#[test]
fn deposit_preauth_do_apply_credentials_map_create_failure_to_tefinternal() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.credential_create_ok = false;

    let result = run_deposit_preauth_do_apply_credential_paths(
        DepositPreauthDoApplyCredentialFacts {
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(
        events.borrow().as_slice(),
        ["cred_owner", "cred_reserve", "cred_sort", "cred_create"]
    );
}

#[test]
fn deposit_preauth_do_apply_credentials_map_missing_dir_page_to_tecdir_full() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.credential_dir_page = None;

    let result = run_deposit_preauth_do_apply_credential_paths(
        DepositPreauthDoApplyCredentialFacts {
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(Ter::TEC_DIR_FULL), "tecDIR_FULL");
    assert_eq!(
        events.borrow().as_slice(),
        [
            "cred_owner",
            "cred_reserve",
            "cred_sort",
            "cred_create",
            "cred_dir"
        ]
    );
}

#[test]
fn deposit_preauth_do_apply_credentials_preserve_current_on_success() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));

    let result = run_deposit_preauth_do_apply_credential_paths(
        DepositPreauthDoApplyCredentialFacts {
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        events.borrow().as_slice(),
        [
            "cred_owner",
            "cred_reserve",
            "cred_sort",
            "cred_create",
            "cred_dir",
            "cred_owner_node",
            "cred_adjust",
        ]
    );
    assert_eq!(sink.credential_owner_node, Some(17));
    assert!(sink.credential_adjusted);
}

#[test]
fn deposit_preauth_do_apply_unauthorize_credentials_return_remove_result_unchanged() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));
    sink.credential_remove_result = Ter::TEC_NO_ENTRY;

    let result = run_deposit_preauth_do_apply_credential_paths(
        DepositPreauthDoApplyCredentialFacts {
            authorize_credentials_present: false,
            unauthorize_credentials_present: true,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(events.borrow().as_slice(), ["cred_remove"]);
}

#[test]
fn deposit_preauth_remove_from_ledger_returns_tecno_entry_for_missing_preauth() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let result = run_deposit_preauth_remove_from_ledger::<u64>(
        None,
        |_| {
            events.borrow_mut().push("dir_remove");
            true
        },
        |_| {
            events.borrow_mut().push("owner");
            true
        },
        || events.borrow_mut().push("adjust"),
        |_| events.borrow_mut().push("erase"),
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert!(events.borrow().is_empty());
}

#[test]
fn deposit_preauth_remove_from_ledger_maps_dir_remove_failure() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let result = run_deposit_preauth_remove_from_ledger(
        Some("preauth"),
        |_| {
            events.borrow_mut().push("dir_remove");
            false
        },
        |_| {
            events.borrow_mut().push("owner");
            true
        },
        || events.borrow_mut().push("adjust"),
        |_| events.borrow_mut().push("erase"),
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(Ter::TEF_BAD_LEDGER), "tefBAD_LEDGER");
    assert_eq!(events.borrow().as_slice(), ["dir_remove"]);
}

#[test]
fn deposit_preauth_remove_from_ledger_returns_tefinternal_for_missing_owner() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let result = run_deposit_preauth_remove_from_ledger(
        Some("preauth"),
        |_| {
            events.borrow_mut().push("dir_remove");
            true
        },
        |_| {
            events.borrow_mut().push("owner");
            false
        },
        || events.borrow_mut().push("adjust"),
        |_| events.borrow_mut().push("erase"),
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(events.borrow().as_slice(), ["dir_remove", "owner"]);
}

#[test]
fn deposit_preauth_remove_from_ledger_preserves_current_on_success() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let erased = Cell::new(None);
    let result = run_deposit_preauth_remove_from_ledger(
        Some("preauth"),
        |_| {
            events.borrow_mut().push("dir_remove");
            true
        },
        |_| {
            events.borrow_mut().push("owner");
            true
        },
        || events.borrow_mut().push("adjust"),
        |preauth| {
            events.borrow_mut().push("erase");
            erased.set(Some(preauth));
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        events.borrow().as_slice(),
        ["dir_remove", "owner", "adjust", "erase"]
    );
    assert_eq!(erased.get(), Some("preauth"));
}

#[test]
fn deposit_preauth_do_apply_runs_credential_paths_after_account_shell() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));

    let result = run_deposit_preauth_do_apply(
        DepositPreauthDoApplyAccountFacts::default(),
        DepositPreauthDoApplyCredentialFacts {
            authorize_credentials_present: true,
            unauthorize_credentials_present: false,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        events.borrow().as_slice(),
        [
            "cred_owner",
            "cred_reserve",
            "cred_sort",
            "cred_create",
            "cred_dir",
            "cred_owner_node",
            "cred_adjust",
        ]
    );
}

#[test]
fn deposit_preauth_do_apply_defaults_to_tessuccess_when_no_path_exists() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut sink = TestDoApplySink::new(std::rc::Rc::clone(&events));

    let result = run_deposit_preauth_do_apply(
        DepositPreauthDoApplyAccountFacts::default(),
        DepositPreauthDoApplyCredentialFacts::default(),
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(events.borrow().is_empty());
}
