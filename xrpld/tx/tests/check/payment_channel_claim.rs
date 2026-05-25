//! Integration tests that pin the narrowed Rust `PaymentChannelClaim.cpp`
//! shell to the current C++ behavior.

use std::cell::Cell;

use basics::base_uint::Uint256;
use protocol::{Ter, trans_token};
use tx::payment_channel_due::PaymentChannelDueFacts;
use tx::payment_channel_helpers::PaymentChannelCloseFacts;
use tx::{
    PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG, PAYMENT_CHANNEL_CLAIM_RENEW_FLAG,
    PaymentChannelClaimApplyFacts, PaymentChannelClaimApplySink,
    PaymentChannelClaimAuthorizationMessageFacts, PaymentChannelClaimPreflightFacts,
    PaymentChannelClaimPreparedDoApplyFacts, PaymentChannelClaimSignaturePreflightFacts,
    build_payment_channel_claim_prepared_do_apply_facts, get_payment_channel_claim_flags_mask,
    run_payment_channel_claim_check_extra_features, run_payment_channel_claim_do_apply,
    run_payment_channel_claim_preclaim, run_payment_channel_claim_preflight,
    run_payment_channel_claim_prepared_do_apply, run_payment_channel_claim_signature_preflight,
    serialize_payment_channel_claim_authorization_message,
};

#[derive(Debug, Default)]
struct TestApplySink {
    destination_exists_result: bool,
    deposit_preauth_result: Ter,
    source_dir_result: Ter,
    destination_dir_result: Ter,
    source_account_exists: bool,
    events: Vec<String>,
    expirations: Vec<u32>,
    refund_drops: Option<u64>,
    owner_count_deltas: Vec<i32>,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            destination_exists_result: true,
            deposit_preauth_result: Ter::TES_SUCCESS,
            source_dir_result: Ter::TES_SUCCESS,
            destination_dir_result: Ter::TES_SUCCESS,
            source_account_exists: true,
            events: Vec::new(),
            expirations: Vec::new(),
            refund_drops: None,
            owner_count_deltas: Vec::new(),
        }
    }
}

impl PaymentChannelClaimApplySink<u32> for TestApplySink {
    fn remove_source_owner_directory(&mut self) -> Ter {
        self.events
            .push("remove_source_owner_directory".to_string());
        self.source_dir_result
    }

    fn remove_destination_owner_directory(&mut self) -> Ter {
        self.events
            .push("remove_destination_owner_directory".to_string());
        self.destination_dir_result
    }

    fn source_account_exists(&mut self) -> bool {
        self.events.push("source_account_exists".to_string());
        self.source_account_exists
    }

    fn apply_refund_to_source_account(&mut self, refund_drops: u64) {
        self.events
            .push("apply_refund_to_source_account".to_string());
        self.refund_drops = Some(refund_drops);
    }

