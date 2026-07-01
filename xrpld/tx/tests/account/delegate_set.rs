use protocol::{Ter, trans_token};
use tx::{
    DelegateSetApplySink, DelegateSetDeleteSink, DelegateSetPreclaimFacts, PERMISSION_MAX_SIZE,
    run_delegate_set_delete_delegate, run_delegate_set_do_apply, run_delegate_set_preclaim,
    run_delegate_set_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestDeleteSink {
    delegate_exists: bool,
    dir_remove_owner_ok: bool,
    dir_remove_destination_result: Option<bool>,
    owner_exists: bool,
    events: Vec<String>,
    owner_count_deltas: Vec<i32>,
    erased: bool,
}

impl TestDeleteSink {
    fn new() -> Self {
        Self {
            delegate_exists: true,
            dir_remove_owner_ok: true,
            dir_remove_destination_result: Some(true),
            owner_exists: true,
            events: Vec::new(),
            owner_count_deltas: Vec::new(),
            erased: false,
        }
    }
}

impl DelegateSetDeleteSink for TestDeleteSink {
    fn delegate_exists_for_delete(&mut self) -> bool {
        self.events.push("delegate_exists".to_string());
        self.delegate_exists
    }

    fn dir_remove_owner(&mut self) -> bool {
        self.events.push("dir_remove_owner".to_string());
        self.dir_remove_owner_ok
    }

    fn dir_remove_destination(&mut self) -> Option<bool> {
        self.events.push("dir_remove_destination".to_string());
        self.dir_remove_destination_result
    }

    fn owner_exists(&mut self) -> bool {
        self.events.push("owner_exists".to_string());
        self.owner_exists
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn erase_delegate(&mut self) {
        self.events.push("erase".to_string());
        self.erased = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    owner_exists: bool,
    delegate_exists: bool,
    has_reserve: bool,
    dir_owner_page: Option<u64>,
    dir_dest_page: Option<u64>,
    events: Vec<String>,
    updated_permissions: Vec<Vec<u32>>,
    staged_permissions: Vec<Vec<u32>>,
    owner_node: Option<u64>,
    destination_node: Option<u64>,
    inserted: bool,
    owner_count_deltas: Vec<i32>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            owner_exists: true,
            delegate_exists: true,
            has_reserve: true,
            dir_owner_page: Some(9),
            dir_dest_page: Some(11),
            events: Vec::new(),
            updated_permissions: Vec::new(),
            staged_permissions: Vec::new(),
            owner_node: None,
            destination_node: None,
            inserted: false,
            owner_count_deltas: Vec::new(),
        }
    }
}

impl DelegateSetDeleteSink for TestApplySink {
    fn delegate_exists_for_delete(&mut self) -> bool {
        self.events.push("delegate_exists_delete".to_string());
        self.delegate_exists
    }

    fn dir_remove_owner(&mut self) -> bool {
        self.events.push("dir_remove_owner".to_string());
        self.dir_owner_page.is_some()
    }

    fn dir_remove_destination(&mut self) -> Option<bool> {
        self.events.push("dir_remove_destination".to_string());
        self.dir_dest_page.map(|_| true)
    }

    fn owner_exists(&mut self) -> bool {
        self.events.push("owner_exists_delete".to_string());
        self.owner_exists
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn erase_delegate(&mut self) {
        self.events.push("erase".to_string());
    }
}

impl DelegateSetApplySink<u32> for TestApplySink {
    type OwnerNode = u64;

    fn owner_exists_for_apply(&mut self) -> bool {
        self.events.push("owner_exists_apply".to_string());
        self.owner_exists
    }

    fn delegate_exists_for_apply(&mut self) -> bool {
        self.events.push("delegate_exists_apply".to_string());
        self.delegate_exists
    }

    fn update_existing_permissions(&mut self, permissions: Vec<u32>) {
        self.events.push("update_existing".to_string());
        self.updated_permissions.push(permissions);
    }

    fn owner_has_reserve_for_create(&mut self) -> bool {
        self.events.push("has_reserve".to_string());
        self.has_reserve
    }

    fn stage_new_delegate(&mut self, permissions: Vec<u32>) {
        self.events.push("stage_new".to_string());
        self.staged_permissions.push(permissions);
    }

    fn dir_insert_owner(&mut self) -> Option<Self::OwnerNode> {
        self.events.push("dir_insert_owner".to_string());
        self.dir_owner_page
    }

    fn set_owner_node(&mut self, page: Self::OwnerNode) {
        self.events.push(format!("owner_node:{page}"));
        self.owner_node = Some(page);
    }

    fn dir_insert_destination(&mut self) -> Option<Self::OwnerNode> {
        self.events.push("dir_insert_destination".to_string());
        self.dir_dest_page
    }

    fn set_destination_node(&mut self, page: Self::OwnerNode) {
        self.events.push(format!("destination_node:{page}"));
        self.destination_node = Some(page);
    }

    fn insert_new_delegate(&mut self) {
        self.events.push("insert".to_string());
        self.inserted = true;
    }
}

#[test]
fn delegate_set_preflight_rejects_large_self_duplicate_and_nondelegable_permissions() {
    let oversized = run_delegate_set_preflight(
        &"alice",
        &"bob",
        &(0..=PERMISSION_MAX_SIZE as u32).collect::<Vec<_>>(),
        |_| true,
    );
    let self_auth = run_delegate_set_preflight(&"alice", &"alice", &[1_u32], |_| true);
    let duplicate = run_delegate_set_preflight(&"alice", &"bob", &[7_u32, 7_u32], |_| true);
    let undelegable = run_delegate_set_preflight(&"alice", &"bob", &[1_u32, 2_u32], |permission| {
        *permission != 2
    });

    assert_eq!(oversized, Ter::TEM_ARRAY_TOO_LARGE);
    assert_eq!(self_auth, Ter::TEM_MALFORMED);
    assert_eq!(duplicate, Ter::TEM_MALFORMED);
    assert_eq!(undelegable, Ter::TEM_MALFORMED);
}

#[test]
fn delegate_set_preclaim_account_target_and_delete_checks() {
    let missing_account = run_delegate_set_preclaim(DelegateSetPreclaimFacts {
        account_exists: false,
        authorize_exists: true,
        authorize_is_pseudo_account: false,
        permissions_empty: false,
        delegate_exists: true,
    });
    let missing_target = run_delegate_set_preclaim(DelegateSetPreclaimFacts {
        account_exists: true,
        authorize_exists: false,
        authorize_is_pseudo_account: false,
        permissions_empty: false,
        delegate_exists: true,
    });
    let pseudo_target = run_delegate_set_preclaim(DelegateSetPreclaimFacts {
        account_exists: true,
        authorize_exists: true,
        authorize_is_pseudo_account: true,
        permissions_empty: false,
        delegate_exists: true,
    });
    let missing_delete_entry = run_delegate_set_preclaim(DelegateSetPreclaimFacts {
        account_exists: true,
        authorize_exists: true,
        authorize_is_pseudo_account: false,
        permissions_empty: true,
        delegate_exists: false,
    });

    assert_eq!(missing_account, Ter::TER_NO_ACCOUNT);
    assert_eq!(missing_target, Ter::TEC_NO_TARGET);
    assert_eq!(pseudo_target, Ter::TEC_NO_PERMISSION);
    assert_eq!(missing_delete_entry, Ter::TEC_NO_ENTRY);
}

#[test]
fn delegate_set_delete_delegate_preserves_cpp_remove_order() {
    let mut sink = TestDeleteSink::new();

    let result = run_delegate_set_delete_delegate(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "delegate_exists",
            "dir_remove_owner",
            "dir_remove_destination",
            "owner_exists",
            "adjust:-1",
            "erase"
        ]
    );
    assert_eq!(sink.owner_count_deltas, vec![-1]);
    assert!(sink.erased);
}

#[test]
fn delegate_set_delete_delegate_maps_current_cpp_failures() {
    let mut missing = TestDeleteSink::new();
    missing.delegate_exists = false;
    assert_eq!(
        run_delegate_set_delete_delegate(&mut missing),
        Ter::TEC_INTERNAL
    );

    let mut dir_fail = TestDeleteSink::new();
    dir_fail.dir_remove_owner_ok = false;
    let dir_result = run_delegate_set_delete_delegate(&mut dir_fail);
    assert_eq!(dir_result, Ter::TEF_BAD_LEDGER);
    assert_eq!(trans_token(dir_result), "tefBAD_LEDGER");

    let mut dest_dir_fail = TestDeleteSink::new();
    dest_dir_fail.dir_remove_destination_result = Some(false);
    assert_eq!(
        run_delegate_set_delete_delegate(&mut dest_dir_fail),
        Ter::TEF_BAD_LEDGER
    );

    let mut missing_owner = TestDeleteSink::new();
    missing_owner.owner_exists = false;
    assert_eq!(
        run_delegate_set_delete_delegate(&mut missing_owner),
        Ter::TEC_INTERNAL
    );
}

#[test]
fn delegate_set_do_apply_updates_deletes_and_creates() {
    let mut update_sink = TestApplySink::new();
    let update = run_delegate_set_do_apply(&[5_u32, 7_u32], &mut update_sink);
    assert_eq!(update, Ter::TES_SUCCESS);
    assert_eq!(
        update_sink.events,
        [
            "owner_exists_apply",
            "delegate_exists_apply",
            "update_existing"
        ]
    );
    assert_eq!(update_sink.updated_permissions, vec![vec![5, 7]]);

    let mut delete_sink = TestApplySink::new();
    let delete = run_delegate_set_do_apply::<u32, _>(&[], &mut delete_sink);
    assert_eq!(delete, Ter::TES_SUCCESS);
    assert_eq!(
        delete_sink.events,
        [
            "owner_exists_apply",
            "delegate_exists_apply",
            "delegate_exists_delete",
            "dir_remove_owner",
            "dir_remove_destination",
            "owner_exists_delete",
            "adjust:-1",
            "erase",
        ]
    );

    let mut create_sink = TestApplySink::new();
    create_sink.delegate_exists = false;
    let create = run_delegate_set_do_apply(&[3_u32, 4_u32], &mut create_sink);
    assert_eq!(create, Ter::TES_SUCCESS);
    assert_eq!(
        create_sink.events,
        [
            "owner_exists_apply",
            "delegate_exists_apply",
            "has_reserve",
            "stage_new",
            "dir_insert_owner",
            "owner_node:9",
            "dir_insert_destination",
            "destination_node:11",
            "insert",
            "adjust:1",
        ]
    );
    assert_eq!(create_sink.staged_permissions, vec![vec![3, 4]]);
    assert!(create_sink.inserted);
}

#[test]
fn delegate_set_do_apply_maps_current_cpp_create_failures() {
    let mut missing_owner = TestApplySink::new();
    missing_owner.owner_exists = false;
    assert_eq!(
        run_delegate_set_do_apply(&[1_u32], &mut missing_owner),
        Ter::TEF_INTERNAL
    );

    let mut empty_create = TestApplySink::new();
    empty_create.delegate_exists = false;
    assert_eq!(
        run_delegate_set_do_apply::<u32, _>(&[], &mut empty_create),
        Ter::TEC_INTERNAL
    );

    let mut no_reserve = TestApplySink::new();
    no_reserve.delegate_exists = false;
    no_reserve.has_reserve = false;
    assert_eq!(
        run_delegate_set_do_apply(&[1_u32], &mut no_reserve),
        Ter::TEC_INSUFFICIENT_RESERVE
    );

    let mut dir_full = TestApplySink::new();
    dir_full.delegate_exists = false;
    dir_full.dir_owner_page = None;
    assert_eq!(
        run_delegate_set_do_apply(&[1_u32], &mut dir_full),
        Ter::TEC_DIR_FULL
    );

    let mut dest_dir_full = TestApplySink::new();
    dest_dir_full.delegate_exists = false;
    dest_dir_full.dir_dest_page = None;
    assert_eq!(
        run_delegate_set_do_apply(&[1_u32], &mut dest_dir_full),
        Ter::TEC_DIR_FULL
    );
}
