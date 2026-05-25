//! Integration tests that pin the narrowed Rust `EscrowCreate.cpp` shells to
//! the current C++ behavior.

use protocol::{SeqProxy, Ter, trans_token};
use tx::{
    EscrowCreateAmountKind, EscrowCreateApplyFacts, EscrowCreateApplySink,
    EscrowCreateIssuePreclaimFacts, EscrowCreateMptPreclaimFacts, EscrowCreatePreclaimFacts,
    EscrowCreatePreflightFacts, TxConsequences, run_escrow_create_do_apply,
    run_escrow_create_issue_preclaim, run_escrow_create_make_tx_consequences,
    run_escrow_create_mpt_preclaim, run_escrow_create_preclaim, run_escrow_create_preflight,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestSink {
    sender_page: Option<u64>,
    destination_page: Option<u64>,
    issuer_page: Option<u64>,
    lock_result: Ter,
    events: Vec<String>,
    owner_count_deltas: Vec<i32>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            sender_page: Some(3),
            destination_page: Some(4),
            issuer_page: Some(5),
            lock_result: Ter::TES_SUCCESS,
            events: Vec::new(),
            owner_count_deltas: Vec::new(),
        }
    }
}

impl EscrowCreateApplySink for TestSink {
    fn create_escrow_entry(&mut self) {
        self.events.push("create".to_string());
    }

    fn set_sequence_field(&mut self) {
        self.events.push("set_sequence".to_string());
    }

    fn set_transfer_rate(&mut self) {
        self.events.push("set_transfer_rate".to_string());
    }

    fn insert_sender_owner_dir(&mut self) -> Option<u64> {
        self.events.push("sender_dir".to_string());
        self.sender_page
    }

    fn set_sender_owner_node(&mut self, page: u64) {
        self.events.push(format!("sender_node:{page}"));
    }

    fn insert_destination_owner_dir(&mut self) -> Option<u64> {
        self.events.push("dest_dir".to_string());
        self.destination_page
    }

    fn set_destination_owner_node(&mut self, page: u64) {
        self.events.push(format!("dest_node:{page}"));
    }

    fn insert_issuer_owner_dir(&mut self) -> Option<u64> {
        self.events.push("issuer_dir".to_string());
        self.issuer_page
    }

    fn set_issuer_owner_node(&mut self, page: u64) {
        self.events.push(format!("issuer_node:{page}"));
    }

    fn deduct_xrp_owner_balance(&mut self) {
        self.events.push("deduct_xrp".to_string());
    }

