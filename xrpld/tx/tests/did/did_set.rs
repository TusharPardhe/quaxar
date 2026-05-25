//! Integration tests that pin the narrowed Rust `DIDSet.cpp` shell to the
//! current C++ behavior.

use protocol::{AccountID, Ter, trans_token};
use tx::{
    DidSetApplyFacts, DidSetApplySink, DidSetCreateMutation, DidSetFieldUpdate, DidSetLoadedEntry,
    DidSetPreflightFacts, DidSetUpdateMutation, MAX_DID_DATA_LENGTH, MAX_DID_DOCUMENT_LENGTH,
    MAX_DID_URI_LENGTH, run_did_set_do_apply, run_did_set_preflight,
};

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestSink {
    existing: Option<DidSetLoadedEntry>,
    owner_account_exists: bool,
    reserve_sufficient: bool,
    owner_dir_page: Option<u64>,
    updated: Option<DidSetUpdateMutation>,
    created: Option<DidSetCreateMutation>,
    owner_count_deltas: Vec<i32>,
    events: Vec<String>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            existing: None,
            owner_account_exists: true,
            reserve_sufficient: true,
            owner_dir_page: Some(44),
            updated: None,
            created: None,
            owner_count_deltas: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl DidSetApplySink for TestSink {
    fn existing_did(&mut self) -> Option<DidSetLoadedEntry> {
        self.events.push("existing".to_string());
        self.existing.clone()
    }

    fn owner_account_exists(&mut self) -> bool {
        self.events.push("owner_exists".to_string());
        self.owner_account_exists
    }

    fn reserve_sufficient(&mut self) -> bool {
        self.events.push("reserve".to_string());
        self.reserve_sufficient
    }

    fn insert_owner_dir(&mut self) -> Option<u64> {
        self.events.push("owner_dir".to_string());
        self.owner_dir_page
    }

    fn update_did(&mut self, mutation: DidSetUpdateMutation) {
        self.events.push("update".to_string());
        self.updated = Some(mutation);
    }

    fn create_did(&mut self, mutation: DidSetCreateMutation) {
        self.events.push("create".to_string());
        self.created = Some(mutation);
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("owner_count:{delta}"));
        self.owner_count_deltas.push(delta);
    }
}

#[test]
fn did_set_preflight_rejects_missing_all_fields() {
    let result = run_did_set_preflight(DidSetPreflightFacts::default());

    assert_eq!(result, Ter::TEM_EMPTY_DID);
    assert_eq!(trans_token(result), "temEMPTY_DID");
}

#[test]
fn did_set_preflight_rejects_all_present_empty() {
    let result = run_did_set_preflight(DidSetPreflightFacts {
        uri_len: Some(0),
        did_document_len: Some(0),
        data_len: Some(0),
    });

    assert_eq!(result, Ter::TEM_EMPTY_DID);
}

