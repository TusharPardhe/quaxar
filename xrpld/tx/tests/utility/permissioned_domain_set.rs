//! Integration tests that pin the narrowed Rust `PermissionedDomainSet.cpp`
//! shell to the current C++ behavior.

use std::cell::Cell;

use protocol::{Ter, trans_token};
use tx::{
    MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE, PermissionedDomainCredential,
    PermissionedDomainSetApplySink, PermissionedDomainSetPreclaimFacts,
    permissioned_domain_set_check_extra_features, run_permissioned_domain_set_do_apply,
    run_permissioned_domain_set_preclaim, run_permissioned_domain_set_preflight,
    sort_permissioned_domain_credentials,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    owner_exists: bool,
    existing_domain_exists: bool,
    has_reserve: bool,
    dir_page: Option<u64>,
    events: Vec<String>,
    replaced_credentials: Vec<Vec<PermissionedDomainCredential<&'static str, &'static str>>>,
    staged_credentials: Vec<Vec<PermissionedDomainCredential<&'static str, &'static str>>>,
    owner_node: Option<u64>,
    owner_count_deltas: Vec<i32>,
    inserted_new_domain: bool,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            owner_exists: true,
            existing_domain_exists: true,
            has_reserve: true,
            dir_page: Some(17),
            events: Vec::new(),
            replaced_credentials: Vec::new(),
            staged_credentials: Vec::new(),
            owner_node: None,
            owner_count_deltas: Vec::new(),
            inserted_new_domain: false,
        }
    }
}

impl PermissionedDomainSetApplySink<PermissionedDomainCredential<&'static str, &'static str>>
    for TestApplySink
{
    type OwnerNode = u64;

    fn owner_exists(&mut self) -> bool {
        self.events.push("owner_exists".to_string());
        self.owner_exists
    }

    fn existing_domain_exists(&mut self) -> bool {
        self.events.push("existing_domain_exists".to_string());
        self.existing_domain_exists
    }

    fn replace_existing_domain_credentials(
        &mut self,
        credentials: Vec<PermissionedDomainCredential<&'static str, &'static str>>,
    ) {
        self.events.push("replace_existing".to_string());
        self.replaced_credentials.push(credentials);
    }

    fn owner_has_reserve_for_new_domain(&mut self) -> bool {
        self.events.push("has_reserve".to_string());
        self.has_reserve
    }

    fn stage_new_domain(
        &mut self,
        credentials: Vec<PermissionedDomainCredential<&'static str, &'static str>>,
    ) {
        self.events.push("stage_new".to_string());
        self.staged_credentials.push(credentials);
    }

    fn dir_insert_new_domain(&mut self) -> Option<Self::OwnerNode> {
        self.events.push("dir_insert".to_string());
        self.dir_page
    }

    fn set_new_domain_owner_node(&mut self, page: Self::OwnerNode) {
        self.events.push(format!("owner_node:{page}"));
        self.owner_node = Some(page);
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn insert_new_domain(&mut self) {
        self.events.push("insert_new".to_string());
        self.inserted_new_domain = true;
    }
}

fn unsorted_credentials() -> Vec<PermissionedDomainCredential<&'static str, &'static str>> {
    vec![
        PermissionedDomainCredential {
            issuer: "bob",
            credential_type: "z",
        },
        PermissionedDomainCredential {
            issuer: "alice",
            credential_type: "b",
        },
        PermissionedDomainCredential {
            issuer: "alice",
            credential_type: "a",
        },
    ]
}

#[test]
fn permissioned_domain_set_check_extra_features_gate() {
    assert!(permissioned_domain_set_check_extra_features(true));
    assert!(!permissioned_domain_set_check_extra_features(false));
}

#[test]
fn permissioned_domain_set_preflight_runs_credentials_check_before_domain_zero_guard() {
    let checked = Cell::new(false);
    let result = run_permissioned_domain_set_preflight(true, true, || {
        checked.set(true);
        Ter::TEM_ARRAY_TOO_LARGE
    });

    assert!(checked.get());
    assert_eq!(result, Ter::TEM_ARRAY_TOO_LARGE);
    assert_eq!(trans_token(result), "temARRAY_TOO_LARGE");
}

#[test]
fn permissioned_domain_set_preflight_rejects_zero_domain_id_after_credentials_pass() {
    let result = run_permissioned_domain_set_preflight(true, true, || Ter::TES_SUCCESS);

    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(result), "temMALFORMED");
}

