//! Integration tests that pin the narrowed Rust `TrustSet.cpp` shell to current
//! C++ behavior.

use protocol::{
    QUALITY_ONE, Ter, lsfHighDeepFreeze, lsfHighFreeze, tfSetDeepFreeze, tfSetFreeze, tfSetfAuth,
    tfTrustSetMask, trans_token,
};
use tx::trust_set::{
    TrustSetCheckPermissionFacts, TrustSetDoApplyFacts, TrustSetDoApplySink, TrustSetPreclaimFacts,
    TrustSetPreflightEvalFacts, run_trust_set_check_permission, run_trust_set_compute_freeze_flags,
    run_trust_set_do_apply_with_facts, run_trust_set_get_flags_mask,
    run_trust_set_preclaim_with_facts, run_trust_set_preflight_eval,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RecordingSink {
    events: Vec<&'static str>,
}

impl TrustSetDoApplySink for RecordingSink {
    fn update_existing_line(&mut self) -> Ter {
        self.events.push("update");
        Ter::TES_SUCCESS
    }

    fn delete_existing_line(&mut self) -> Ter {
        self.events.push("delete");
        Ter::TES_SUCCESS
    }

    fn create_line(&mut self) -> Ter {
        self.events.push("create");
        Ter::TES_SUCCESS
    }
}

fn preflight_facts() -> TrustSetPreflightEvalFacts {
    TrustSetPreflightEvalFacts {
        tx_flags: 0,
        deep_freeze_enabled: true,
        limit_is_legal_net: true,
        limit_is_native: false,
        limit_currency_is_bad: false,
        limit_is_negative: false,
        issuer_present: true,
    }
}

fn preclaim_facts() -> TrustSetPreclaimFacts {
    TrustSetPreclaimFacts {
        account_exists: true,
        tx_flags: 0,
        account_requires_auth: true,
        destination_is_source: false,
        destination_exists: true,
        amm_or_single_asset_vault_enabled: false,
        destination_account_flags: 0,
        fix_disallow_incoming_v1_enabled: false,
        trustline_exists: true,
        destination_is_pseudo_account: false,
        pseudo_destination_is_amm: false,
        pseudo_destination_is_vault_or_loan_broker: false,
        amm_ledger_entry_exists: false,
        amm_lp_token_balance_non_zero: false,
        amm_lp_token_currency_matches_limit: false,
        deep_freeze_enabled: false,
        account_no_freeze: false,
        high_account_side: true,
        current_trustline_flags: 0,
    }
}

#[test]
fn trust_set_flags_mask_catalog() {
    assert_eq!(run_trust_set_get_flags_mask(), tfTrustSetMask);
}

#[test]
fn trust_set_compute_freeze_flags_freeze_toggle_shape() {
    let set_flags = run_trust_set_compute_freeze_flags(0, true, false, true, false, true, false);
    let clear_flags =
        run_trust_set_compute_freeze_flags(set_flags, true, false, false, true, false, true);

    assert_ne!(set_flags & lsfHighFreeze, 0);
    assert_ne!(set_flags & lsfHighDeepFreeze, 0);
    assert_eq!(clear_flags & lsfHighFreeze, 0);
    assert_eq!(clear_flags & lsfHighDeepFreeze, 0);
}

#[test]
fn trust_set_preflight_rejects_deep_freeze_flags_when_feature_disabled() {
    let result = run_trust_set_preflight_eval(
        TrustSetPreflightEvalFacts {
            tx_flags: tfSetDeepFreeze,
            deep_freeze_enabled: false,
            ..preflight_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

#[test]
fn trust_set_preflight_rejects_bad_limit_shapes() {
    let native = run_trust_set_preflight_eval(
        TrustSetPreflightEvalFacts {
            limit_is_native: true,
            ..preflight_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );
    let bad_currency = run_trust_set_preflight_eval(
        TrustSetPreflightEvalFacts {
            limit_currency_is_bad: true,
            ..preflight_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );
    let negative = run_trust_set_preflight_eval(
        TrustSetPreflightEvalFacts {
            limit_is_negative: true,
            ..preflight_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );
    let no_dst = run_trust_set_preflight_eval(
        TrustSetPreflightEvalFacts {
            issuer_present: false,
            ..preflight_facts()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(native, Ter::TEM_BAD_LIMIT);
    assert_eq!(bad_currency, Ter::TEM_BAD_CURRENCY);
    assert_eq!(negative, Ter::TEM_BAD_LIMIT);
    assert_eq!(no_dst, Ter::TEM_DST_NEEDED);
}

#[test]
fn trust_set_check_permission_follows_delegate_and_granular_rules() {
    let missing_delegate = run_trust_set_check_permission(TrustSetCheckPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: false,
        check_tx_permission_result: Ter::TES_SUCCESS,
        tx_flags: 0,
        quality_in_present: false,
        quality_out_present: false,
        trustline_exists: true,
        granular_trustline_authorize: false,
        granular_trustline_freeze: false,
        granular_trustline_unfreeze: false,
        current_limit_equals_proposed_limit: true,
    });
    let bad_flags = run_trust_set_check_permission(TrustSetCheckPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        check_tx_permission_result: Ter::TEM_INVALID,
        tx_flags: tfSetDeepFreeze,
        quality_in_present: false,
        quality_out_present: false,
        trustline_exists: true,
        granular_trustline_authorize: true,
        granular_trustline_freeze: true,
        granular_trustline_unfreeze: true,
        current_limit_equals_proposed_limit: true,
    });
    let missing_granular = run_trust_set_check_permission(TrustSetCheckPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        check_tx_permission_result: Ter::TEM_INVALID,
        tx_flags: tfSetfAuth,
        quality_in_present: false,
        quality_out_present: false,
        trustline_exists: true,
        granular_trustline_authorize: false,
        granular_trustline_freeze: true,
        granular_trustline_unfreeze: true,
        current_limit_equals_proposed_limit: true,
    });
    let success = run_trust_set_check_permission(TrustSetCheckPermissionFacts {
        delegate_present: true,
        delegate_entry_exists: true,
        check_tx_permission_result: Ter::TEM_INVALID,
        tx_flags: tfSetfAuth | tfSetFreeze,
        quality_in_present: false,
        quality_out_present: false,
        trustline_exists: true,
        granular_trustline_authorize: true,
        granular_trustline_freeze: true,
        granular_trustline_unfreeze: true,
        current_limit_equals_proposed_limit: true,
    });

    assert_eq!(missing_delegate, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(bad_flags, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(missing_granular, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(success, Ter::TES_SUCCESS);
}

#[test]
fn trust_set_preclaim_rejects_no_account_and_bad_auth_gate() {
    let no_account = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        account_exists: false,
        ..preclaim_facts()
    });
    let no_auth_required = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        tx_flags: tfSetfAuth,
        account_requires_auth: false,
        ..preclaim_facts()
    });

    assert_eq!(no_account, Ter::TER_NO_ACCOUNT);
    assert_eq!(no_auth_required, Ter::TEF_NO_AUTH_REQUIRED);
}

#[test]
fn trust_set_preclaim_maps_destination_and_disallow_incoming_rules() {
    let dst_is_src = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        destination_is_source: true,
        ..preclaim_facts()
    });
    let no_dst = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        amm_or_single_asset_vault_enabled: true,
        destination_exists: false,
        ..preclaim_facts()
    });
    let disallow = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        destination_account_flags: protocol::lsfDisallowIncomingTrustline,
        fix_disallow_incoming_v1_enabled: false,
        trustline_exists: false,
        ..preclaim_facts()
    });

    assert_eq!(dst_is_src, Ter::TEM_DST_IS_SRC);
    assert_eq!(no_dst, Ter::TEC_NO_DST);
    assert_eq!(disallow, Ter::TEC_NO_PERMISSION);
}

