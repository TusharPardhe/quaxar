//! Integration tests that pin the landed Rust `XChainBridge.cpp` bridge-owner
//! subset to the current C++ behavior.

use protocol::{
    AccountID, IOUAmount, Issue, STAmount, Ter,
    XCHAIN_MODIFY_BRIDGE_CLEAR_ACCOUNT_CREATE_AMOUNT_FLAG, XCHAIN_MODIFY_BRIDGE_FLAGS_MASK,
    currency_from_string, genesis_account_id, sf_generic, trans_token, xrp_issue,
};
use tx::{
    XBRIDGE_MAX_ACCOUNT_CREATE_CLAIMS, XChainBridgeChainType, XChainBridgeSpec,
    XChainCreateBridgeApplyFacts, XChainCreateBridgeApplySink, XChainCreateBridgeMutation,
    XChainCreateBridgePreclaimFacts, XChainCreateBridgePreflightFacts,
    XChainModifyBridgeApplyFacts, XChainModifyBridgeApplySink, XChainModifyBridgePreclaimFacts,
    XChainModifyBridgePreflightFacts, run_xchain_create_bridge_do_apply,
    run_xchain_create_bridge_preclaim, run_xchain_create_bridge_preflight,
    run_xchain_modify_bridge_do_apply, run_xchain_modify_bridge_get_flags_mask,
    run_xchain_modify_bridge_preclaim, run_xchain_modify_bridge_preflight,
};

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn genesis_account() -> AccountID {
    AccountID::from_slice(genesis_account_id().data())
        .expect("genesis account id width must match AccountID")
}

fn usd_issue(issuer: AccountID) -> Issue {
    Issue::new(currency_from_string("USD"), issuer)
}

fn xrp_amount(drops: u64) -> STAmount {
    STAmount::new_native(drops, false)
}

fn neg_xrp_amount(drops: u64) -> STAmount {
    STAmount::new_native(drops, true)
}

fn iou_amount(value: i64, issuer: AccountID) -> STAmount {
    STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(value, 0).expect("IOU amount should normalize"),
        usd_issue(issuer),
    )
}

fn xrp_bridge(locking_door: AccountID, issuing_door: AccountID) -> XChainBridgeSpec {
    XChainBridgeSpec {
        locking_chain_door: locking_door,
        locking_chain_issue: xrp_issue(),
        issuing_chain_door: issuing_door,
        issuing_chain_issue: xrp_issue(),
    }
}

