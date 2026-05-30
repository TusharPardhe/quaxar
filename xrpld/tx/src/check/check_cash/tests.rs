use protocol::{Ter, trans_token};

use super::{
    CheckCashApplyFacts, CheckCashApplySink, CheckCashDoApplyBranch, CheckCashDoApplyCleanupPlan,
    CheckCashDoApplyPlan, CheckCashIouFlowCallPlan, CheckCashIouFlowOutcome,
    CheckCashIouFlowRequest, CheckCashIouFlowResult, CheckCashIouLimitOverridePlan,
    CheckCashIouLimitSide, CheckCashIouTrustCreateDestinationState,
    CheckCashIouTrustCreateFieldShape, CheckCashIouTrustCreateHandoffPlan,
    CheckCashIouTrustCreatePlan, CheckCashIouTrustlinePlan, CheckCashPreclaimFacts,
    CheckCashPreflightFacts, run_check_cash_build_iou_flow_deliver,
    run_check_cash_build_iou_flow_request, run_check_cash_compute_xrp_deliver,
    run_check_cash_do_apply, run_check_cash_do_apply_iou_path, run_check_cash_do_apply_xrp_path,
    run_check_cash_finish_iou_flow, run_check_cash_plan_do_apply,
    run_check_cash_plan_iou_flow_call, run_check_cash_plan_iou_limit_override,
    run_check_cash_plan_iou_trust_create, run_check_cash_plan_iou_trust_create_caller,
    run_check_cash_plan_iou_trust_create_destination_state,
    run_check_cash_plan_iou_trust_create_field_shape, run_check_cash_plan_iou_trust_create_handoff,
    run_check_cash_plan_iou_trust_create_post_create_available_check,
    run_check_cash_plan_iou_trustline, run_check_cash_preclaim, run_check_cash_preflight,
    run_check_cash_select_requested_value,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestApplySink {
    xrp_liquid_sufficient: bool,
    transfer_xrp_result: Ter,
    create_iou_trustline_result: Ter,
    trustline_available_after_create: bool,
    prepare_iou_flow_limit_result: Ter,
    run_iou_flow_result: CheckCashIouFlowResult,
    remove_destination_dir: bool,
    remove_owner_dir: bool,
    run_iou_flow_deliver_min_flags: Vec<bool>,
    owner_count_deltas: Vec<i32>,
    events: Vec<String>,
    erased: bool,
    applied: bool,
}

impl TestApplySink {
    fn new() -> Self {
        Self {
            xrp_liquid_sufficient: true,
            transfer_xrp_result: Ter::TES_SUCCESS,
            create_iou_trustline_result: Ter::TES_SUCCESS,
            trustline_available_after_create: true,
            prepare_iou_flow_limit_result: Ter::TES_SUCCESS,
            run_iou_flow_result: CheckCashIouFlowResult {
                ter: Ter::TES_SUCCESS,
                meets_requested_amount: true,
                meets_deliver_min: true,
            },
            remove_destination_dir: true,
            remove_owner_dir: true,
            run_iou_flow_deliver_min_flags: Vec::new(),
            owner_count_deltas: Vec::new(),
            events: Vec::new(),
            erased: false,
            applied: false,
        }
    }
}

impl CheckCashApplySink for TestApplySink {
    fn xrp_liquid_sufficient(&mut self) -> bool {
        self.events.push("xrp_liquid".to_string());
        self.xrp_liquid_sufficient
    }

    fn record_delivered_xrp(&mut self) {
        self.events.push("deliver_xrp".to_string());
    }

    fn transfer_xrp(&mut self) -> Ter {
        self.events.push("transfer_xrp".to_string());
        self.transfer_xrp_result
    }

    fn create_iou_trustline(&mut self) -> Ter {
        self.events.push("create_iou_trustline".to_string());
        self.create_iou_trustline_result
    }

    fn update_destination_after_trustline_create(&mut self) {
        self.events
            .push("update_destination_after_trustline_create".to_string());
    }

    fn trustline_available_after_create(&mut self) -> bool {
        self.events
            .push("trustline_available_after_create".to_string());
        self.trustline_available_after_create
    }

    fn prepare_iou_flow_limit(&mut self) -> Ter {
        self.events.push("prepare_iou_flow_limit".to_string());
        self.prepare_iou_flow_limit_result
    }

    fn run_iou_flow(&mut self, deliver_min_present: bool) -> CheckCashIouFlowResult {
        self.events.push("run_iou_flow".to_string());
        self.run_iou_flow_deliver_min_flags
            .push(deliver_min_present);
        self.run_iou_flow_result
    }

    fn record_delivered_iou(&mut self) {
        self.events.push("deliver_iou".to_string());
    }

    fn reload_check_after_iou_flow(&mut self) {
        self.events.push("reload_check_after_iou_flow".to_string());
    }

    fn restore_iou_flow_limit(&mut self) {
        self.events.push("restore_iou_flow_limit".to_string());
    }

    fn remove_destination_dir(&mut self) -> bool {
        self.events.push("remove_destination".to_string());
        self.remove_destination_dir
    }

    fn remove_owner_dir(&mut self) -> bool {
        self.events.push("remove_owner".to_string());
        self.remove_owner_dir
    }

    fn adjust_owner_count(&mut self, delta: i32) {
        self.events.push(format!("adjust:{delta}"));
        self.owner_count_deltas.push(delta);
    }

    fn erase_check(&mut self) {
        self.events.push("erase".to_string());
        self.erased = true;
    }

    fn apply_view(&mut self) {
        self.events.push("apply".to_string());
        self.applied = true;
    }
}

fn preflight_facts() -> CheckCashPreflightFacts {
    CheckCashPreflightFacts {
        amount_present: true,
        deliver_min_present: false,
        value_is_legal: true,
        value_signum_positive: true,
        value_currency_is_bad: false,
    }
}

fn preclaim_facts() -> CheckCashPreclaimFacts {
    CheckCashPreclaimFacts {
        check_exists: true,
        tx_account_is_check_destination: true,
        check_source_is_destination: false,
        source_account_exists: true,
        destination_account_exists: true,
        destination_require_dest_tag: false,
        check_has_destination_tag: false,
        check_expired: false,
        requested_currency_matches_send_max: true,
        requested_issuer_matches_send_max: true,
        requested_value_exceeds_send_max: false,
        requested_value_exceeds_available_funds: false,
        requested_value_native: false,
        requested_value_issuer_is_destination: false,
        issuer_exists: true,
        issuer_requires_auth: false,
        destination_trustline_exists: true,
        destination_trustline_authorized: true,
        destination_trustline_frozen: false,
    }
}

fn apply_facts() -> CheckCashApplyFacts {
    CheckCashApplyFacts {
        check_exists: true,
        source_account_exists: true,
        destination_account_exists: true,
        source_is_destination: false,
        send_max_is_native: true,
        amount_present: true,
        deliver_min_present: false,
        iou_trustline_exists: true,
        iou_destination_can_pay_trustline_reserve: true,
    }
}

#[test]
fn check_cash_preflight_requires_exactly_one_amount_field() {
    let missing = run_check_cash_preflight(CheckCashPreflightFacts {
        amount_present: false,
        deliver_min_present: false,
        ..preflight_facts()
    });
    let both = run_check_cash_preflight(CheckCashPreflightFacts {
        amount_present: true,
        deliver_min_present: true,
        ..preflight_facts()
    });

    assert_eq!(missing, Ter::TEM_MALFORMED);
    assert_eq!(both, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(missing), "temMALFORMED");
}

#[test]
fn check_cash_preflight_rejects_bad_amount() {
    let result = run_check_cash_preflight(CheckCashPreflightFacts {
        value_is_legal: false,
        ..preflight_facts()
    });

    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
    assert_eq!(trans_token(result), "temBAD_AMOUNT");
}

#[test]
fn check_cash_preflight_rejects_bad_currency() {
    let result = run_check_cash_preflight(CheckCashPreflightFacts {
        value_currency_is_bad: true,
        ..preflight_facts()
    });

    assert_eq!(result, Ter::TEM_BAD_CURRENCY);
    assert_eq!(trans_token(result), "temBAD_CURRENCY");
}

#[test]
fn check_cash_select_requested_value_prefers_amount_then_deliver_min() {
    assert_eq!(
        run_check_cash_select_requested_value(Some(5_u32), Some(7_u32)),
        Some(5)
    );
    assert_eq!(
        run_check_cash_select_requested_value::<u32>(None, Some(7)),
        Some(7)
    );
    assert_eq!(
        run_check_cash_select_requested_value::<u32>(None, None),
        None
    );
}

#[test]
fn check_cash_preclaim_rejects_missing_check() {
    let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
        check_exists: false,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn check_cash_preclaim_rejects_wrong_destination() {
    let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
        tx_account_is_check_destination: false,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
}

#[test]
fn check_cash_preclaim_maps_self_check_to_internal() {
    let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
        check_source_is_destination: true,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
}

#[test]
fn check_cash_preclaim_rejects_missing_source_or_destination() {
    let missing_source = run_check_cash_preclaim(CheckCashPreclaimFacts {
        source_account_exists: false,
        ..preclaim_facts()
    });
    let missing_destination = run_check_cash_preclaim(CheckCashPreclaimFacts {
        destination_account_exists: false,
        ..preclaim_facts()
    });

    assert_eq!(missing_source, Ter::TEC_NO_ENTRY);
    assert_eq!(missing_destination, Ter::TEC_NO_ENTRY);
}

#[test]
fn check_cash_preclaim_rejects_required_destination_tag() {
    let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
        destination_require_dest_tag: true,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
    assert_eq!(trans_token(result), "tecDST_TAG_NEEDED");
}

#[test]
fn check_cash_preclaim_rejects_expired_check() {
    let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
        check_expired: true,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_EXPIRED);
    assert_eq!(trans_token(result), "tecEXPIRED");
}

#[test]
fn check_cash_preclaim_rejects_currency_or_issuer_mismatch() {
    let currency = run_check_cash_preclaim(CheckCashPreclaimFacts {
        requested_currency_matches_send_max: false,
        ..preclaim_facts()
    });
    let issuer = run_check_cash_preclaim(CheckCashPreclaimFacts {
        requested_issuer_matches_send_max: false,
        ..preclaim_facts()
    });

    assert_eq!(currency, Ter::TEM_MALFORMED);
    assert_eq!(issuer, Ter::TEM_MALFORMED);
}

#[test]
fn check_cash_preclaim_rejects_value_above_sendmax_or_funds() {
    let send_max = run_check_cash_preclaim(CheckCashPreclaimFacts {
        requested_value_exceeds_send_max: true,
        ..preclaim_facts()
    });
    let funds = run_check_cash_preclaim(CheckCashPreclaimFacts {
        requested_value_exceeds_available_funds: true,
        ..preclaim_facts()
    });

    assert_eq!(send_max, Ter::TEC_PATH_PARTIAL);
    assert_eq!(funds, Ter::TEC_PATH_PARTIAL);
    assert_eq!(trans_token(send_max), "tecPATH_PARTIAL");
}

#[test]
fn check_cash_preclaim_rejects_missing_or_unauthorized_issuer_line() {
    let no_issuer = run_check_cash_preclaim(CheckCashPreclaimFacts {
        issuer_exists: false,
        ..preclaim_facts()
    });
    let no_line = run_check_cash_preclaim(CheckCashPreclaimFacts {
        issuer_requires_auth: true,
        destination_trustline_exists: false,
        ..preclaim_facts()
    });
    let unauthorized = run_check_cash_preclaim(CheckCashPreclaimFacts {
        issuer_requires_auth: true,
        destination_trustline_authorized: false,
        ..preclaim_facts()
    });

    assert_eq!(no_issuer, Ter::TEC_NO_ISSUER);
    assert_eq!(no_line, Ter::TEC_NO_AUTH);
    assert_eq!(unauthorized, Ter::TEC_NO_AUTH);
}

#[test]
fn check_cash_preclaim_rejects_frozen_destination_line() {
    let result = run_check_cash_preclaim(CheckCashPreclaimFacts {
        destination_trustline_frozen: true,
        ..preclaim_facts()
    });

    assert_eq!(result, Ter::TEC_FROZEN);
    assert_eq!(trans_token(result), "tecFROZEN");
}

#[test]
fn check_cash_compute_xrp_deliver_clamps_with_deliver_min_formula() {
    assert_eq!(
        run_check_cash_compute_xrp_deliver(80_u64, 120, 90, false),
        80
    );
    assert_eq!(
        run_check_cash_compute_xrp_deliver(80_u64, 120, 90, true),
        90
    );
    assert_eq!(run_check_cash_compute_xrp_deliver(80_u64, 70, 60, true), 80);
}

#[test]
fn check_cash_plan_iou_trustline_maps_cpp_truster_and_limit_side() {
    let create_plan = run_check_cash_plan_iou_trustline(30_u32, 10, 20, false, true).unwrap();
    let issuer_is_destination =
        run_check_cash_plan_iou_trustline(20_u32, 10, 20, true, true).unwrap();

    assert_eq!(
        create_plan,
        CheckCashIouTrustlinePlan {
            truster: 20,
            needs_create: true,
            tweaked_limit_side: CheckCashIouLimitSide::Low,
        }
    );
    assert_eq!(
        issuer_is_destination,
        CheckCashIouTrustlinePlan {
            truster: 10,
            needs_create: false,
            tweaked_limit_side: CheckCashIouLimitSide::High,
        }
    );
}

#[test]
fn check_cash_plan_iou_trustline_rejects_missing_reserve() {
    let result = run_check_cash_plan_iou_trustline(30_u32, 10, 20, false, false);

    assert_eq!(result, Err(Ter::TEC_NO_LINE_INSUF_RESERVE));
}

#[test]
fn check_cash_plan_iou_trust_create_reports_cpp_reserve_gate() {
    let create =
        run_check_cash_plan_iou_trust_create(30_u32, 20_u32, false, true, false, -1_i32, 0_i32)
            .unwrap();
    let existing =
        run_check_cash_plan_iou_trust_create(20_u32, 20_u32, true, false, true, -1_i32, 0_i32)
            .unwrap();
    let missing_reserve =
        run_check_cash_plan_iou_trust_create(30_u32, 20_u32, false, false, false, -1_i32, 0_i32);

    assert_eq!(
        create,
        CheckCashIouTrustCreatePlan {
            needs_create: true,
            can_create: true,
            dest_low: true,
            authorize_account: false,
            no_ripple: true,
            freeze: false,
            deep_freeze: false,
            initial_balance: -1,
            limit: 0,
            quality_in: 0,
            quality_out: 0,
        }
    );
    assert_eq!(
        existing,
        CheckCashIouTrustCreatePlan {
            needs_create: false,
            can_create: false,
            dest_low: false,
            authorize_account: false,
            no_ripple: false,
            freeze: false,
            deep_freeze: false,
            initial_balance: -1,
            limit: 0,
            quality_in: 0,
            quality_out: 0,
        }
    );
    assert_eq!(missing_reserve, Err(Ter::TEC_NO_LINE_INSUF_RESERVE));
}

#[test]
fn check_cash_plan_iou_trust_create_handoff_selects_truster_and_carries_identity() {
    let plan = run_check_cash_plan_iou_trust_create_handoff(
        30_u32, 10_u32, 20_u32, 77_u32, 88_u32, false, true, false, -1_i32, 0_i32,
    )
    .unwrap();

    assert_eq!(
        plan,
        CheckCashIouTrustCreateHandoffPlan {
            truster: 20,
            trustline_key: 77,
            trustline_index: 88,
            destination_update_after_create: true,
            needs_create: true,
            can_create: true,
            dest_low: true,
            authorize_account: false,
            no_ripple: true,
            freeze: false,
            deep_freeze: false,
            initial_balance: -1,
            limit: 0,
            quality_in: 0,
            quality_out: 0,
        }
    );
}

#[test]
fn check_cash_plan_iou_trust_create_handoff_uses_source_when_issuer_matches_destination() {
    let plan = run_check_cash_plan_iou_trust_create_handoff(
        20_u32, 10_u32, 20_u32, 99_u32, 100_u32, true, false, true, -1_i32, 0_i32,
    )
    .unwrap();

    assert_eq!(
        plan,
        CheckCashIouTrustCreateHandoffPlan {
            truster: 10,
            trustline_key: 99,
            trustline_index: 100,
            destination_update_after_create: false,
            needs_create: false,
            can_create: false,
            dest_low: false,
            authorize_account: false,
            no_ripple: false,
            freeze: false,
            deep_freeze: false,
            initial_balance: -1,
            limit: 0,
            quality_in: 0,
            quality_out: 0,
        }
    );
}

#[test]
fn check_cash_plan_iou_trust_create_handoff_rejects_missing_reserve() {
    let result = run_check_cash_plan_iou_trust_create_handoff(
        30_u32, 10_u32, 20_u32, 77_u32, 88_u32, false, false, false, -1_i32, 0_i32,
    );

    assert_eq!(result, Err(Ter::TEC_NO_LINE_INSUF_RESERVE));
}

#[test]
fn check_cash_plan_iou_trust_create_caller_packages_destination_state_and_callback() {
    let plan = run_check_cash_plan_iou_trust_create_caller(
        30_u32, 10_u32, 20_u32, 77_u32, 88_u32, false, true, false, 4, -1_i32, 0_i32,
    )
    .unwrap();

    assert_eq!(
        plan.destination_state,
        CheckCashIouTrustCreateDestinationState {
            owner_count: 4,
            default_ripple_enabled: false,
        }
    );
    assert_eq!(
        plan.trust_create,
        CheckCashIouTrustCreateHandoffPlan {
            truster: 20,
            trustline_key: 77,
            trustline_index: 88,
            destination_update_after_create: true,
            needs_create: true,
            can_create: true,
            dest_low: true,
            authorize_account: false,
            no_ripple: true,
            freeze: false,
            deep_freeze: false,
            initial_balance: -1,
            limit: 0,
            quality_in: 0,
            quality_out: 0,
        }
    );
}

#[test]
fn check_cash_plan_iou_trust_create_field_shape() {
    let low = run_check_cash_plan_iou_trust_create_field_shape(true);
    let high = run_check_cash_plan_iou_trust_create_field_shape(false);

    assert_eq!(
        low,
        CheckCashIouTrustCreateFieldShape {
            dest_low: true,
            initial_balance_is_zero: true,
            destination_limit_is_zero: true,
        }
    );
    assert_eq!(
        high,
        CheckCashIouTrustCreateFieldShape {
            dest_low: false,
            initial_balance_is_zero: true,
            destination_limit_is_zero: true,
        }
    );
}

#[test]
fn check_cash_plan_iou_trust_create_destination_state_keeps_account_snapshot() {
    assert_eq!(
        run_check_cash_plan_iou_trust_create_destination_state(9, true),
        CheckCashIouTrustCreateDestinationState {
            owner_count: 9,
            default_ripple_enabled: true,
        }
    );
}

#[test]
fn check_cash_plan_iou_trust_create_post_create_check() {
    assert!(run_check_cash_plan_iou_trust_create_post_create_available_check(false));
    assert!(!run_check_cash_plan_iou_trust_create_post_create_available_check(true));
}

#[test]
fn check_cash_plan_do_apply_selects_branch_and_cleanup() {
    let self_cash = run_check_cash_plan_do_apply(true, true);
    let xrp = run_check_cash_plan_do_apply(false, true);
    let iou = run_check_cash_plan_do_apply(false, false);

    assert_eq!(
        self_cash,
        CheckCashDoApplyPlan {
            branch: CheckCashDoApplyBranch::SelfCash,
            cleanup: CheckCashDoApplyCleanupPlan {
                remove_destination_dir: false,
                remove_owner_dir: true,
                adjust_owner_count_delta: -1,
                erase_check: true,
                apply_view: true,
            },
        }
    );
    assert_eq!(
        xrp,
        CheckCashDoApplyPlan {
            branch: CheckCashDoApplyBranch::Xrp,
            cleanup: CheckCashDoApplyCleanupPlan {
                remove_destination_dir: true,
                remove_owner_dir: true,
                adjust_owner_count_delta: -1,
                erase_check: true,
                apply_view: true,
            },
        }
    );
    assert_eq!(
        iou,
        CheckCashDoApplyPlan {
            branch: CheckCashDoApplyBranch::Iou,
            cleanup: CheckCashDoApplyCleanupPlan {
                remove_destination_dir: true,
                remove_owner_dir: true,
                adjust_owner_count_delta: -1,
                erase_check: true,
                apply_view: true,
            },
        }
    );
}

#[test]
fn check_cash_build_iou_flow_deliver_uses_deliver_min_limit_only_when_needed() {
    let mut invoked = false;
    let direct = run_check_cash_build_iou_flow_deliver(55_u64, false, |_| {
        invoked = true;
        999
    });
    assert_eq!(direct, 55);
    assert!(!invoked);

    let limited = run_check_cash_build_iou_flow_deliver(55_u64, true, |value| value + 500);
    assert_eq!(limited, 555);
}

#[test]
fn check_cash_plan_iou_limit_override_uses_cpp_limit_side_rule() {
    let low = run_check_cash_plan_iou_limit_override(30_u32, 20_u32, 999_u64);
    let high = run_check_cash_plan_iou_limit_override(20_u32, 20_u32, 999_u64);

    assert_eq!(
        low,
        CheckCashIouLimitOverridePlan {
            tweaked_limit_side: CheckCashIouLimitSide::Low,
            temporary_limit: 999,
        }
    );
    assert_eq!(
        high,
        CheckCashIouLimitOverridePlan {
            tweaked_limit_side: CheckCashIouLimitSide::High,
            temporary_limit: 999,
        }
    );
}

#[test]
fn check_cash_build_iou_flow_request_flags() {
    let request = run_check_cash_build_iou_flow_request(55_u64, true, |value| value + 500);

    assert_eq!(
        request,
        CheckCashIouFlowRequest {
            deliver: 555,
            partial_payment: true,
            default_path: true,
            owner_pays_transfer_fee: true,
        }
    );
}

#[test]
fn check_cash_plan_iou_flow_call_packages_request_and_limit_override() {
    let plan = run_check_cash_plan_iou_flow_call(30_u32, 20_u32, 55_u64, true, 999_u64, |value| {
        value + 500
    });

    assert_eq!(
        plan,
        CheckCashIouFlowCallPlan {
            request: CheckCashIouFlowRequest {
                deliver: 555,
                partial_payment: true,
                default_path: true,
                owner_pays_transfer_fee: true,
            },
            limit_override: CheckCashIouLimitOverridePlan {
                tweaked_limit_side: CheckCashIouLimitSide::Low,
                temporary_limit: 999,
            },
        }
    );
}

#[test]
fn check_cash_finish_iou_flow_maps_cpp_result_rules() {
    let failure = run_check_cash_finish_iou_flow(
        CheckCashIouFlowResult {
            ter: Ter::TER_NO_RIPPLE,
            meets_requested_amount: true,
            meets_deliver_min: false,
        },
        false,
        true,
    );
    let partial = run_check_cash_finish_iou_flow(
        CheckCashIouFlowResult {
            ter: Ter::TES_SUCCESS,
            meets_requested_amount: true,
            meets_deliver_min: false,
        },
        false,
        true,
    );
    let exact_amount_partial = run_check_cash_finish_iou_flow(
        CheckCashIouFlowResult {
            ter: Ter::TES_SUCCESS,
            meets_requested_amount: false,
            meets_deliver_min: true,
        },
        true,
        false,
    );
    let success = run_check_cash_finish_iou_flow(
        CheckCashIouFlowResult {
            ter: Ter::TES_SUCCESS,
            meets_requested_amount: true,
            meets_deliver_min: true,
        },
        false,
        false,
    );

    assert_eq!(
        failure,
        CheckCashIouFlowOutcome {
            ter: Ter::TER_NO_RIPPLE,
            record_delivered_amount: false,
            reload_check_after_flow: false,
        }
    );
    assert_eq!(
        partial,
        CheckCashIouFlowOutcome {
            ter: Ter::TEC_PATH_PARTIAL,
            record_delivered_amount: false,
            reload_check_after_flow: false,
        }
    );
    assert_eq!(
        exact_amount_partial,
        CheckCashIouFlowOutcome {
            ter: Ter::TEC_PATH_PARTIAL,
            record_delivered_amount: false,
            reload_check_after_flow: false,
        }
    );
    assert_eq!(
        success,
        CheckCashIouFlowOutcome {
            ter: Ter::TES_SUCCESS,
            record_delivered_amount: true,
            reload_check_after_flow: true,
        }
    );
}

#[test]
fn check_cash_preclaim_skips_issuer_checks_for_native_or_issuer_destination() {
    let native = run_check_cash_preclaim(CheckCashPreclaimFacts {
        requested_value_native: true,
        issuer_exists: false,
        ..preclaim_facts()
    });
    let issuer_is_destination = run_check_cash_preclaim(CheckCashPreclaimFacts {
        requested_value_issuer_is_destination: true,
        issuer_exists: false,
        ..preclaim_facts()
    });

    assert_eq!(native, Ter::TES_SUCCESS);
    assert_eq!(issuer_is_destination, Ter::TES_SUCCESS);
}

#[test]
fn check_cash_do_apply_xrp_path_maps_unfunded_payment() {
    let mut sink = TestApplySink::new();
    sink.xrp_liquid_sufficient = false;

    let result = run_check_cash_do_apply_xrp_path(false, &mut sink);

    assert_eq!(result, Ter::TEC_UNFUNDED_PAYMENT);
    assert_eq!(trans_token(result), "tecUNFUNDED_PAYMENT");
    assert_eq!(sink.events, ["xrp_liquid"]);
}

#[test]
fn check_cash_do_apply_xrp_path_records_deliver_before_transfer() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply_xrp_path(true, &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["xrp_liquid", "deliver_xrp", "transfer_xrp"]);
}

