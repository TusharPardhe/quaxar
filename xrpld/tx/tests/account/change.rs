//! Integration tests that pin the narrowed Rust `Change.cpp` front shell to
//! the current C++ behavior.

use std::{
    cell::Cell,
    panic::{AssertUnwindSafe, catch_unwind},
};

use basics::base_uint::Uint256;
use protocol::{
    DecodedAmendmentsEntry, DecodedDisabledValidator, DecodedMajorityEntry,
    DecodedNegativeUnlEntry, Ter, TxType, trans_token,
};
use tx::change::{CHANGE_PRECOMPUTE_ASSERT_MESSAGE, run_change_precompute};
use tx::{
    ChangeAmendmentFacts, ChangePreclaimFacts, ChangePreflightFacts, ChangeUnlModifyFacts,
    run_change_apply_amendment, run_change_apply_unl_modify, run_change_do_apply,
    run_change_preclaim, run_change_preflight, run_change_preflight_flag_mask,
};

fn test_validator(tag: u8) -> Vec<u8> {
    vec![tag; 33]
}

fn test_amendment(tag: u8) -> Uint256 {
    Uint256::from_array([tag; 32])
}

#[test]
fn change_preflight_flag_mask_follows_lending_protocol_gate() {
    assert_eq!(run_change_preflight_flag_mask(false), 0);
    assert_ne!(run_change_preflight_flag_mask(true), 0);
}

#[test]
fn change_preflight_rejects_bad_source_fee_signature_and_sequence() {
    let bad_source = run_change_preflight(
        Ter::TES_SUCCESS,
        ChangePreflightFacts {
            account_is_zero: false,
            ..ChangePreflightFacts::default()
        },
    );
    let bad_fee = run_change_preflight(
        Ter::TES_SUCCESS,
        ChangePreflightFacts {
            account_is_zero: true,
            fee_is_native_and_zero: false,
            ..ChangePreflightFacts::default()
        },
    );
    let bad_signature = run_change_preflight(
        Ter::TES_SUCCESS,
        ChangePreflightFacts {
            account_is_zero: true,
            fee_is_native_and_zero: true,
            signing_pub_key_empty: true,
            signature_empty: false,
            ..ChangePreflightFacts::default()
        },
    );
    let bad_sequence = run_change_preflight(
        Ter::TES_SUCCESS,
        ChangePreflightFacts {
            account_is_zero: true,
            fee_is_native_and_zero: true,
            signing_pub_key_empty: true,
            signature_empty: true,
            sequence_is_zero: false,
            ..ChangePreflightFacts::default()
        },
    );

    assert_eq!(bad_source, Ter::TEM_BAD_SRC_ACCOUNT);
    assert_eq!(bad_fee, Ter::TEM_BAD_FEE);
    assert_eq!(bad_signature, Ter::TEM_BAD_SIGNATURE);
    assert_eq!(bad_sequence, Ter::TEM_BAD_SEQUENCE);
}