fn iou_bridge(
    locking_door: AccountID,
    locking_issue: Issue,
    issuing_door: AccountID,
    issuing_issue: Issue,
) -> XChainBridgeSpec {
    XChainBridgeSpec {
        locking_chain_door: locking_door,
        locking_chain_issue: locking_issue,
        issuing_chain_door: issuing_door,
        issuing_chain_issue: issuing_issue,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCreateSink {
    account_exists: bool,
    owner_dir_page: Option<u64>,
    owner_count_deltas: Vec<i32>,
    created: Option<XChainCreateBridgeMutation>,
    updated_account: bool,
    events: Vec<String>,
}

impl TestCreateSink {
    fn new() -> Self {
        Self {
            account_exists: true,
            owner_dir_page: Some(77),
            owner_count_deltas: Vec::new(),
            created: None,
            updated_account: false,
            events: Vec::new(),
        }
    }
}

impl XChainCreateBridgeApplySink for TestCreateSink {
    fn account_exists(&mut self) -> bool {
        self.events.push("account_exists".to_string());
        self.account_exists
    }

    fn insert_owner_dir(&mut self) -> Option<u64> {
        self.events.push("owner_dir".to_string());
        self.owner_dir_page
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("owner_count:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn create_bridge(&mut self, mutation: XChainCreateBridgeMutation) {
        self.events.push("create_bridge".to_string());
        self.created = Some(mutation);
    }

    fn update_account(&mut self) {
        self.events.push("update_account".to_string());
        self.updated_account = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestModifySink {
    account_exists: bool,
    bridge_exists: bool,
    set_rewards: Vec<STAmount>,
    set_min_account_create: Vec<STAmount>,
    cleared: usize,
    finished: usize,
    events: Vec<String>,
}

impl TestModifySink {
    fn new() -> Self {
        Self {
            account_exists: true,
            bridge_exists: true,
            set_rewards: Vec::new(),
            set_min_account_create: Vec::new(),
            cleared: 0,
            finished: 0,
            events: Vec::new(),
        }
    }
}

impl XChainModifyBridgeApplySink for TestModifySink {
    fn account_exists(&mut self) -> bool {
        self.events.push("account_exists".to_string());
        self.account_exists
    }

    fn bridge_exists(&mut self) -> bool {
        self.events.push("bridge_exists".to_string());
        self.bridge_exists
    }

    fn set_reward(&mut self, reward: STAmount) {
        self.events.push("set_reward".to_string());
        self.set_rewards.push(reward);
    }

    fn set_min_account_create(&mut self, amount: STAmount) {
        self.events.push("set_min_account_create".to_string());
        self.set_min_account_create.push(amount);
    }

    fn clear_min_account_create_if_present(&mut self) {
        self.events.push("clear_min_account_create".to_string());
        self.cleared += 1;
    }

    fn finish_bridge_update(&mut self) {
        self.events.push("finish".to_string());
        self.finished += 1;
    }
}

#[test]
fn xchain_bridge_constant_and_chain_helpers_match_cpp_shape() {
    assert_eq!(XBRIDGE_MAX_ACCOUNT_CREATE_CLAIMS, 128);
    assert_eq!(
        XChainBridgeSpec::other_chain(XChainBridgeChainType::Locking),
        XChainBridgeChainType::Issuing
    );
    assert_eq!(
        XChainBridgeSpec::src_chain(true),
        XChainBridgeChainType::Locking
    );
    assert_eq!(
        XChainBridgeSpec::dst_chain(true),
        XChainBridgeChainType::Issuing
    );
}

#[test]
fn xchain_create_bridge_preflight_rejects_current_cpp_malformed_cases() {
    let locking = account("1111111111111111111111111111111111111111");
    let other = account("2222222222222222222222222222222222222222");
    let issuer = account("3333333333333333333333333333333333333333");

    let equal_doors = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: xrp_amount(10),
        min_account_create: None,
        bridge: xrp_bridge(locking, locking),
    });
    assert_eq!(equal_doors, Ter::TEM_XCHAIN_EQUAL_DOOR_ACCOUNTS);

    let non_door = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: other,
        reward: xrp_amount(10),
        min_account_create: None,
        bridge: xrp_bridge(locking, genesis_account()),
    });
    assert_eq!(non_door, Ter::TEM_XCHAIN_BRIDGE_NONDOOR_OWNER);

    let mixed_issues = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: xrp_amount(10),
        min_account_create: None,
        bridge: XChainBridgeSpec {
            locking_chain_door: locking,
            locking_chain_issue: xrp_issue(),
            issuing_chain_door: issuer,
            issuing_chain_issue: usd_issue(issuer),
        },
    });
    assert_eq!(mixed_issues, Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES);

    let bad_reward = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: iou_amount(1, issuer),
        min_account_create: None,
        bridge: xrp_bridge(locking, genesis_account()),
    });
    assert_eq!(bad_reward, Ter::TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT);

    let negative_reward = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: neg_xrp_amount(1),
        min_account_create: None,
        bridge: xrp_bridge(locking, genesis_account()),
    });
    assert_eq!(negative_reward, Ter::TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT);

    let bad_min = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: xrp_amount(10),
        min_account_create: Some(iou_amount(1, issuer)),
        bridge: xrp_bridge(locking, genesis_account()),
    });
    assert_eq!(
        bad_min,
        Ter::TEM_XCHAIN_BRIDGE_BAD_MIN_ACCOUNT_CREATE_AMOUNT
    );

    let wrong_xrp_issuer = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: xrp_amount(10),
        min_account_create: None,
        bridge: xrp_bridge(locking, other),
    });
    assert_eq!(wrong_xrp_issuer, Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES);

    let wrong_iou_issuer = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: xrp_amount(10),
        min_account_create: None,
        bridge: iou_bridge(locking, usd_issue(issuer), other, usd_issue(issuer)),
    });
    assert_eq!(wrong_iou_issuer, Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES);

    let locking_owns_locked_issue =
        run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
            account: locking,
            reward: xrp_amount(10),
            min_account_create: None,
            bridge: iou_bridge(locking, usd_issue(locking), issuer, usd_issue(issuer)),
        });
    assert_eq!(locking_owns_locked_issue, Ter::TEM_XCHAIN_BRIDGE_BAD_ISSUES);
}

