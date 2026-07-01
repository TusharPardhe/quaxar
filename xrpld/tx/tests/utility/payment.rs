//! Integration tests that pin the narrowed Rust `Payment.cpp` shell to current
//! C++ behavior.

use protocol::{SeqProxy, Ter, trans_token};
use tx::payment::{
    MAX_PATH_SIZE, PAYMENT_FLAGS_MASK, PAYMENT_MPT_V1_FLAGS_MASK, PaymentCheckPermissionFacts,
    PaymentDoApplyBranch, PaymentDoApplyFacts, PaymentDoApplySink, PaymentPreclaimFacts,
    PaymentPreflightEvalFacts, TF_LIMIT_QUALITY, TF_NO_DIRECT_RIPPLE, TF_PARTIAL_PAYMENT,
    run_payment_calculate_max_xrp_spend, run_payment_check_extra_features,
    run_payment_check_permission, run_payment_do_apply_with_facts, run_payment_get_flags_mask,
    run_payment_make_tx_consequences, run_payment_preclaim_with_facts, run_payment_preflight_eval,
    run_payment_select_do_apply_branch,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RecordingSink {
    events: Vec<&'static str>,
}

impl PaymentDoApplySink for RecordingSink {
    fn touch_destination(&mut self) {
        self.events.push("touch_destination");
    }

    fn apply_ripple_payment(&mut self) {
        self.events.push("apply_ripple");
    }

    fn record_ripple_delivered_amount(&mut self) {
        self.events.push("deliver_ripple");
    }

    fn apply_mpt_payment(&mut self) {
        self.events.push("apply_mpt");
    }

    fn record_mpt_delivered_amount(&mut self) {
        self.events.push("deliver_mpt");
    }

    fn apply_xrp_payment(&mut self) {
        self.events.push("apply_xrp");
    }

    fn clear_xrp_password_spent(&mut self) {
        self.events.push("clear_password_spent");
    }
}

fn preflight_eval_facts() -> PaymentPreflightEvalFacts {
    PaymentPreflightEvalFacts {
        tx_flags: 0,
        mptokens_v1_enabled: true,
        mptokens_v2_enabled: true,
        amount_is_mpt: false,
        paths_present: false,
        send_max_present: false,
        send_max_asset_matches_amount: true,
        send_max_is_mpt: false,
        amount_is_legal_net: true,
        max_source_is_legal_net: true,
        destination_present: true,
        max_source_positive: true,
        amount_positive: true,
        src_asset_bad: false,
        dst_asset_bad: false,
        src_asset_is_xrp: true,
        dst_asset_is_xrp: true,
        account_equals_destination: false,
        src_dst_tokens_equal: false,
        deliver_min_present: false,
        deliver_min_is_legal_net: true,
        deliver_min_is_positive: true,
        deliver_min_asset_matches_amount: true,
        deliver_min_not_greater_than_amount: true,
    }
}

fn preclaim_facts() -> PaymentPreclaimFacts {
    PaymentPreclaimFacts {
        tx_flags: 0,
        has_paths: false,
        send_max_present: false,
        dst_amount_native: true,
        destination_exists: true,
        view_open: false,
        destination_requires_tag: false,
        destination_tag_present: false,
        destination_can_create_with_amount: true,
        path_count: 0,
        path_has_too_long_segment: false,
        credentials_valid_result: Ter::TES_SUCCESS,
        domain_id_present: false,
        source_in_domain: true,
        destination_in_domain: true,
        is_batch_inner: false,
        batch_v1_1_enabled: false,
    }
}

#[test]
fn payment_masks_and_path_limits_match_cpp() {
    assert_eq!(MAX_PATH_SIZE, 6);
    assert_eq!(PAYMENT_FLAGS_MASK, 0x3ff8_ffff);
    assert_eq!(PAYMENT_MPT_V1_FLAGS_MASK, 0x3ffd_ffff);
}

#[test]
fn payment_feature_gates_match_cpp_checks() {
    assert!(run_payment_check_extra_features(false, false, false, false));
    assert!(!run_payment_check_extra_features(true, false, false, false));
    assert!(!run_payment_check_extra_features(false, false, true, false));
}

#[test]
fn payment_flag_mask_switches_for_legacy_mpt() {
    assert_eq!(run_payment_get_flags_mask(false, true), PAYMENT_FLAGS_MASK);
    assert_eq!(run_payment_get_flags_mask(true, true), PAYMENT_FLAGS_MASK);
    assert_eq!(
        run_payment_get_flags_mask(true, false),
        PAYMENT_MPT_V1_FLAGS_MASK
    );
}

#[test]
fn payment_consequences_use_sendmax_then_amount_xrp() {
    let sendmax = run_payment_make_tx_consequences(10, SeqProxy::sequence(7), Some(40), Some(30));
    let amount = run_payment_make_tx_consequences(10, SeqProxy::sequence(7), None, Some(30));
    let none = run_payment_make_tx_consequences(10, SeqProxy::sequence(7), None, None);

    assert_eq!(sendmax.potential_spend(), 40);
    assert_eq!(amount.potential_spend(), 30);
    assert_eq!(none.potential_spend(), 0);
    assert_eq!(run_payment_calculate_max_xrp_spend(Some(9), Some(8)), 9);
}

#[test]
fn payment_preflight_rejects_disabled_mpt_and_bad_xrp_flag_usage() {
    let disabled = run_payment_preflight_eval(
        PaymentPreflightEvalFacts {
            amount_is_mpt: true,
            mptokens_v1_enabled: false,
            ..preflight_eval_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );
    let no_direct = run_payment_preflight_eval(
        PaymentPreflightEvalFacts {
            tx_flags: TF_NO_DIRECT_RIPPLE,
            ..preflight_eval_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(disabled, Ter::TEM_DISABLED);
    assert_eq!(no_direct, Ter::TEM_BAD_SEND_XRP_NO_DIRECT);
}

#[test]
fn payment_preflight_requires_partial_for_deliver_min() {
    let missing_partial = run_payment_preflight_eval(
        PaymentPreflightEvalFacts {
            deliver_min_present: true,
            src_asset_is_xrp: false,
            dst_asset_is_xrp: false,
            ..preflight_eval_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );
    let valid = run_payment_preflight_eval(
        PaymentPreflightEvalFacts {
            tx_flags: TF_PARTIAL_PAYMENT,
            deliver_min_present: true,
            src_asset_is_xrp: false,
            dst_asset_is_xrp: false,
            ..preflight_eval_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(missing_partial, Ter::TEM_BAD_AMOUNT);
    assert_eq!(valid, Ter::TES_SUCCESS);
}

#[test]
fn payment_check_permission_follows_delegate_and_granular_rules() {
    let missing_delegate = run_payment_check_permission(PaymentCheckPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: false,
        check_tx_permission_result: Ter::TES_SUCCESS,
        send_max_present: false,
        send_max_asset_matches_amount: true,
        paths_present: false,
        payment_mint_permission: false,
        payment_burn_permission: false,
        amount_is_xrp: false,
        amount_issuer_is_source: false,
        amount_issuer_is_destination: false,
    });
    let mint = run_payment_check_permission(PaymentCheckPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        check_tx_permission_result: Ter::TEM_INVALID,
        send_max_present: false,
        send_max_asset_matches_amount: true,
        paths_present: false,
        payment_mint_permission: true,
        payment_burn_permission: false,
        amount_is_xrp: false,
        amount_issuer_is_source: true,
        amount_issuer_is_destination: false,
    });
    let denied = run_payment_check_permission(PaymentCheckPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        check_tx_permission_result: Ter::TEM_INVALID,
        send_max_present: true,
        send_max_asset_matches_amount: false,
        paths_present: false,
        payment_mint_permission: true,
        payment_burn_permission: true,
        amount_is_xrp: false,
        amount_issuer_is_source: true,
        amount_issuer_is_destination: true,
    });

    assert_eq!(missing_delegate, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(mint, Ter::TES_SUCCESS);
    assert_eq!(denied, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(trans_token(denied), "terNO_DELEGATE_PERMISSION");
}

#[test]
fn payment_preclaim_maps_destination_and_path_gate_results() {
    let no_dst = run_payment_preclaim_with_facts(PaymentPreclaimFacts {
        destination_exists: false,
        dst_amount_native: false,
        ..preclaim_facts()
    });
    let no_dst_partial = run_payment_preclaim_with_facts(PaymentPreclaimFacts {
        destination_exists: false,
        view_open: true,
        tx_flags: TF_PARTIAL_PAYMENT,
        ..preclaim_facts()
    });
    let path_count = run_payment_preclaim_with_facts(PaymentPreclaimFacts {
        has_paths: true,
        view_open: true,
        path_count: MAX_PATH_SIZE + 1,
        ..preclaim_facts()
    });

    assert_eq!(no_dst, Ter::TEC_NO_DST);
    assert_eq!(no_dst_partial, Ter::TEL_NO_DST_PARTIAL);
    assert_eq!(path_count, Ter::TEL_BAD_PATH_COUNT);
}

#[test]
fn payment_preclaim_maps_credential_and_domain_failures() {
    let credentials = run_payment_preclaim_with_facts(PaymentPreclaimFacts {
        credentials_valid_result: Ter::TEC_FROZEN,
        ..preclaim_facts()
    });
    let domain = run_payment_preclaim_with_facts(PaymentPreclaimFacts {
        domain_id_present: true,
        source_in_domain: true,
        destination_in_domain: false,
        ..preclaim_facts()
    });

    assert_eq!(credentials, Ter::TEC_FROZEN);
    assert_eq!(domain, Ter::TEC_NO_PERMISSION);
}

#[test]
fn payment_do_apply_branch_selection() {
    assert!(matches!(
        run_payment_select_do_apply_branch(true, false, true, false, false),
        PaymentDoApplyBranch::Ripple
    ));
    assert!(matches!(
        run_payment_select_do_apply_branch(false, false, false, true, false),
        PaymentDoApplyBranch::MptDirect
    ));
    assert!(matches!(
        run_payment_select_do_apply_branch(false, false, true, false, false),
        PaymentDoApplyBranch::XrpDirect
    ));
}

#[test]
fn payment_do_apply_ripple_maps_retry_and_partial_delivery() {
    let mut sink = RecordingSink::default();
    let retry = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            has_paths: true,
            dst_amount_native: false,
            ripple_result_is_retry: true,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink,
    );
    let mut sink2 = RecordingSink::default();
    let partial = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            has_paths: true,
            dst_amount_native: false,
            ripple_actual_out_equals_dst: false,
            ripple_actual_out_below_deliver_min: true,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink2,
    );

    assert_eq!(retry, Ter::TEC_PATH_DRY);
    assert_eq!(partial, Ter::TEC_PATH_PARTIAL);
    assert_eq!(sink.events, ["touch_destination", "apply_ripple"]);
    assert_eq!(sink2.events, ["touch_destination", "apply_ripple"]);
}

#[test]
fn payment_do_apply_mpt_maps_auth_frozen_and_path_partial() {
    let mut sink = RecordingSink::default();
    let no_auth = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            dst_amount_native: false,
            dst_amount_is_mpt: true,
            mpt_require_auth_source_result: Ter::TEC_NO_AUTH,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink,
    );
    let mut sink2 = RecordingSink::default();
    let locked = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            dst_amount_native: false,
            dst_amount_is_mpt: true,
            mpt_holder_to_holder: true,
            mpt_any_frozen: true,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink2,
    );
    let mut sink3 = RecordingSink::default();
    let partial = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            dst_amount_native: false,
            dst_amount_is_mpt: true,
            mpt_account_send_result: Ter::TEC_INSUFFICIENT_FUNDS,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink3,
    );

    assert_eq!(no_auth, Ter::TEC_NO_AUTH);
    assert_eq!(locked, Ter::TEC_LOCKED);
    assert_eq!(partial, Ter::TEC_PATH_PARTIAL);
}

#[test]
fn payment_do_apply_xrp_maps_internal_unfunded_and_permission_paths() {
    let mut sink = RecordingSink::default();
    let internal = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            xrp_source_exists: false,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink,
    );
    let mut sink2 = RecordingSink::default();
    let unfunded = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            xrp_has_funds_for_payment_plus_reserve: false,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink2,
    );
    let mut sink3 = RecordingSink::default();
    let denied = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            xrp_destination_is_pseudo: true,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink3,
    );

    assert_eq!(internal, Ter::TEF_INTERNAL);
    assert_eq!(unfunded, Ter::TEC_UNFUNDED_PAYMENT);
    assert_eq!(denied, Ter::TEC_NO_PERMISSION);
}

#[test]
fn payment_do_apply_xrp_success_records_password_clear_step() {
    let mut sink = RecordingSink::default();
    let result = run_payment_do_apply_with_facts(
        PaymentDoApplyFacts {
            xrp_destination_password_spent: true,
            ..PaymentDoApplyFacts::xrp_success_defaults()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        ["touch_destination", "apply_xrp", "clear_password_spent"]
    );
}

#[test]
fn payment_preflight_maps_xrp_limit_quality_rejection() {
    let limit = run_payment_preflight_eval(
        PaymentPreflightEvalFacts {
            tx_flags: TF_LIMIT_QUALITY,
            ..preflight_eval_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(limit, Ter::TEM_BAD_SEND_XRP_LIMIT);
}