    fn adjust_source_owner_count(&mut self, delta: i32) {
        self.events
            .push(format!("adjust_source_owner_count:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn erase_channel(&mut self) {
        self.events.push("erase_channel".to_string());
    }

    fn destination_exists(&mut self) -> bool {
        self.events.push("destination_exists".to_string());
        self.destination_exists_result
    }

    fn verify_deposit_preauth(&mut self) -> Ter {
        self.events.push("verify_deposit_preauth".to_string());
        self.deposit_preauth_result
    }

    fn set_channel_balance(&mut self, balance_drops: u64) {
        self.events
            .push(format!("set_channel_balance:{balance_drops}"));
    }

    fn add_destination_balance(&mut self, delta_drops: u64) {
        self.events
            .push(format!("add_destination_balance:{delta_drops}"));
    }

    fn persist_destination_balance(&mut self) {
        self.events.push("persist_destination_balance".to_string());
    }

    fn persist_channel_balance(&mut self) {
        self.events.push("persist_channel_balance".to_string());
    }

    fn clear_expiration(&mut self) {
        self.events.push("clear_expiration".to_string());
    }

    fn set_expiration(&mut self, expiration: u32) {
        self.events.push("set_expiration".to_string());
        self.expirations.push(expiration);
    }
}

fn preflight_facts() -> PaymentChannelClaimPreflightFacts {
    PaymentChannelClaimPreflightFacts {
        balance_present: false,
        balance_is_xrp: true,
        balance_positive: true,
        amount_present: false,
        amount_is_xrp: true,
        amount_positive: true,
        balance_exceeds_amount: false,
        tx_flags: 0,
        signature: None,
    }
}

fn signature_facts() -> PaymentChannelClaimSignaturePreflightFacts {
    PaymentChannelClaimSignaturePreflightFacts {
        public_key_present: true,
        requested_balance_drops: 400,
        authorization_message: PaymentChannelClaimAuthorizationMessageFacts {
            channel_key: Uint256::from_hex(
                "0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210",
            )
            .expect("channel key should parse"),
            authorized_amount_drops: 500,
        },
        public_key_type_valid: true,
    }
}

fn signature_preflight_facts() -> PaymentChannelClaimPreflightFacts {
    PaymentChannelClaimPreflightFacts {
        balance_present: true,
        signature: Some(signature_facts()),
        ..preflight_facts()
    }
}

fn apply_facts() -> PaymentChannelClaimApplyFacts<u32> {
    PaymentChannelClaimApplyFacts {
        channel_exists: true,
        due_facts: PaymentChannelDueFacts {
            cancel_after: None,
            expiration: None,
            close_time: 60,
        },
        close_facts: PaymentChannelCloseFacts {
            destination_owner_directory_present: true,
            channel_amount_drops: 1_000,
            channel_balance_drops: 250,
        },
        tx_account_is_source: true,
        tx_account_is_destination: false,
        balance_present: false,
        signature_present: false,
        provided_public_key_matches_channel: true,
        requested_balance_exceeds_channel_funds: false,
        requested_balance_not_above_channel_balance: false,
        channel_balance_drops: 250,
        requested_balance_drops: 400,
        renew_flag: false,
        close_flag: false,
        channel_fully_paid: false,
        current_expiration: None,
        close_time: 60,
        settle_delay: 30,
    }
}

fn prepared_do_apply_facts() -> PaymentChannelClaimPreparedDoApplyFacts<u32> {
    build_payment_channel_claim_prepared_do_apply_facts(apply_facts())
}

#[test]
fn payment_channel_claim_feature_gate() {
    assert!(run_payment_channel_claim_check_extra_features(false, false));
    assert!(run_payment_channel_claim_check_extra_features(true, true));
    assert!(!run_payment_channel_claim_check_extra_features(true, false));
}

#[test]
fn payment_channel_claim_flags_mask() {
    assert_eq!(get_payment_channel_claim_flags_mask(), 0x3ffc_ffff);
}

#[test]
fn payment_channel_claim_preflight_ordering() {
    assert_eq!(
        run_payment_channel_claim_preflight(
            PaymentChannelClaimPreflightFacts {
                balance_present: true,
                balance_positive: false,
                ..preflight_facts()
            },
            |_| panic!("verifier should not run after bad balance"),
            || panic!("credentials should not run after bad balance"),
        ),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_payment_channel_claim_preflight(
            PaymentChannelClaimPreflightFacts {
                amount_present: true,
                amount_is_xrp: false,
                ..preflight_facts()
            },
            |_| panic!("verifier should not run after bad amount"),
            || panic!("credentials should not run after bad amount"),
        ),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_payment_channel_claim_preflight(
            PaymentChannelClaimPreflightFacts {
                balance_exceeds_amount: true,
                ..preflight_facts()
            },
            |_| panic!("verifier should not run after balance > amount"),
            || panic!("credentials should not run after balance > amount"),
        ),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_payment_channel_claim_preflight(
            PaymentChannelClaimPreflightFacts {
                tx_flags: PAYMENT_CHANNEL_CLAIM_RENEW_FLAG | PAYMENT_CHANNEL_CLAIM_CLOSE_FLAG,
                ..preflight_facts()
            },
            |_| panic!("verifier should not run after malformed flags"),
            || panic!("credentials should not run after malformed flags"),
        ),
        Ter::TEM_MALFORMED
    );
}

#[test]
fn payment_channel_claim_signature_helper_authorization_layout() {
    let message = serialize_payment_channel_claim_authorization_message(
        &signature_facts().authorization_message,
    );

    assert_eq!(
        message,
        vec![
            0x43, 0x4c, 0x4d, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0xFE, 0xDC,
            0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF,
            0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01, 0xF4
        ]
    );
}

#[test]
fn payment_channel_claim_signature_helper_failure_order() {
    assert_eq!(
        run_payment_channel_claim_signature_preflight(false, signature_facts(), |_| panic!(
            "verifier should not run without balance"
        ),),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_payment_channel_claim_signature_preflight(
            true,
            PaymentChannelClaimSignaturePreflightFacts {
                public_key_present: false,
                ..signature_facts()
            },
            |_| panic!("verifier should not run without public key"),
        ),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_payment_channel_claim_signature_preflight(
            true,
            PaymentChannelClaimSignaturePreflightFacts {
                requested_balance_drops: 600,
                ..signature_facts()
            },
            |_| panic!("verifier should not run after bad amount"),
        ),
        Ter::TEM_BAD_AMOUNT
    );
    assert_eq!(
        run_payment_channel_claim_signature_preflight(
            true,
            PaymentChannelClaimSignaturePreflightFacts {
                public_key_type_valid: false,
                ..signature_facts()
            },
            |_| panic!("verifier should not run after bad key type"),
        ),
        Ter::TEM_MALFORMED
    );
    let bad_signature =
        run_payment_channel_claim_signature_preflight(true, signature_facts(), |_| false);
    assert_eq!(bad_signature, Ter::TEM_BAD_SIGNATURE);
    assert_eq!(trans_token(bad_signature), "temBAD_SIGNATURE");
}

#[test]
fn payment_channel_claim_preflight_runs_credentials_helper_after_signature_checks() {
    let mut credentials_called = false;

    let result = run_payment_channel_claim_preflight(
        signature_preflight_facts(),
        |_| true,
        || {
            credentials_called = true;
            Ter::TEC_NO_PERMISSION
        },
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert!(credentials_called);
}

#[test]
fn payment_channel_claim_preflight_returns_credentials_result_after_local_checks() {
    let credentials_called = Cell::new(false);

    let result = run_payment_channel_claim_preflight(
        preflight_facts(),
        |_| unreachable!(),
        || {
            credentials_called.set(true);
            Ter::TEC_NO_PERMISSION
        },
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert!(credentials_called.get());
}

#[test]
fn payment_channel_claim_preclaim_uses_correct_cpp_path() {
    let lower_called = Cell::new(false);
    let credentials_called = Cell::new(false);

    let delegated = run_payment_channel_claim_preclaim(
        false,
        || {
            lower_called.set(true);
            Ter::TER_NO_ACCOUNT
        },
        || {
            credentials_called.set(true);
            Ter::TEC_NO_PERMISSION
        },
    );

    assert_eq!(delegated, Ter::TER_NO_ACCOUNT);
    assert!(lower_called.get());
    assert!(!credentials_called.get());

    let credentials = run_payment_channel_claim_preclaim(
        true,
        || panic!("lower preclaim should not run with credentials enabled"),
        || {
            credentials_called.set(true);
            Ter::TEC_NO_PERMISSION
        },
    );

    assert_eq!(credentials, Ter::TEC_NO_PERMISSION);
}

#[test]
fn payment_channel_claim_do_apply_maps_missing_or_closed_channel() {
    let mut missing_sink = TestApplySink::new();
    let missing = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            channel_exists: false,
            ..apply_facts()
        },
        &mut missing_sink,
    );
    assert_eq!(missing, Ter::TEC_NO_TARGET);
    assert!(missing_sink.events.is_empty());

    let mut closed_sink = TestApplySink::new();
    closed_sink.source_dir_result = Ter::TES_SUCCESS;
    closed_sink.destination_dir_result = Ter::TES_SUCCESS;
    closed_sink.source_account_exists = true;
    let closed = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            due_facts: PaymentChannelDueFacts {
                cancel_after: Some(60),
                expiration: None,
                close_time: 60,
            },
            tx_account_is_source: false,
            tx_account_is_destination: false,
            ..apply_facts()
        },
        &mut closed_sink,
    );
    assert_eq!(closed, Ter::TES_SUCCESS);
    assert_eq!(
        closed_sink.events,
        [
            "remove_source_owner_directory",
            "remove_destination_owner_directory",
            "source_account_exists",
            "apply_refund_to_source_account",
            "adjust_source_owner_count:-1",
            "erase_channel"
        ]
    );
    assert_eq!(closed_sink.refund_drops, Some(750));
    assert_eq!(closed_sink.owner_count_deltas, [-1]);
}