#[test]
fn xchain_create_bridge_preflight_accepts_valid_xrp_and_iou_bridges() {
    let locking = account("1111111111111111111111111111111111111111");
    let issuing = account("2222222222222222222222222222222222222222");

    let xrp = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: xrp_amount(10),
        min_account_create: Some(xrp_amount(1000)),
        bridge: xrp_bridge(locking, genesis_account()),
    });
    assert_eq!(xrp, Ter::TES_SUCCESS);

    let iou = run_xchain_create_bridge_preflight(XChainCreateBridgePreflightFacts {
        account: locking,
        reward: xrp_amount(10),
        min_account_create: None,
        bridge: iou_bridge(locking, usd_issue(issuing), issuing, usd_issue(issuing)),
    });
    assert_eq!(iou, Ter::TES_SUCCESS);
}

#[test]
fn xchain_create_bridge_preclaim_ordering_and_errors() {
    let locking = account("1111111111111111111111111111111111111111");
    let issuer = account("2222222222222222222222222222222222222222");
    let bridge = iou_bridge(locking, usd_issue(issuer), issuer, usd_issue(issuer));

    let duplicate = run_xchain_create_bridge_preclaim(XChainCreateBridgePreclaimFacts {
        account: locking,
        bridge,
        bridge_exists_on_locking: true,
        bridge_exists_on_issuing: false,
        source_issue_issuer_exists: false,
        source_issue_allows_clawback: true,
        account_exists: false,
        reserve_sufficient: false,
    });
    assert_eq!(duplicate, Ter::TEC_DUPLICATE);

    let no_issuer = run_xchain_create_bridge_preclaim(XChainCreateBridgePreclaimFacts {
        account: locking,
        bridge,
        bridge_exists_on_locking: false,
        bridge_exists_on_issuing: false,
        source_issue_issuer_exists: false,
        source_issue_allows_clawback: false,
        account_exists: true,
        reserve_sufficient: true,
    });
    assert_eq!(no_issuer, Ter::TEC_NO_ISSUER);

    let no_permission = run_xchain_create_bridge_preclaim(XChainCreateBridgePreclaimFacts {
        account: locking,
        bridge,
        bridge_exists_on_locking: false,
        bridge_exists_on_issuing: false,
        source_issue_issuer_exists: true,
        source_issue_allows_clawback: true,
        account_exists: true,
        reserve_sufficient: true,
    });
    assert_eq!(no_permission, Ter::TEC_NO_PERMISSION);

    let no_account = run_xchain_create_bridge_preclaim(XChainCreateBridgePreclaimFacts {
        account: locking,
        bridge: xrp_bridge(locking, genesis_account()),
        bridge_exists_on_locking: false,
        bridge_exists_on_issuing: false,
        source_issue_issuer_exists: true,
        source_issue_allows_clawback: false,
        account_exists: false,
        reserve_sufficient: true,
    });
    assert_eq!(no_account, Ter::TER_NO_ACCOUNT);

    let no_reserve = run_xchain_create_bridge_preclaim(XChainCreateBridgePreclaimFacts {
        account: locking,
        bridge: xrp_bridge(locking, genesis_account()),
        bridge_exists_on_locking: false,
        bridge_exists_on_issuing: false,
        source_issue_issuer_exists: true,
        source_issue_allows_clawback: false,
        account_exists: true,
        reserve_sufficient: false,
    });
    assert_eq!(no_reserve, Ter::TEC_INSUFFICIENT_RESERVE);

    let success = run_xchain_create_bridge_preclaim(XChainCreateBridgePreclaimFacts {
        account: locking,
        bridge: xrp_bridge(locking, genesis_account()),
        bridge_exists_on_locking: false,
        bridge_exists_on_issuing: false,
        source_issue_issuer_exists: true,
        source_issue_allows_clawback: false,
        account_exists: true,
        reserve_sufficient: true,
    });
    assert_eq!(success, Ter::TES_SUCCESS);
}