#[test]
fn check_cash_do_apply_xrp_path_skips_deliver_for_amount_branch() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply_xrp_path(false, &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["xrp_liquid", "transfer_xrp"]);
}

#[test]
fn check_cash_do_apply_xrp_path_returns_transfer_failure_unchanged() {
    let mut sink = TestApplySink::new();
    sink.transfer_xrp_result = Ter::TEC_INSUFFICIENT_RESERVE;

    let result = run_check_cash_do_apply_xrp_path(true, &mut sink);

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(sink.events, ["xrp_liquid", "deliver_xrp", "transfer_xrp"]);
}

#[test]
fn check_cash_do_apply_iou_path_rejects_insufficient_reserve() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply_iou_path(false, false, true, false, &mut sink);

    assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
    assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
    assert!(sink.events.is_empty());
}

#[test]
fn check_cash_do_apply_iou_path_returns_trust_create_failure_unchanged() {
    let mut sink = TestApplySink::new();
    sink.create_iou_trustline_result = Ter::TER_NO_RIPPLE;

    let result = run_check_cash_do_apply_iou_path(false, true, true, false, &mut sink);

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(sink.events, ["create_iou_trustline"]);
}

#[test]
fn check_cash_do_apply_iou_path_returns_prepare_failure_unchanged() {
    let mut sink = TestApplySink::new();
    sink.prepare_iou_flow_limit_result = Ter::TEC_NO_LINE;

    let result = run_check_cash_do_apply_iou_path(true, true, true, false, &mut sink);

    assert_eq!(result, Ter::TEC_NO_LINE);
    assert_eq!(trans_token(result), "tecNO_LINE");
    assert_eq!(sink.events, ["prepare_iou_flow_limit"]);
}