#[test]
fn did_set_preflight_allows_single_empty_present_field() {
    let result = run_did_set_preflight(DidSetPreflightFacts {
        uri_len: Some(0),
        did_document_len: None,
        data_len: None,
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn did_set_preflight_rejects_oversized_fields() {
    let uri = run_did_set_preflight(DidSetPreflightFacts {
        uri_len: Some(MAX_DID_URI_LENGTH + 1),
        did_document_len: None,
        data_len: None,
    });
    let document = run_did_set_preflight(DidSetPreflightFacts {
        uri_len: None,
        did_document_len: Some(MAX_DID_DOCUMENT_LENGTH + 1),
        data_len: None,
    });
    let data = run_did_set_preflight(DidSetPreflightFacts {
        uri_len: None,
        did_document_len: None,
        data_len: Some(MAX_DID_DATA_LENGTH + 1),
    });

    assert_eq!(uri, Ter::TEM_MALFORMED);
    assert_eq!(document, Ter::TEM_MALFORMED);
    assert_eq!(data, Ter::TEM_MALFORMED);
}

#[test]
fn did_set_do_apply_updates_existing_entry_and_preserves_empty_guard() {
    let mut sink = TestSink::new();
    sink.existing = Some(DidSetLoadedEntry {
        uri: Some(vec![1, 2]),
        did_document: Some(vec![3, 4]),
        data: None,
    });

    let result = run_did_set_do_apply(
        DidSetApplyFacts {
            account: account("1111111111111111111111111111111111111111"),
            uri: Some(Vec::new()),
            did_document: Some(vec![9, 9]),
            data: None,
            fix_empty_did_enabled: true,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["existing", "update"]);
    assert_eq!(
        sink.updated,
        Some(DidSetUpdateMutation {
            uri: DidSetFieldUpdate::Remove,
            did_document: DidSetFieldUpdate::Set(vec![9, 9]),
            data: DidSetFieldUpdate::NoChange,
        })
    );
}

#[test]
fn did_set_do_apply_rejects_existing_entry_becoming_empty() {
    let mut sink = TestSink::new();
    sink.existing = Some(DidSetLoadedEntry {
        uri: Some(vec![1]),
        did_document: None,
        data: None,
    });

    let result = run_did_set_do_apply(
        DidSetApplyFacts {
            account: account("1111111111111111111111111111111111111111"),
            uri: Some(Vec::new()),
            did_document: None,
            data: None,
            fix_empty_did_enabled: true,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_EMPTY_DID);
    assert_eq!(trans_token(result), "tecEMPTY_DID");
    assert_eq!(sink.events, ["existing"]);
    assert!(sink.updated.is_none());
}

#[test]
fn did_set_do_apply_create_path_maps_missing_account_reserve_and_dir_failures() {
    let base_facts = DidSetApplyFacts {
        account: account("1111111111111111111111111111111111111111"),
        uri: Some(vec![7]),
        did_document: None,
        data: None,
        fix_empty_did_enabled: true,
    };

    let mut missing_owner = TestSink::new();
    missing_owner.owner_account_exists = false;
    assert_eq!(
        run_did_set_do_apply(base_facts.clone(), &mut missing_owner),
        Ter::TEF_INTERNAL
    );
    assert_eq!(missing_owner.events, ["existing", "owner_exists"]);

    let mut no_reserve = TestSink::new();
    no_reserve.reserve_sufficient = false;
    assert_eq!(
        run_did_set_do_apply(base_facts.clone(), &mut no_reserve),
        Ter::TEC_INSUFFICIENT_RESERVE
    );
    assert_eq!(no_reserve.events, ["existing", "owner_exists", "reserve"]);

    let mut dir_full = TestSink::new();
    dir_full.owner_dir_page = None;
    assert_eq!(
        run_did_set_do_apply(base_facts, &mut dir_full),
        Ter::TEC_DIR_FULL
    );
    assert_eq!(
        dir_full.events,
        ["existing", "owner_exists", "reserve", "owner_dir"]
    );
}

#[test]
fn did_set_do_apply_create_path_honors_fix_empty_did_and_creates_nonempty_fields() {
    let owner = account("1111111111111111111111111111111111111111");

    let mut gated = TestSink::new();
    let gated_result = run_did_set_do_apply(
        DidSetApplyFacts {
            account: owner,
            uri: Some(Vec::new()),
            did_document: None,
            data: Some(Vec::new()),
            fix_empty_did_enabled: true,
        },
        &mut gated,
    );
    assert_eq!(gated_result, Ter::TEC_EMPTY_DID);
    assert_eq!(gated.events, ["existing", "owner_exists", "reserve"]);

    let mut sink = TestSink::new();
    let result = run_did_set_do_apply(
        DidSetApplyFacts {
            account: owner,
            uri: Some(vec![1, 2, 3]),
            did_document: Some(Vec::new()),
            data: Some(vec![4, 5]),
            fix_empty_did_enabled: true,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "existing",
            "owner_exists",
            "reserve",
            "owner_dir",
            "create",
            "owner_count:1"
        ]
    );
    assert_eq!(
        sink.created,
        Some(DidSetCreateMutation {
            account: owner,
            uri: Some(vec![1, 2, 3]),
            did_document: None,
            data: Some(vec![4, 5]),
            owner_node: 44,
        })
    );
    assert_eq!(sink.owner_count_deltas, vec![1]);
}