#[test]
fn xchain_create_bridge_do_apply_preserves_current_and_initial_fields() {
    let locking = account("1111111111111111111111111111111111111111");
    let bridge = xrp_bridge(locking, genesis_account());

    let mut missing = TestCreateSink::new();
    missing.account_exists = false;
    let missing_result = run_xchain_create_bridge_do_apply(
        XChainCreateBridgeApplyFacts {
            account: locking,
            reward: xrp_amount(10),
            min_account_create: Some(xrp_amount(100)),
            bridge,
        },
        &mut missing,
    );
    assert_eq!(missing_result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(missing_result), "tecINTERNAL");
    assert_eq!(missing.events, ["account_exists"]);

    let mut dir_full = TestCreateSink::new();
    dir_full.owner_dir_page = None;
    let dir_result = run_xchain_create_bridge_do_apply(
        XChainCreateBridgeApplyFacts {
            account: locking,
            reward: xrp_amount(10),
            min_account_create: Some(xrp_amount(100)),
            bridge,
        },
        &mut dir_full,
    );
    assert_eq!(dir_result, Ter::TEC_DIR_FULL);
    assert_eq!(dir_full.events, ["account_exists", "owner_dir"]);

    let mut sink = TestCreateSink::new();
    let result = run_xchain_create_bridge_do_apply(
        XChainCreateBridgeApplyFacts {
            account: locking,
            reward: xrp_amount(10),
            min_account_create: Some(xrp_amount(100)),
            bridge,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "account_exists",
            "owner_dir",
            "owner_count:1",
            "create_bridge",
            "update_account"
        ]
    );
    assert_eq!(sink.owner_count_deltas, vec![1]);
    assert!(sink.updated_account);
    assert_eq!(
        sink.created,
        Some(XChainCreateBridgeMutation {
            account: locking,
            reward: xrp_amount(10),
            min_account_create: Some(xrp_amount(100)),
            bridge,
            chain_type: XChainBridgeChainType::Locking,
            xchain_claim_id: 0,
            xchain_account_create_count: 0,
            xchain_account_claim_count: 0,
            owner_node: 77,
        })
    );
}