#[test]
fn payment_channel_claim_do_apply_checks_payment_guards() {
    let mut no_signature_sink = TestApplySink::new();
    let no_signature = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            tx_account_is_source: false,
            tx_account_is_destination: true,
            balance_present: true,
            ..apply_facts()
        },
        &mut no_signature_sink,
    );
    assert_eq!(no_signature, Ter::TEM_BAD_SIGNATURE);

    let mut bad_signer_sink = TestApplySink::new();
    let bad_signer = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            balance_present: true,
            signature_present: true,
            provided_public_key_matches_channel: false,
            ..apply_facts()
        },
        &mut bad_signer_sink,
    );
    assert_eq!(bad_signer, Ter::TEM_BAD_SIGNER);
    assert_eq!(trans_token(bad_signer), "temBAD_SIGNER");

    let mut unfunded_sink = TestApplySink::new();
    let unfunded = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            balance_present: true,
            requested_balance_exceeds_channel_funds: true,
            ..apply_facts()
        },
        &mut unfunded_sink,
    );
    assert_eq!(unfunded, Ter::TEC_UNFUNDED_PAYMENT);

    let mut already_paid_sink = TestApplySink::new();
    let already_paid = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            balance_present: true,
            requested_balance_not_above_channel_balance: true,
            ..apply_facts()
        },
        &mut already_paid_sink,
    );
    assert_eq!(already_paid, Ter::TEC_UNFUNDED_PAYMENT);
}