#[test]
fn check_cash_do_apply_iou_path_maps_missing_trustline_after_create() {
    let mut sink = TestApplySink::new();
    sink.trustline_available_after_create = false;

    let result = run_check_cash_do_apply_iou_path(false, true, true, false, &mut sink);

    assert_eq!(result, Ter::TEC_NO_LINE);
    assert_eq!(
        sink.events,
        [
            "create_iou_trustline",
            "update_destination_after_trustline_create",
            "trustline_available_after_create"
        ]
    );
}

#[test]
fn check_cash_do_apply_iou_path_restores_limit_on_flow_failure() {
    let mut sink = TestApplySink::new();
    sink.run_iou_flow_result = CheckCashIouFlowResult {
        ter: Ter::TER_NO_RIPPLE,
        meets_requested_amount: true,
        meets_deliver_min: false,
    };

    let result = run_check_cash_do_apply_iou_path(true, true, false, true, &mut sink);

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(
        sink.events,
        [
            "prepare_iou_flow_limit",
            "run_iou_flow",
            "restore_iou_flow_limit"
        ]
    );
}

#[test]
fn check_cash_do_apply_iou_path_maps_unsatisfied_deliver_min() {
    let mut sink = TestApplySink::new();
    sink.run_iou_flow_result = CheckCashIouFlowResult {
        ter: Ter::TES_SUCCESS,
        meets_requested_amount: true,
        meets_deliver_min: false,
    };

    let result = run_check_cash_do_apply_iou_path(true, true, false, true, &mut sink);

    assert_eq!(result, Ter::TEC_PATH_PARTIAL);
    assert_eq!(
        sink.events,
        [
            "prepare_iou_flow_limit",
            "run_iou_flow",
            "restore_iou_flow_limit"
        ]
    );
}