#[test]
fn xchain_modify_bridge_flags_and_preflight_match_cpp() {
    let locking = account("1111111111111111111111111111111111111111");
    let bridge = xrp_bridge(locking, genesis_account());

    assert_eq!(
        run_xchain_modify_bridge_get_flags_mask(),
        XCHAIN_MODIFY_BRIDGE_FLAGS_MASK
    );

    let none = run_xchain_modify_bridge_preflight(XChainModifyBridgePreflightFacts {
        account: locking,
        reward: None,
        min_account_create: None,
        clear_account_create: false,
        bridge,
    });
    assert_eq!(none, Ter::TEM_MALFORMED);

    let both = run_xchain_modify_bridge_preflight(XChainModifyBridgePreflightFacts {
        account: locking,
        reward: None,
        min_account_create: Some(xrp_amount(10)),
        clear_account_create: true,
        bridge,
    });
    assert_eq!(both, Ter::TEM_MALFORMED);

    let non_door = run_xchain_modify_bridge_preflight(XChainModifyBridgePreflightFacts {
        account: account("2222222222222222222222222222222222222222"),
        reward: Some(xrp_amount(1)),
        min_account_create: None,
        clear_account_create: false,
        bridge,
    });
    assert_eq!(non_door, Ter::TEM_XCHAIN_BRIDGE_NONDOOR_OWNER);

    let bad_reward = run_xchain_modify_bridge_preflight(XChainModifyBridgePreflightFacts {
        account: locking,
        reward: Some(iou_amount(
            1,
            account("3333333333333333333333333333333333333333"),
        )),
        min_account_create: None,
        clear_account_create: false,
        bridge,
    });
    assert_eq!(bad_reward, Ter::TEM_XCHAIN_BRIDGE_BAD_REWARD_AMOUNT);

    let bad_min = run_xchain_modify_bridge_preflight(XChainModifyBridgePreflightFacts {
        account: locking,
        reward: Some(xrp_amount(1)),
        min_account_create: Some(neg_xrp_amount(1)),
        clear_account_create: false,
        bridge,
    });
    assert_eq!(
        bad_min,
        Ter::TEM_XCHAIN_BRIDGE_BAD_MIN_ACCOUNT_CREATE_AMOUNT
    );

    let ok = run_xchain_modify_bridge_preflight(XChainModifyBridgePreflightFacts {
        account: locking,
        reward: Some(xrp_amount(2)),
        min_account_create: None,
        clear_account_create: true,
        bridge,
    });
    assert_eq!(ok, Ter::TES_SUCCESS);
}

#[test]
fn xchain_modify_bridge_preclaim_and_do_apply_match_cpp() {
    let no_entry = run_xchain_modify_bridge_preclaim(XChainModifyBridgePreclaimFacts {
        bridge_exists: false,
    });
    assert_eq!(no_entry, Ter::TEC_NO_ENTRY);

    let success = run_xchain_modify_bridge_preclaim(XChainModifyBridgePreclaimFacts {
        bridge_exists: true,
    });
    assert_eq!(success, Ter::TES_SUCCESS);

    let mut missing_account = TestModifySink::new();
    missing_account.account_exists = false;
    let missing_account_result = run_xchain_modify_bridge_do_apply(
        XChainModifyBridgeApplyFacts {
            reward: Some(xrp_amount(5)),
            min_account_create: None,
            clear_account_create: false,
        },
        &mut missing_account,
    );
    assert_eq!(missing_account_result, Ter::TEC_INTERNAL);
    assert_eq!(missing_account.events, ["account_exists"]);

    let mut missing_bridge = TestModifySink::new();
    missing_bridge.bridge_exists = false;
    let missing_bridge_result = run_xchain_modify_bridge_do_apply(
        XChainModifyBridgeApplyFacts {
            reward: Some(xrp_amount(5)),
            min_account_create: None,
            clear_account_create: false,
        },
        &mut missing_bridge,
    );
    assert_eq!(missing_bridge_result, Ter::TEC_INTERNAL);
    assert_eq!(missing_bridge.events, ["account_exists", "bridge_exists"]);

    let mut sink = TestModifySink::new();
    let result = run_xchain_modify_bridge_do_apply(
        XChainModifyBridgeApplyFacts {
            reward: Some(xrp_amount(5)),
            min_account_create: Some(xrp_amount(25)),
            clear_account_create: true,
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "account_exists",
            "bridge_exists",
            "set_reward",
            "set_min_account_create",
            "clear_min_account_create",
            "finish"
        ]
    );
    assert_eq!(sink.set_rewards, vec![xrp_amount(5)]);
    assert_eq!(sink.set_min_account_create, vec![xrp_amount(25)]);
    assert_eq!(sink.cleared, 1);
    assert_eq!(sink.finished, 1);
    assert_eq!(
        XCHAIN_MODIFY_BRIDGE_CLEAR_ACCOUNT_CREATE_AMOUNT_FLAG,
        0x0001_0000
    );
}