#[test]
fn change_preflight_accepts_zero_fee_unsigned_pseudo_tx() {
    let result = run_change_preflight(
        Ter::TES_SUCCESS,
        ChangePreflightFacts {
            account_is_zero: true,
            fee_is_native_and_zero: true,
            signing_pub_key_empty: true,
            signature_empty: true,
            sequence_is_zero: true,
            ..ChangePreflightFacts::default()
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn change_preclaim_validates_fee_field_shapes() {
    let xrp_missing = run_change_preclaim(
        TxType::FEE,
        ChangePreclaimFacts {
            xrp_fees_enabled: true,
            base_fee_drops_present: true,
            reserve_base_drops_present: true,
            ..ChangePreclaimFacts::default()
        },
    );
    let xrp_forbids_legacy = run_change_preclaim(
        TxType::FEE,
        ChangePreclaimFacts {
            xrp_fees_enabled: true,
            base_fee_drops_present: true,
            reserve_base_drops_present: true,
            reserve_increment_drops_present: true,
            base_fee_present: true,
            ..ChangePreclaimFacts::default()
        },
    );
    let legacy_missing = run_change_preclaim(
        TxType::FEE,
        ChangePreclaimFacts {
            base_fee_present: true,
            reference_fee_units_present: true,
            reserve_base_present: true,
            ..ChangePreclaimFacts::default()
        },
    );
    let legacy_forbids_xrp = run_change_preclaim(
        TxType::FEE,
        ChangePreclaimFacts {
            base_fee_present: true,
            reference_fee_units_present: true,
            reserve_base_present: true,
            reserve_increment_present: true,
            base_fee_drops_present: true,
            ..ChangePreclaimFacts::default()
        },
    );

    assert_eq!(xrp_missing, Ter::TEM_MALFORMED);
    assert_eq!(xrp_forbids_legacy, Ter::TEM_MALFORMED);
    assert_eq!(legacy_missing, Ter::TEM_MALFORMED);
    assert_eq!(legacy_forbids_xrp, Ter::TEM_DISABLED);
}

#[test]
fn change_preclaim_accepts_known_tx_types_and_rejects_unknown() {
    assert_eq!(
        run_change_preclaim(TxType::AMENDMENT, ChangePreclaimFacts::default()),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        run_change_preclaim(TxType::UNL_MODIFY, ChangePreclaimFacts::default()),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        run_change_preclaim(TxType::PAYMENT, ChangePreclaimFacts::default()),
        Ter::TEM_UNKNOWN
    );
}

#[test]
fn change_do_apply_dispatches_known_types_and_rejects_unknown() {
    assert_eq!(
        run_change_do_apply(
            TxType::AMENDMENT,
            || Ter::TES_SUCCESS,
            || Ter::TEM_INVALID,
            || Ter::TEM_INVALID,
        ),
        Ter::TES_SUCCESS
    );
    assert_eq!(
        run_change_do_apply(
            TxType::PAYMENT,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        ),
        Ter::TEF_FAILURE
    );
}

#[test]
fn change_precompute_asserts_non_zero_account_and_runs_parent_only_on_zero() {
    let parent_precompute_called = Cell::new(false);

    let panic = catch_unwind(AssertUnwindSafe(|| {
        run_change_precompute(|| parent_precompute_called.set(true), false);
    }))
    .expect_err("non-zero change account should assert");

    let message = if let Some(message) = panic.downcast_ref::<String>() {
        message.as_str()
    } else if let Some(message) = panic.downcast_ref::<&'static str>() {
        message
    } else {
        panic!("unexpected panic payload");
    };

    assert!(!parent_precompute_called.get());
    assert!(message.contains(CHANGE_PRECOMPUTE_ASSERT_MESSAGE));

    run_change_precompute(|| parent_precompute_called.set(true), true);

    assert!(parent_precompute_called.get());
}

#[test]
fn change_apply_amendment_rejects_enabled_duplicate_before_flag_checks() {
    let amendment = test_amendment(0x10);
    let result = run_change_apply_amendment(
        &ChangeAmendmentFacts {
            amendment,
            got_majority: true,
            lost_majority: true,
            parent_close_time: 99,
            amendment_supported: true,
        },
        Some(&DecodedAmendmentsEntry {
            amendments: vec![test_amendment(0x10)],
            majorities: vec![],
            ..DecodedAmendmentsEntry::default()
        }),
    );

    assert_eq!(result.result, Ter::TEF_ALREADY);
    assert!(result.amendments_entry.is_none());
}

#[test]
fn change_apply_amendment_rejects_conflicting_flags_and_majority_mismatches() {
    let amendment = test_amendment(0x11);
    let invalid_flags = run_change_apply_amendment(
        &ChangeAmendmentFacts {
            amendment,
            got_majority: true,
            lost_majority: true,
            parent_close_time: 100,
            amendment_supported: true,
        },
        Some(&DecodedAmendmentsEntry::default()),
    );
    let got_majority_duplicate = run_change_apply_amendment(
        &ChangeAmendmentFacts {
            amendment,
            got_majority: true,
            lost_majority: false,
            parent_close_time: 101,
            amendment_supported: true,
        },
        Some(&DecodedAmendmentsEntry {
            majorities: vec![DecodedMajorityEntry {
                amendment,
                close_time: 7,
            }],
            ..DecodedAmendmentsEntry::default()
        }),
    );
    let lost_majority_missing = run_change_apply_amendment(
        &ChangeAmendmentFacts {
            amendment,
            got_majority: false,
            lost_majority: true,
            parent_close_time: 102,
            amendment_supported: true,
        },
        Some(&DecodedAmendmentsEntry::default()),
    );

    assert_eq!(invalid_flags.result, Ter::TEM_INVALID_FLAG);
    assert_eq!(got_majority_duplicate.result, Ter::TEF_ALREADY);
    assert_eq!(lost_majority_missing.result, Ter::TEF_ALREADY);
}

#[test]
fn change_apply_amendment_enables_amendment_and_preserves_other_majorities() {
    let amendment = test_amendment(0x12);
    let other_majority = test_amendment(0x13);
    let outcome = run_change_apply_amendment(
        &ChangeAmendmentFacts {
            amendment,
            got_majority: false,
            lost_majority: false,
            parent_close_time: 103,
            amendment_supported: false,
        },
        Some(&DecodedAmendmentsEntry {
            majorities: vec![DecodedMajorityEntry {
                amendment: other_majority,
                close_time: 88,
            }],
            ..DecodedAmendmentsEntry::default()
        }),
    );

    assert_eq!(outcome.result, Ter::TES_SUCCESS);
    assert!(outcome.amendment_blocked);
    assert!(!outcome.unsupported_majority_warning);
    assert_eq!(
        outcome.amendments_entry,
        Some(DecodedAmendmentsEntry {
            amendments: vec![amendment],
            majorities: vec![DecodedMajorityEntry {
                amendment: other_majority,
                close_time: 88,
            }],
            ..DecodedAmendmentsEntry::default()
        })
    );
}

#[test]
fn change_apply_amendment_records_majority_and_preserves_other_entries() {
    let amendment = test_amendment(0x14);
    let other_majority = test_amendment(0x15);
    let outcome = run_change_apply_amendment(
        &ChangeAmendmentFacts {
            amendment,
            got_majority: true,
            lost_majority: false,
            parent_close_time: 105,
            amendment_supported: false,
        },
        Some(&DecodedAmendmentsEntry {
            majorities: vec![DecodedMajorityEntry {
                amendment: other_majority,
                close_time: 9,
            }],
            ..DecodedAmendmentsEntry::default()
        }),
    );

    assert_eq!(outcome.result, Ter::TES_SUCCESS);
    assert!(!outcome.amendment_blocked);
    assert!(outcome.unsupported_majority_warning);
    assert_eq!(
        outcome.amendments_entry,
        Some(DecodedAmendmentsEntry {
            majorities: vec![
                DecodedMajorityEntry {
                    amendment: other_majority,
                    close_time: 9,
                },
                DecodedMajorityEntry {
                    amendment,
                    close_time: 105,
                },
            ],
            ..DecodedAmendmentsEntry::default()
        })
    );
}

#[test]
fn change_apply_unl_modify_rejects_non_flag_wrong_format_and_bad_key() {
    let not_flag = run_change_apply_unl_modify(
        &ChangeUnlModifyFacts {
            is_flag_ledger: false,
            ..ChangeUnlModifyFacts::default()
        },
        None,
    );
    let wrong_format = run_change_apply_unl_modify(
        &ChangeUnlModifyFacts {
            is_flag_ledger: true,
            unl_modify_disabling: Some(2),
            ledger_sequence: Some(10),
            current_ledger_sequence: 10,
            validator_public_key: Some(test_validator(0x11)),
            validator_public_key_type_known: true,
        },
        None,
    );
    let bad_key = run_change_apply_unl_modify(
        &ChangeUnlModifyFacts {
            is_flag_ledger: true,
            unl_modify_disabling: Some(1),
            ledger_sequence: Some(10),
            current_ledger_sequence: 10,
            validator_public_key: Some(test_validator(0x22)),
            validator_public_key_type_known: false,
        },
        None,
    );

    assert_eq!(not_flag.result, Ter::TEF_FAILURE);
    assert_eq!(wrong_format.result, Ter::TEF_FAILURE);
    assert_eq!(bad_key.result, Ter::TEF_FAILURE);
}

#[test]
fn change_apply_unl_modify_disable_and_reenable_paths_match_cpp() {
    let validator = test_validator(0x33);
    let disable = run_change_apply_unl_modify(
        &ChangeUnlModifyFacts {
            is_flag_ledger: true,
            unl_modify_disabling: Some(1),
            ledger_sequence: Some(20),
            current_ledger_sequence: 20,
            validator_public_key: Some(validator.clone()),
            validator_public_key_type_known: true,
        },
        Some(&DecodedNegativeUnlEntry {
            disabled_validators: vec![DecodedDisabledValidator {
                public_key: test_validator(0x44),
                first_ledger_sequence: 7,
            }],
            ..DecodedNegativeUnlEntry::default()
        }),
    );
    let reenable = run_change_apply_unl_modify(
        &ChangeUnlModifyFacts {
            is_flag_ledger: true,
            unl_modify_disabling: Some(0),
            ledger_sequence: Some(21),
            current_ledger_sequence: 21,
            validator_public_key: Some(validator.clone()),
            validator_public_key_type_known: true,
        },
        Some(&DecodedNegativeUnlEntry {
            disabled_validators: vec![DecodedDisabledValidator {
                public_key: validator.clone(),
                first_ledger_sequence: 9,
            }],
            ..DecodedNegativeUnlEntry::default()
        }),
    );

    assert_eq!(disable.result, Ter::TES_SUCCESS);
    assert_eq!(
        disable
            .negative_unl_entry
            .as_ref()
            .and_then(|entry| entry.validator_to_disable.as_ref()),
        Some(&validator)
    );
    assert_eq!(reenable.result, Ter::TES_SUCCESS);
    assert_eq!(
        reenable
            .negative_unl_entry
            .as_ref()
            .and_then(|entry| entry.validator_to_re_enable.as_ref()),
        Some(&validator)
    );
}
