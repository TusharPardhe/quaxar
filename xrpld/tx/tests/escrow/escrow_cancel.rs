//! Integration tests that pin the narrowed Rust `EscrowCancel.cpp` shells to
//! the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    EscrowCancelApplySink, EscrowCancelIssuePreclaimFacts, EscrowCancelMptPreclaimFacts,
    EscrowCancelPreclaimFacts, run_escrow_cancel_do_apply, run_escrow_cancel_issue_preclaim,
    run_escrow_cancel_mpt_preclaim, run_escrow_cancel_preclaim, run_escrow_cancel_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestSink {
    escrow_exists: bool,
    token_escrow_enabled: bool,
    cancel_after_present: bool,
    cancel_after_passed: bool,
    destination_node_present: bool,
    destination_remove_ok: bool,
    amount_is_xrp: bool,
    token_unlock_result: Ter,
    issuer_node_present: bool,
    issuer_remove_ok: bool,
    owner_exists: bool,
    events: Vec<String>,
    owner_count_deltas: Vec<i32>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            escrow_exists: true,
            token_escrow_enabled: true,
            cancel_after_present: true,
            cancel_after_passed: true,
            destination_node_present: true,
            destination_remove_ok: true,
            amount_is_xrp: true,
            token_unlock_result: Ter::TES_SUCCESS,
            issuer_node_present: false,
            issuer_remove_ok: true,
            owner_exists: true,
            events: Vec::new(),
            owner_count_deltas: Vec::new(),
        }
    }
}

impl EscrowCancelApplySink for TestSink {
    fn escrow_exists(&mut self) -> bool {
        self.events.push("escrow_exists".to_string());
        self.escrow_exists
    }

    fn token_escrow_enabled(&mut self) -> bool {
        self.events.push("token_enabled".to_string());
        self.token_escrow_enabled
    }

    fn cancel_after_present(&mut self) -> bool {
        self.events.push("cancel_after_present".to_string());
        self.cancel_after_present
    }

    fn cancel_after_passed(&mut self) -> bool {
        self.events.push("cancel_after_passed".to_string());
        self.cancel_after_passed
    }

    fn remove_owner_dir(&mut self) -> bool {
        self.events.push("remove_owner_dir".to_string());
        true
    }

    fn destination_node_present(&mut self) -> bool {
        self.events.push("dest_node_present".to_string());
        self.destination_node_present
    }

    fn remove_destination_dir(&mut self) -> bool {
        self.events.push("remove_dest_dir".to_string());
        self.destination_remove_ok
    }

    fn amount_is_xrp(&mut self) -> bool {
        self.events.push("amount_is_xrp".to_string());
        self.amount_is_xrp
    }

    fn credit_owner_xrp(&mut self) {
        self.events.push("credit_xrp".to_string());
    }

    fn apply_token_unlock(&mut self) -> Ter {
        self.events.push("token_unlock".to_string());
        self.token_unlock_result
    }

    fn issuer_node_present(&mut self) -> bool {
        self.events.push("issuer_node_present".to_string());
        self.issuer_node_present
    }

    fn remove_issuer_dir(&mut self) -> bool {
        self.events.push("remove_issuer_dir".to_string());
        self.issuer_remove_ok
    }

    fn owner_exists(&mut self) -> bool {
        self.events.push("owner_exists".to_string());
        self.owner_exists
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn update_owner(&mut self) {
        self.events.push("update_owner".to_string());
    }

    fn erase_escrow(&mut self) {
        self.events.push("erase".to_string());
    }
}

#[test]
fn escrow_cancel_preflight_and_preclaim_helpers_match_cpp() {
    assert_eq!(run_escrow_cancel_preflight(), Ter::TES_SUCCESS);
    assert_eq!(
        run_escrow_cancel_issue_preclaim(EscrowCancelIssuePreclaimFacts {
            issuer_equals_account: true,
            require_auth_result: Ter::TES_SUCCESS,
        }),
        Ter::TEC_INTERNAL
    );
    assert_eq!(
        run_escrow_cancel_mpt_preclaim(EscrowCancelMptPreclaimFacts {
            issuer_equals_account: false,
            issuance_exists: false,
            require_auth_result: Ter::TES_SUCCESS,
        }),
        Ter::TEC_OBJECT_NOT_FOUND
    );
    assert_eq!(
        run_escrow_cancel_preclaim(EscrowCancelPreclaimFacts {
            token_escrow_enabled: true,
            escrow_exists: false,
            amount_is_xrp: true,
            asset_preclaim_result: Ter::TES_SUCCESS,
        }),
        Ter::TEC_NO_TARGET
    );
}

#[test]
fn escrow_cancel_do_apply_preserves_cpp_xrp_order() {
    let mut sink = TestSink::new();

    let result = run_escrow_cancel_do_apply(&mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "escrow_exists",
            "cancel_after_present",
            "cancel_after_passed",
            "remove_owner_dir",
            "dest_node_present",
            "remove_dest_dir",
            "amount_is_xrp",
            "credit_xrp",
            "owner_exists",
            "adjust:-1",
            "update_owner",
            "erase",
        ]
    );
    assert_eq!(sink.owner_count_deltas, vec![-1]);
}

#[test]
fn escrow_cancel_do_apply_preserves_cpp_token_branch_and_failures() {
    let mut token = TestSink::new();
    token.amount_is_xrp = false;
    token.issuer_node_present = true;

    let token_result = run_escrow_cancel_do_apply(&mut token);
    assert_eq!(token_result, Ter::TES_SUCCESS);
    assert!(token.events.contains(&"token_unlock".to_string()));
    assert!(token.events.contains(&"remove_issuer_dir".to_string()));

    let mut too_soon = TestSink::new();
    too_soon.cancel_after_passed = false;
    let too_soon_result = run_escrow_cancel_do_apply(&mut too_soon);
    assert_eq!(too_soon_result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(too_soon_result), "tecNO_PERMISSION");

    let mut disabled = TestSink::new();
    disabled.amount_is_xrp = false;
    disabled.token_escrow_enabled = false;
    let disabled_result = run_escrow_cancel_do_apply(&mut disabled);
    assert_eq!(disabled_result, Ter::TEM_DISABLED);
}