#[test]
fn trust_set_preclaim_maps_pseudo_account_rules() {
    let amm_internal = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        destination_is_pseudo_account: true,
        pseudo_destination_is_amm: true,
        trustline_exists: false,
        amm_ledger_entry_exists: false,
        ..preclaim_facts()
    });
    let amm_empty = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        destination_is_pseudo_account: true,
        pseudo_destination_is_amm: true,
        trustline_exists: false,
        amm_ledger_entry_exists: true,
        amm_lp_token_balance_non_zero: false,
        ..preclaim_facts()
    });
    let pseudo_other = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        destination_is_pseudo_account: true,
        pseudo_destination_is_amm: false,
        pseudo_destination_is_vault_or_loan_broker: false,
        ..preclaim_facts()
    });

    assert_eq!(amm_internal, Ter::TEC_INTERNAL);
    assert_eq!(amm_empty, Ter::TEC_AMM_EMPTY);
    assert_eq!(pseudo_other, Ter::TEC_PSEUDO_ACCOUNT);
}

#[test]
fn trust_set_preclaim_enforces_deep_freeze_invariants() {
    let no_freeze = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        deep_freeze_enabled: true,
        account_no_freeze: true,
        tx_flags: tfSetFreeze,
        ..preclaim_facts()
    });
    let contradictory = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        deep_freeze_enabled: true,
        tx_flags: tfSetFreeze | protocol::tfClearFreeze,
        ..preclaim_facts()
    });
    let deep_without_freeze = run_trust_set_preclaim_with_facts(TrustSetPreclaimFacts {
        deep_freeze_enabled: true,
        tx_flags: tfSetDeepFreeze,
        high_account_side: true,
        current_trustline_flags: 0,
        ..preclaim_facts()
    });

    assert_eq!(no_freeze, Ter::TEC_NO_PERMISSION);
    assert_eq!(contradictory, Ter::TEC_NO_PERMISSION);
    assert_eq!(deep_without_freeze, Ter::TEC_NO_PERMISSION);
}