#[test]
fn permissioned_domain_set_preclaim_preserves() {
    let missing_account = run_permissioned_domain_set_preclaim(
        PermissionedDomainSetPreclaimFacts {
            account_exists: false,
            domain_id_present: true,
            domain_exists: false,
            domain_owned_by_account: false,
        },
        [false],
    );
    let missing_issuer = run_permissioned_domain_set_preclaim(
        PermissionedDomainSetPreclaimFacts {
            account_exists: true,
            domain_id_present: false,
            domain_exists: false,
            domain_owned_by_account: false,
        },
        [true, false],
    );
    let missing_domain = run_permissioned_domain_set_preclaim(
        PermissionedDomainSetPreclaimFacts {
            account_exists: true,
            domain_id_present: true,
            domain_exists: false,
            domain_owned_by_account: false,
        },
        [true],
    );
    let owner_mismatch = run_permissioned_domain_set_preclaim(
        PermissionedDomainSetPreclaimFacts {
            account_exists: true,
            domain_id_present: true,
            domain_exists: true,
            domain_owned_by_account: false,
        },
        [true],
    );

    assert_eq!(missing_account, Ter::TEF_INTERNAL);
    assert_eq!(missing_issuer, Ter::TEC_NO_ISSUER);
    assert_eq!(missing_domain, Ter::TEC_NO_ENTRY);
    assert_eq!(owner_mismatch, Ter::TEC_NO_PERMISSION);
}

#[test]
fn permissioned_domain_set_sort_make_sorted_pair_order() {
    let sorted = sort_permissioned_domain_credentials(unsorted_credentials());

    assert_eq!(
        sorted,
        vec![
            PermissionedDomainCredential {
                issuer: "alice",
                credential_type: "a",
            },
            PermissionedDomainCredential {
                issuer: "alice",
                credential_type: "b",
            },
            PermissionedDomainCredential {
                issuer: "bob",
                credential_type: "z",
            },
        ]
    );
}

#[test]
fn permissioned_domain_set_do_apply_updates_existing_domain_with_sorted_credentials() {
    let mut sink = TestApplySink::new();

    let result = run_permissioned_domain_set_do_apply(unsorted_credentials(), true, &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        ["owner_exists", "existing_domain_exists", "replace_existing"]
    );
    assert_eq!(
        sink.replaced_credentials,
        vec![sort_permissioned_domain_credentials(unsorted_credentials())]
    );
}

#[test]
fn permissioned_domain_set_do_apply_preserves_create_order() {
    let mut sink = TestApplySink::new();

    let result = run_permissioned_domain_set_do_apply(unsorted_credentials(), false, &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "owner_exists",
            "has_reserve",
            "stage_new",
            "dir_insert",
            "owner_node:17",
            "adjust:1",
            "insert_new",
        ]
    );
    assert_eq!(
        sink.staged_credentials,
        vec![sort_permissioned_domain_credentials(unsorted_credentials())]
    );
    assert_eq!(sink.owner_count_deltas, vec![1]);
    assert!(sink.inserted_new_domain);
}

#[test]
fn permissioned_domain_set_do_apply_maps_missing_owner_existing_domain_and_reserve_failures() {
    let mut missing_owner = TestApplySink::new();
    missing_owner.owner_exists = false;
    assert_eq!(
        run_permissioned_domain_set_do_apply(unsorted_credentials(), false, &mut missing_owner),
        Ter::TEF_INTERNAL
    );

    let mut missing_existing = TestApplySink::new();
    missing_existing.existing_domain_exists = false;
    assert_eq!(
        run_permissioned_domain_set_do_apply(unsorted_credentials(), true, &mut missing_existing),
        Ter::TEF_INTERNAL
    );

    let mut no_reserve = TestApplySink::new();
    no_reserve.has_reserve = false;
    assert_eq!(
        run_permissioned_domain_set_do_apply(unsorted_credentials(), false, &mut no_reserve),
        Ter::TEC_INSUFFICIENT_RESERVE
    );
}

#[test]
fn permissioned_domain_set_do_apply_maps_dir_full() {
    let mut sink = TestApplySink::new();
    sink.dir_page = None;

    let result = run_permissioned_domain_set_do_apply(unsorted_credentials(), false, &mut sink);

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(
        sink.events,
        ["owner_exists", "has_reserve", "stage_new", "dir_insert"]
    );
}

#[test]
fn permissioned_domain_set_constant_protocol_limit() {
    assert_eq!(MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE, 10);
}
