//! Current front the reference implementation shell.
//!
//! This ports the deterministic behavior around:
//!
//! - the LendingProtocol-gated `tfEnableAmendmentMask` selection,
//! - zero-account, zero-fee, signature-free, and zero-sequence pseudo-tx
//!   preflight checks,
//! - the open-ledger preclaim guard,
//! - `ttFEE` legacy-versus-XRPFees field-shape validation,
//! - `ttAMENDMENT` / `ttFEE` / `ttUNL_MODIFY` acceptance,
//! - the top-level `doApply()` transaction-type dispatch,
//! - and the deterministic `applyUNLModify()` validation and mutation path.

use basics::base_uint::Uint256;
use protocol::{
    DecodedAmendmentsEntry, DecodedMajorityEntry, DecodedNegativeUnlEntry,
    ENABLE_AMENDMENT_FLAGS_MASK, NotTec, Ter, TxType, is_tes_success,
};

pub const CHANGE_PRECOMPUTE_ASSERT_MESSAGE: &str = "xrpl::Change::preCompute : zero account";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChangePreflightFacts {
    pub account_is_zero: bool,
    pub fee_is_native_and_zero: bool,
    pub signing_pub_key_empty: bool,
    pub signature_empty: bool,
    pub signers_present: bool,
    pub sequence_is_zero: bool,
    pub previous_txn_id_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChangePreclaimFacts {
    pub ledger_is_open: bool,
    pub xrp_fees_enabled: bool,
    pub base_fee_present: bool,
    pub reference_fee_units_present: bool,
    pub reserve_base_present: bool,
    pub reserve_increment_present: bool,
    pub base_fee_drops_present: bool,
    pub reserve_base_drops_present: bool,
    pub reserve_increment_drops_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeAmendmentFacts {
    pub amendment: Uint256,
    pub got_majority: bool,
    pub lost_majority: bool,
    pub parent_close_time: u32,
    pub amendment_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeAmendmentOutcome {
    pub result: Ter,
    pub amendments_entry: Option<DecodedAmendmentsEntry>,
    pub amendment_blocked: bool,
    pub unsupported_majority_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeUnlModifyFacts {
    pub is_flag_ledger: bool,
    pub unl_modify_disabling: Option<u8>,
    pub ledger_sequence: Option<u32>,
    pub current_ledger_sequence: u32,
    pub validator_public_key: Option<Vec<u8>>,
    pub validator_public_key_type_known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeUnlModifyOutcome {
    pub result: Ter,
    pub negative_unl_entry: Option<DecodedNegativeUnlEntry>,
}

pub const fn run_change_preflight_flag_mask(lending_protocol_enabled: bool) -> u32 {
    if lending_protocol_enabled {
        ENABLE_AMENDMENT_FLAGS_MASK
    } else {
        0
    }
}

pub fn run_change_preflight(lower_preflight_result: NotTec, facts: ChangePreflightFacts) -> NotTec {
    if !is_tes_success(lower_preflight_result) {
        return lower_preflight_result;
    }

    if !facts.account_is_zero {
        return Ter::TEM_BAD_SRC_ACCOUNT;
    }

    if !facts.fee_is_native_and_zero {
        return Ter::TEM_BAD_FEE;
    }

    if !facts.signing_pub_key_empty || !facts.signature_empty || facts.signers_present {
        return Ter::TEM_BAD_SIGNATURE;
    }

    if !facts.sequence_is_zero || facts.previous_txn_id_present {
        return Ter::TEM_BAD_SEQUENCE;
    }

    Ter::TES_SUCCESS
}

pub const fn run_change_preclaim(txn_type: TxType, facts: ChangePreclaimFacts) -> Ter {
    if facts.ledger_is_open {
        return Ter::TEM_INVALID;
    }

    match txn_type {
        TxType::FEE => {
            if facts.xrp_fees_enabled {
                if !facts.base_fee_drops_present
                    || !facts.reserve_base_drops_present
                    || !facts.reserve_increment_drops_present
                {
                    return Ter::TEM_MALFORMED;
                }
                if facts.base_fee_present
                    || facts.reference_fee_units_present
                    || facts.reserve_base_present
                    || facts.reserve_increment_present
                {
                    return Ter::TEM_MALFORMED;
                }
            } else {
                if !facts.base_fee_present
                    || !facts.reference_fee_units_present
                    || !facts.reserve_base_present
                    || !facts.reserve_increment_present
                {
                    return Ter::TEM_MALFORMED;
                }
                if facts.base_fee_drops_present
                    || facts.reserve_base_drops_present
                    || facts.reserve_increment_drops_present
                {
                    return Ter::TEM_DISABLED;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::AMENDMENT | TxType::UNL_MODIFY => Ter::TES_SUCCESS,
        _ => Ter::TEM_UNKNOWN,
    }
}

pub fn run_change_do_apply<ApplyAmendment, ApplyFee, ApplyUnlModify>(
    txn_type: TxType,
    run_apply_amendment: ApplyAmendment,
    run_apply_fee: ApplyFee,
    run_apply_unl_modify: ApplyUnlModify,
) -> Ter
where
    ApplyAmendment: FnOnce() -> Ter,
    ApplyFee: FnOnce() -> Ter,
    ApplyUnlModify: FnOnce() -> Ter,
{
    match txn_type {
        TxType::AMENDMENT => run_apply_amendment(),
        TxType::FEE => run_apply_fee(),
        TxType::UNL_MODIFY => run_apply_unl_modify(),
        _ => Ter::TEF_FAILURE,
    }
}

pub fn run_change_precompute(run_transactor_precompute: impl FnOnce(), account_is_zero: bool) {
    assert!(account_is_zero, "{CHANGE_PRECOMPUTE_ASSERT_MESSAGE}");
    run_transactor_precompute();
}

fn change_amendment_failure(result: Ter) -> ChangeAmendmentOutcome {
    ChangeAmendmentOutcome {
        result,
        amendments_entry: None,
        amendment_blocked: false,
        unsupported_majority_warning: false,
    }
}

pub fn run_change_apply_amendment(
    facts: &ChangeAmendmentFacts,
    amendments_entry: Option<&DecodedAmendmentsEntry>,
) -> ChangeAmendmentOutcome {
    let mut amendment_object = amendments_entry.cloned().unwrap_or_default();

    if amendment_object.amendments.contains(&facts.amendment) {
        return change_amendment_failure(Ter::TEF_ALREADY);
    }

    if facts.got_majority && facts.lost_majority {
        return change_amendment_failure(Ter::TEM_INVALID_FLAG);
    }

    let mut new_majorities = Vec::with_capacity(amendment_object.majorities.len());
    let mut found = false;

    for majority in amendment_object.majorities {
        if majority.amendment == facts.amendment {
            if facts.got_majority {
                return change_amendment_failure(Ter::TEF_ALREADY);
            }
            found = true;
        } else {
            new_majorities.push(majority);
        }
    }

    if !found && facts.lost_majority {
        return change_amendment_failure(Ter::TEF_ALREADY);
    }

    let mut amendment_blocked = false;
    let unsupported_majority_warning = facts.got_majority && !facts.amendment_supported;

    if facts.got_majority {
        new_majorities.push(DecodedMajorityEntry {
            close_time: facts.parent_close_time,
            amendment: facts.amendment,
        });
    } else if !facts.lost_majority {
        amendment_object.amendments.push(facts.amendment);
        if !facts.amendment_supported {
            amendment_blocked = true;
        }
    }

    amendment_object.majorities = new_majorities;

    ChangeAmendmentOutcome {
        result: Ter::TES_SUCCESS,
        amendments_entry: Some(amendment_object),
        amendment_blocked,
        unsupported_majority_warning,
    }
}

fn change_unl_modify_failure() -> ChangeUnlModifyOutcome {
    ChangeUnlModifyOutcome {
        result: Ter::TEF_FAILURE,
        negative_unl_entry: None,
    }
}

pub fn run_change_apply_unl_modify(
    facts: &ChangeUnlModifyFacts,
    negative_unl_entry: Option<&DecodedNegativeUnlEntry>,
) -> ChangeUnlModifyOutcome {
    if !facts.is_flag_ledger {
        return change_unl_modify_failure();
    }

    let Some(unl_modify_disabling) = facts.unl_modify_disabling else {
        return change_unl_modify_failure();
    };
    if unl_modify_disabling > 1
        || facts.ledger_sequence.is_none()
        || facts.validator_public_key.is_none()
    {
        return change_unl_modify_failure();
    }

    let disabling = unl_modify_disabling != 0;
    let seq = facts
        .ledger_sequence
        .expect("ledger sequence presence already checked");
    if seq != facts.current_ledger_sequence {
        return change_unl_modify_failure();
    }

    if !facts.validator_public_key_type_known {
        return change_unl_modify_failure();
    }

    let validator = facts
        .validator_public_key
        .as_ref()
        .expect("validator presence already checked");
    let mut negative_unl = negative_unl_entry.cloned().unwrap_or_default();
    let found = negative_unl
        .disabled_validators
        .iter()
        .any(|entry| entry.public_key == *validator);

    if disabling {
        if negative_unl.validator_to_disable.is_some() {
            return change_unl_modify_failure();
        }

        if negative_unl
            .validator_to_re_enable
            .as_ref()
            .is_some_and(|current| current == validator)
        {
            return change_unl_modify_failure();
        }

        if found {
            return change_unl_modify_failure();
        }

        negative_unl.validator_to_disable = Some(validator.clone());
    } else {
        if negative_unl.validator_to_re_enable.is_some() {
            return change_unl_modify_failure();
        }

        if negative_unl
            .validator_to_disable
            .as_ref()
            .is_some_and(|current| current == validator)
        {
            return change_unl_modify_failure();
        }

        if !found {
            return change_unl_modify_failure();
        }

        negative_unl.validator_to_re_enable = Some(validator.clone());
    }

    ChangeUnlModifyOutcome {
        result: Ter::TES_SUCCESS,
        negative_unl_entry: Some(negative_unl),
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::{cell::Cell, rc::Rc};

    use basics::base_uint::Uint256;
    use protocol::DecodedMajorityEntry;
    use protocol::{
        DecodedAmendmentsEntry, DecodedDisabledValidator, DecodedNegativeUnlEntry, Ter, TxType,
        trans_token,
    };

    use super::{
        CHANGE_PRECOMPUTE_ASSERT_MESSAGE, ChangeAmendmentFacts, ChangePreclaimFacts,
        ChangePreflightFacts, ChangeUnlModifyFacts, run_change_apply_amendment,
        run_change_apply_unl_modify, run_change_do_apply, run_change_preclaim,
        run_change_precompute, run_change_preflight, run_change_preflight_flag_mask,
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
    fn change_preflight_returns_first_lower_failure() {
        let result = run_change_preflight(Ter::TEM_INVALID_FLAG, ChangePreflightFacts::default());

        assert_eq!(result, Ter::TEM_INVALID_FLAG);
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
    fn change_preclaim_rejects_open_ledger() {
        let result = run_change_preclaim(
            TxType::AMENDMENT,
            ChangePreclaimFacts {
                ledger_is_open: true,
                ..ChangePreclaimFacts::default()
            },
        );

        assert_eq!(result, Ter::TEM_INVALID);
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
        let xrp_valid = run_change_preclaim(
            TxType::FEE,
            ChangePreclaimFacts {
                xrp_fees_enabled: true,
                base_fee_drops_present: true,
                reserve_base_drops_present: true,
                reserve_increment_drops_present: true,
                ..ChangePreclaimFacts::default()
            },
        );
        let legacy_valid = run_change_preclaim(
            TxType::FEE,
            ChangePreclaimFacts {
                base_fee_present: true,
                reference_fee_units_present: true,
                reserve_base_present: true,
                reserve_increment_present: true,
                ..ChangePreclaimFacts::default()
            },
        );

        assert_eq!(xrp_missing, Ter::TEM_MALFORMED);
        assert_eq!(xrp_forbids_legacy, Ter::TEM_MALFORMED);
        assert_eq!(legacy_missing, Ter::TEM_MALFORMED);
        assert_eq!(legacy_forbids_xrp, Ter::TEM_DISABLED);
        assert_eq!(xrp_valid, Ter::TES_SUCCESS);
        assert_eq!(legacy_valid, Ter::TES_SUCCESS);
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
    fn change_do_apply_dispatches_by_tx_type() {
        let calls = Rc::new(Cell::new(0_u8));
        let amendment_calls = Rc::clone(&calls);
        let fee_calls = Rc::clone(&calls);
        let unl_calls = Rc::clone(&calls);

        let amendment = run_change_do_apply(
            TxType::AMENDMENT,
            move || {
                amendment_calls.set(amendment_calls.get() + 1);
                Ter::TES_SUCCESS
            },
            move || {
                fee_calls.set(fee_calls.get() + 10);
                Ter::TEM_INVALID
            },
            move || {
                unl_calls.set(unl_calls.get() + 100);
                Ter::TEM_INVALID
            },
        );

        assert_eq!(amendment, Ter::TES_SUCCESS);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn change_do_apply_returns_failure_for_unknown_tx_type() {
        let result = run_change_do_apply(
            TxType::PAYMENT,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEF_FAILURE);
    }

    #[test]
    fn change_precompute_asserts_zero_account_before_parent_precompute() {
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
    }

    #[test]
    fn change_precompute_runs_parent_precompute_for_zero_account() {
        let parent_precompute_called = Cell::new(false);

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
    fn change_apply_amendment_rejects_matching_majority_on_got_majority() {
        let amendment = test_amendment(0x14);
        let other_majority = test_amendment(0x15);
        let outcome = run_change_apply_amendment(
            &ChangeAmendmentFacts {
                amendment,
                got_majority: true,
                lost_majority: false,
                parent_close_time: 104,
                amendment_supported: false,
            },
            Some(&DecodedAmendmentsEntry {
                majorities: vec![
                    DecodedMajorityEntry {
                        amendment,
                        close_time: 7,
                    },
                    DecodedMajorityEntry {
                        amendment: other_majority,
                        close_time: 8,
                    },
                ],
                ..DecodedAmendmentsEntry::default()
            }),
        );

        assert_eq!(outcome.result, Ter::TEF_ALREADY);
        assert!(outcome.amendments_entry.is_none());
    }

    #[test]
    fn change_apply_amendment_records_majority_and_preserves_other_entries() {
        let amendment = test_amendment(0x16);
        let other_majority = test_amendment(0x17);
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
    fn change_apply_unl_modify_rejects_non_flag_ledger_wrong_format_and_seq_mismatch() {
        let validator = test_validator(0x11);

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
                validator_public_key: Some(validator.clone()),
                validator_public_key_type_known: true,
            },
            None,
        );
        let wrong_seq = run_change_apply_unl_modify(
            &ChangeUnlModifyFacts {
                is_flag_ledger: true,
                unl_modify_disabling: Some(1),
                ledger_sequence: Some(9),
                current_ledger_sequence: 10,
                validator_public_key: Some(validator),
                validator_public_key_type_known: true,
            },
            None,
        );

        assert_eq!(not_flag.result, Ter::TEF_FAILURE);
        assert_eq!(wrong_format.result, Ter::TEF_FAILURE);
        assert_eq!(wrong_seq.result, Ter::TEF_FAILURE);
        assert_eq!(not_flag.negative_unl_entry, None);
        assert_eq!(wrong_format.negative_unl_entry, None);
        assert_eq!(wrong_seq.negative_unl_entry, None);
    }

    #[test]
    fn change_apply_unl_modify_rejects_unknown_validator_key() {
        let result = run_change_apply_unl_modify(
            &ChangeUnlModifyFacts {
                is_flag_ledger: true,
                unl_modify_disabling: Some(1),
                ledger_sequence: Some(25),
                current_ledger_sequence: 25,
                validator_public_key: Some(test_validator(0x22)),
                validator_public_key_type_known: false,
            },
            None,
        );

        assert_eq!(result.result, Ter::TEF_FAILURE);
        assert_eq!(result.negative_unl_entry, None);
    }

    #[test]
    fn change_apply_unl_modify_disable_path_failure_order_and_mutation() {
        let validator = test_validator(0x33);
        let existing_disabled = test_validator(0x44);
        let to_re_enable = test_validator(0x55);
        let facts = ChangeUnlModifyFacts {
            is_flag_ledger: true,
            unl_modify_disabling: Some(1),
            ledger_sequence: Some(30),
            current_ledger_sequence: 30,
            validator_public_key: Some(validator.clone()),
            validator_public_key_type_known: true,
        };

        let already_has_to_disable = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                validator_to_disable: Some(existing_disabled.clone()),
                ..DecodedNegativeUnlEntry::default()
            }),
        );
        let same_as_to_re_enable = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                validator_to_re_enable: Some(validator.clone()),
                ..DecodedNegativeUnlEntry::default()
            }),
        );
        let already_disabled = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                disabled_validators: vec![DecodedDisabledValidator {
                    public_key: validator.clone(),
                    first_ledger_sequence: 9,
                }],
                validator_to_re_enable: Some(to_re_enable),
                ..DecodedNegativeUnlEntry::default()
            }),
        );
        let success = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                disabled_validators: vec![DecodedDisabledValidator {
                    public_key: existing_disabled.clone(),
                    first_ledger_sequence: 7,
                }],
                ..DecodedNegativeUnlEntry::default()
            }),
        );

        assert_eq!(already_has_to_disable.result, Ter::TEF_FAILURE);
        assert_eq!(same_as_to_re_enable.result, Ter::TEF_FAILURE);
        assert_eq!(already_disabled.result, Ter::TEF_FAILURE);
        assert_eq!(success.result, Ter::TES_SUCCESS);
        assert_eq!(
            success.negative_unl_entry,
            Some(DecodedNegativeUnlEntry {
                disabled_validators: vec![DecodedDisabledValidator {
                    public_key: existing_disabled,
                    first_ledger_sequence: 7,
                }],
                validator_to_disable: Some(validator),
                ..DecodedNegativeUnlEntry::default()
            })
        );
    }

    #[test]
    fn change_apply_unl_modify_reenable_path_failure_order_and_mutation() {
        let validator = test_validator(0x66);
        let to_disable = test_validator(0x77);
        let facts = ChangeUnlModifyFacts {
            is_flag_ledger: true,
            unl_modify_disabling: Some(0),
            ledger_sequence: Some(40),
            current_ledger_sequence: 40,
            validator_public_key: Some(validator.clone()),
            validator_public_key_type_known: true,
        };

        let already_has_to_re_enable = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                validator_to_re_enable: Some(test_validator(0x88)),
                ..DecodedNegativeUnlEntry::default()
            }),
        );
        let same_as_to_disable = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                validator_to_disable: Some(validator.clone()),
                ..DecodedNegativeUnlEntry::default()
            }),
        );
        let not_disabled = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                validator_to_disable: Some(to_disable.clone()),
                ..DecodedNegativeUnlEntry::default()
            }),
        );
        let success = run_change_apply_unl_modify(
            &facts,
            Some(&DecodedNegativeUnlEntry {
                disabled_validators: vec![DecodedDisabledValidator {
                    public_key: validator.clone(),
                    first_ledger_sequence: 12,
                }],
                validator_to_disable: Some(to_disable),
                ..DecodedNegativeUnlEntry::default()
            }),
        );

        assert_eq!(already_has_to_re_enable.result, Ter::TEF_FAILURE);
        assert_eq!(same_as_to_disable.result, Ter::TEF_FAILURE);
        assert_eq!(not_disabled.result, Ter::TEF_FAILURE);
        assert_eq!(success.result, Ter::TES_SUCCESS);
        assert_eq!(
            success.negative_unl_entry,
            Some(DecodedNegativeUnlEntry {
                disabled_validators: vec![DecodedDisabledValidator {
                    public_key: validator.clone(),
                    first_ledger_sequence: 12,
                }],
                validator_to_disable: Some(test_validator(0x77)),
                validator_to_re_enable: Some(validator),
                ..DecodedNegativeUnlEntry::default()
            })
        );
    }

    #[test]
    fn change_apply_unl_modify_creates_negative_unl_entry_on_first_disable() {
        let validator = test_validator(0x99);
        let result = run_change_apply_unl_modify(
            &ChangeUnlModifyFacts {
                is_flag_ledger: true,
                unl_modify_disabling: Some(1),
                ledger_sequence: Some(50),
                current_ledger_sequence: 50,
                validator_public_key: Some(validator.clone()),
                validator_public_key_type_known: true,
            },
            None,
        );

        assert_eq!(result.result, Ter::TES_SUCCESS);
        assert_eq!(
            result.negative_unl_entry,
            Some(DecodedNegativeUnlEntry {
                validator_to_disable: Some(validator),
                ..DecodedNegativeUnlEntry::default()
            })
        );
    }
}