#[test]
fn payment_channel_claim_do_apply_propagates_close_helper_failure() {
    let mut sink = TestApplySink::new();
    sink.source_dir_result = Ter::TEC_NO_DST;

    let result = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            due_facts: PaymentChannelDueFacts {
                cancel_after: Some(60),
                expiration: None,
                close_time: 60,
            },
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(sink.events, ["remove_source_owner_directory"]);
}

#[test]
fn payment_channel_claim_do_apply_returns_destination_or_preauth_errors_before_mutation() {
    let mut no_dst_sink = TestApplySink::new();
    no_dst_sink.destination_exists_result = false;
    let no_dst = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            balance_present: true,
            ..apply_facts()
        },
        &mut no_dst_sink,
    );
    assert_eq!(no_dst, Ter::TEC_NO_DST);
    assert_eq!(no_dst_sink.events, ["destination_exists"]);

    let mut preauth_sink = TestApplySink::new();
    preauth_sink.deposit_preauth_result = Ter::TEC_NO_PERMISSION;
    let preauth = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            balance_present: true,
            ..apply_facts()
        },
        &mut preauth_sink,
    );
    assert_eq!(preauth, Ter::TEC_NO_PERMISSION);
    assert_eq!(
        preauth_sink.events,
        ["destination_exists", "verify_deposit_preauth"]
    );
}

#[test]
fn payment_channel_claim_do_apply_handles_renew_and_close() {
    let mut renew_sink = TestApplySink::new();
    let renew = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            tx_account_is_source: false,
            tx_account_is_destination: true,
            renew_flag: true,
            ..apply_facts()
        },
        &mut renew_sink,
    );
    assert_eq!(renew, Ter::TEC_NO_PERMISSION);
    assert!(renew_sink.events.is_empty());

    let mut close_now_sink = TestApplySink::new();
    let close_now = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            tx_account_is_source: false,
            tx_account_is_destination: true,
            close_flag: true,
            ..apply_facts()
        },
        &mut close_now_sink,
    );
    assert_eq!(close_now, Ter::TES_SUCCESS);
    assert_eq!(
        close_now_sink.events,
        [
            "remove_source_owner_directory",
            "remove_destination_owner_directory",
            "source_account_exists",
            "apply_refund_to_source_account",
            "adjust_source_owner_count:-1",
            "erase_channel"
        ]
    );

    let mut close_later_sink = TestApplySink::new();
    let close_later = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            close_flag: true,
            current_expiration: Some(100),
            ..apply_facts()
        },
        &mut close_later_sink,
    );
    assert_eq!(close_later, Ter::TES_SUCCESS);
    assert_eq!(close_later_sink.events, ["set_expiration"]);
    assert_eq!(close_later_sink.expirations, vec![90]);
}

#[test]
fn payment_channel_claim_do_apply_preserves_success_order() {
    let mut sink = TestApplySink::new();

    let result = run_payment_channel_claim_do_apply(
        PaymentChannelClaimApplyFacts {
            balance_present: true,
            renew_flag: true,
            close_flag: true,
            current_expiration: Some(100),
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "destination_exists",
            "verify_deposit_preauth",
            "set_channel_balance:400",
            "add_destination_balance:150",
            "persist_destination_balance",
            "persist_channel_balance",
            "clear_expiration",
            "set_expiration"
        ]
    );
}

#[test]
fn prepared_payment_channel_claim_do_apply_matches_direct_claim_do_apply() {
    let mut helper_sink = TestApplySink::new();
    let helper_result =
        run_payment_channel_claim_prepared_do_apply(prepared_do_apply_facts(), &mut helper_sink);

    let mut direct_sink = TestApplySink::new();
    let direct_result = run_payment_channel_claim_do_apply(apply_facts(), &mut direct_sink);

    assert_eq!(helper_result, direct_result);
    assert_eq!(helper_sink.events, direct_sink.events);
    assert_eq!(helper_sink.expirations, direct_sink.expirations);
}