#[test]
fn check_cash_do_apply_iou_path_runs_success_path_in() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply_iou_path(false, true, false, true, &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.run_iou_flow_deliver_min_flags, vec![true]);
    assert_eq!(
        sink.events,
        [
            "create_iou_trustline",
            "update_destination_after_trustline_create",
            "trustline_available_after_create",
            "prepare_iou_flow_limit",
            "run_iou_flow",
            "deliver_iou",
            "reload_check_after_iou_flow",
            "restore_iou_flow_limit"
        ]
    );
}

#[test]
fn check_cash_do_apply_iou_path_records_delivery_and_reload_without_deliver_min() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply_iou_path(false, true, true, false, &mut sink);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.run_iou_flow_deliver_min_flags, vec![false]);
    assert_eq!(
        sink.events,
        [
            "create_iou_trustline",
            "update_destination_after_trustline_create",
            "trustline_available_after_create",
            "prepare_iou_flow_limit",
            "run_iou_flow",
            "deliver_iou",
            "reload_check_after_iou_flow",
            "restore_iou_flow_limit"
        ]
    );
}

#[test]
fn check_cash_do_apply_maps_missing_check_to_failed_processing() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply(
        CheckCashApplyFacts {
            check_exists: false,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_FAILED_PROCESSING);
    assert_eq!(trans_token(result), "tecFAILED_PROCESSING");
    assert!(sink.events.is_empty());
}