#[test]
fn trust_set_do_apply_maps_internal_and_destination_checks() {
    let mut sink = RecordingSink::default();
    let no_src = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            source_account_exists: false,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink,
    );
    let mut sink2 = RecordingSink::default();
    let no_dst = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            destination_account_exists: false,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink2,
    );

    assert_eq!(no_src, Ter::TEF_INTERNAL);
    assert_eq!(no_dst, Ter::TEC_NO_DST);
}

#[test]
fn trust_set_do_apply_existing_line_maps_cpp_paths() {
    let mut sink = RecordingSink::default();
    let no_permission = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            set_no_ripple: true,
            clear_no_ripple: false,
            set_no_ripple_on_negative_balance_attempt: true,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink,
    );
    let mut sink2 = RecordingSink::default();
    let deleted = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            default_after_update: true,
            trust_delete_result: Ter::TEF_BAD_LEDGER,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink2,
    );
    let mut sink3 = RecordingSink::default();
    let insuf_reserve = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            reserve_increase_for_local_side: true,
            local_has_reserve_for_increase: false,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink3,
    );

    assert_eq!(no_permission, Ter::TEC_NO_PERMISSION);
    assert_eq!(deleted, Ter::TEF_BAD_LEDGER);
    assert_eq!(insuf_reserve, Ter::TEC_INSUF_RESERVE_LINE);
    assert_eq!(sink2.events, ["delete"]);
}

#[test]
fn trust_set_do_apply_missing_line_maps_cpp_paths() {
    let mut sink = RecordingSink::default();
    let redundant = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            line_exists: false,
            limit_is_zero: true,
            quality_in_present: false,
            quality_out_present: false,
            set_auth: false,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink,
    );
    let mut sink2 = RecordingSink::default();
    let no_reserve = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            line_exists: false,
            limit_is_zero: false,
            has_reserve_to_create_line: false,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink2,
    );
    let mut sink3 = RecordingSink::default();
    let created = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            line_exists: false,
            limit_is_zero: false,
            trust_create_result: Ter::TEC_NO_AUTH,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink3,
    );

    assert_eq!(redundant, Ter::TEC_NO_LINE_REDUNDANT);
    assert_eq!(no_reserve, Ter::TEC_NO_LINE_INSUF_RESERVE);
    assert_eq!(created, Ter::TEC_NO_AUTH);
    assert_eq!(sink3.events, ["create"]);
}

#[test]
fn trust_set_do_apply_treats_quality_one_as_default_when_line_missing() {
    let mut sink = RecordingSink::default();
    let result = run_trust_set_do_apply_with_facts(
        TrustSetDoApplyFacts {
            line_exists: false,
            limit_is_zero: true,
            quality_in_present: true,
            quality_in: QUALITY_ONE,
            quality_out_present: true,
            quality_out: QUALITY_ONE,
            set_auth: false,
            ..TrustSetDoApplyFacts::success_defaults()
        },
        &mut sink,
    );

    assert_eq!(result, Ter::TEC_NO_LINE_REDUNDANT);
    assert!(sink.events.is_empty());
    assert_eq!(trans_token(result), "tecNO_LINE_REDUNDANT");
}