    fn lock_non_xrp_amount(&mut self) -> Ter {
        self.events.push("lock_non_xrp".to_string());
        self.lock_result
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn update_owner(&mut self) {
        self.events.push("update_owner".to_string());
    }
}

#[test]
fn escrow_create_make_tx_consequences_spend_shape() {
    let xrp = run_escrow_create_make_tx_consequences(
        10,
        SeqProxy::sequence(7),
        EscrowCreateAmountKind::Xrp,
        400,
    );
    let token = run_escrow_create_make_tx_consequences(
        10,
        SeqProxy::sequence(7),
        EscrowCreateAmountKind::Issue,
        400,
    );

    assert_eq!(
        xrp,
        TxConsequences::with_potential_spend(10, SeqProxy::sequence(7), 400)
    );
    assert_eq!(token, TxConsequences::new(10, SeqProxy::sequence(7)));
}

#[test]
fn escrow_create_preflight_and_preclaim_match_cpp_guards() {
    let malformed = run_escrow_create_preflight(EscrowCreatePreflightFacts {
        amount_kind: EscrowCreateAmountKind::Xrp,
        amount_positive: true,
        feature_token_escrow_enabled: false,
        feature_mptokens_enabled: false,
        issue_has_bad_currency: false,
        mpt_amount_within_limit: true,
        cancel_after_present: false,
        finish_after_present: false,
        cancel_after_strictly_after_finish_after: true,
        condition_present: false,
        condition_valid: true,
    });
    let bad_issue_currency = run_escrow_create_preflight(EscrowCreatePreflightFacts {
        amount_kind: EscrowCreateAmountKind::Issue,
        amount_positive: true,
        feature_token_escrow_enabled: true,
        feature_mptokens_enabled: false,
        issue_has_bad_currency: true,
        mpt_amount_within_limit: true,
        cancel_after_present: true,
        finish_after_present: true,
        cancel_after_strictly_after_finish_after: true,
        condition_present: false,
        condition_valid: true,
    });
    let issue_preclaim = run_escrow_create_issue_preclaim(EscrowCreateIssuePreclaimFacts {
        issuer_equals_account: false,
        issuer_exists: true,
        issuer_allows_trustline_locking: true,
        trustline_exists: true,
        trustline_balance_sign_valid: true,
        sender_auth_result: Ter::TES_SUCCESS,
        destination_auth_result: Ter::TES_SUCCESS,
        sender_frozen: false,
        destination_frozen: false,
        spendable_amount_positive: true,
        spendable_amount_covers_amount: true,
        can_add_amount: true,
    });
    let mpt_locked = run_escrow_create_mpt_preclaim(EscrowCreateMptPreclaimFacts {
        issuer_equals_account: false,
        issuance_exists: true,
        issuance_can_escrow: true,
        issuance_issuer_matches: true,
        sender_token_exists: true,
        sender_auth_result: Ter::TES_SUCCESS,
        destination_auth_result: Ter::TES_SUCCESS,
        sender_locked: true,
        destination_locked: false,
        can_transfer_result: Ter::TES_SUCCESS,
        spendable_amount_positive: true,
        spendable_amount_covers_amount: true,
    });
    let outer = run_escrow_create_preclaim(EscrowCreatePreclaimFacts {
        destination_exists: true,
        destination_is_pseudo_account: false,
        amount_kind: EscrowCreateAmountKind::Issue,
        token_escrow_enabled: true,
        asset_preclaim_result: Ter::TES_SUCCESS,
    });

    assert_eq!(malformed, Ter::TEM_BAD_EXPIRATION);
    assert_eq!(bad_issue_currency, Ter::TEM_BAD_CURRENCY);
    assert_eq!(issue_preclaim, Ter::TES_SUCCESS);
    assert_eq!(mpt_locked, Ter::TEC_LOCKED);
    assert_eq!(outer, Ter::TES_SUCCESS);
}

#[test]
fn escrow_create_do_apply_preserves_for_xrp_and_token_paths() {
    let mut xrp = TestSink::new();
    let xrp_result = run_escrow_create_do_apply(
        EscrowCreateApplyFacts {
            cancel_after_expired: false,
            finish_after_expired: false,
            owner_exists: true,
            reserve_sufficient: true,
            amount_is_xrp: true,
            xrp_balance_covers_amount: true,
            destination_exists: true,
            destination_requires_tag: false,
            destination_tag_present: false,
            include_sequence_field: true,
            should_set_transfer_rate: false,
            destination_is_sender: false,
            issuer_owner_dir_required: false,
        },
        &mut xrp,
    );

    assert_eq!(xrp_result, Ter::TES_SUCCESS);
    assert_eq!(
        xrp.events,
        [
            "create",
            "set_sequence",
            "sender_dir",
            "sender_node:3",
            "dest_dir",
            "dest_node:4",
            "deduct_xrp",
            "adjust:1",
            "update_owner",
        ]
    );

    let mut token = TestSink::new();
    let token_result = run_escrow_create_do_apply(
        EscrowCreateApplyFacts {
            cancel_after_expired: false,
            finish_after_expired: false,
            owner_exists: true,
            reserve_sufficient: true,
            amount_is_xrp: false,
            xrp_balance_covers_amount: false,
            destination_exists: true,
            destination_requires_tag: false,
            destination_tag_present: false,
            include_sequence_field: false,
            should_set_transfer_rate: true,
            destination_is_sender: true,
            issuer_owner_dir_required: true,
        },
        &mut token,
    );

    assert_eq!(token_result, Ter::TES_SUCCESS);
    assert_eq!(
        token.events,
        [
            "create",
            "set_transfer_rate",
            "sender_dir",
            "sender_node:3",
            "issuer_dir",
            "issuer_node:5",
            "lock_non_xrp",
            "adjust:1",
            "update_owner",
        ]
    );
}

#[test]
fn escrow_create_do_apply_maps_cpp_failures() {
    let mut sink = TestSink::new();
    sink.sender_page = None;

    let dir_full = run_escrow_create_do_apply(
        EscrowCreateApplyFacts {
            cancel_after_expired: false,
            finish_after_expired: false,
            owner_exists: true,
            reserve_sufficient: true,
            amount_is_xrp: true,
            xrp_balance_covers_amount: true,
            destination_exists: true,
            destination_requires_tag: false,
            destination_tag_present: false,
            include_sequence_field: false,
            should_set_transfer_rate: false,
            destination_is_sender: true,
            issuer_owner_dir_required: false,
        },
        &mut sink,
    );
    assert_eq!(dir_full, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(dir_full), "tecDIR_FULL");
}