#[test]
fn check_cash_do_apply_maps_missing_accounts_to_failed_processing() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply(
        CheckCashApplyFacts {
            source_account_exists: false,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_FAILED_PROCESSING);
    assert!(sink.events.is_empty());
}

#[test]
fn check_cash_do_apply_returns_xrp_failure_before_cleanup() {
    let mut sink = TestApplySink::new();
    sink.transfer_xrp_result = Ter::TEC_INSUFFICIENT_RESERVE;

    let result = run_check_cash_do_apply(
        CheckCashApplyFacts {
            deliver_min_present: true,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(sink.events, ["xrp_liquid", "deliver_xrp", "transfer_xrp"]);
}

#[test]
fn check_cash_do_apply_delegates_iou_branch_with_deliver_min_flag() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply(
        CheckCashApplyFacts {
            send_max_is_native: false,
            deliver_min_present: true,
            iou_trustline_exists: false,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.run_iou_flow_deliver_min_flags, vec![true]);
    assert_eq!(
        sink.events,
        [
            "create_iou_trustline",
            "update_destination_after_trustline_create",
            "trustline_available_after_create",
            "prepare_iou_flow_limit",
            "run_iou_flow",
            "deliver_iou",
            "reload_check_after_iou_flow",
            "restore_iou_flow_limit",
            "remove_destination",
            "remove_owner",
            "adjust:-1",
            "erase",
            "apply"
        ]
    );
}

#[test]
fn check_cash_do_apply_skips_transfer_and_destination_remove_for_self_cash() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply(
        CheckCashApplyFacts {
            source_is_destination: true,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(sink.events, ["remove_owner", "adjust:-1", "erase", "apply"]);
}

#[test]
fn check_cash_do_apply_maps_destination_remove_failure() {
    let mut sink = TestApplySink::new();
    sink.remove_destination_dir = false;

    let result = run_check_cash_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(
        sink.events,
        ["xrp_liquid", "transfer_xrp", "remove_destination"]
    );
}

#[test]
fn check_cash_do_apply_maps_owner_remove_failure() {
    let mut sink = TestApplySink::new();
    sink.remove_owner_dir = false;

    let result = run_check_cash_do_apply(apply_facts(), &mut sink);

    assert_eq!(result, Ter::TEF_BAD_LEDGER);
    assert_eq!(
        sink.events,
        [
            "xrp_liquid",
            "transfer_xrp",
            "remove_destination",
            "remove_owner"
        ]
    );
}

#[test]
fn check_cash_do_apply_runs_success_path_in() {
    let mut sink = TestApplySink::new();

    let result = run_check_cash_do_apply(
        CheckCashApplyFacts {
            deliver_min_present: true,
            ..apply_facts()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        sink.events,
        [
            "xrp_liquid",
            "deliver_xrp",
            "transfer_xrp",
            "remove_destination",
            "remove_owner",
            "adjust:-1",
            "erase",
            "apply"
        ]
    );
    assert_eq!(sink.owner_count_deltas, vec![-1]);
    assert!(sink.erased);
    assert!(sink.applied);
}
