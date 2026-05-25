//! Integration tests that pin the higher `Change.cpp` owner wrapper to the
//! current C++ branch composition and result shaping.

use protocol::{DecodedAmendmentsEntry, DecodedDisabledValidator, DecodedNegativeUnlEntry, Ter};
use tx::change_base_fee::ChangeApplyFeeTxFields;
use tx::{
    ChangeAmendmentFacts, ChangeOwnerDoApplyCarrier, ChangeOwnerDoApplyOutcome,
    ChangeUnlModifyFacts, run_change_owner_do_apply,
};

fn validator(tag: u8) -> Vec<u8> {
    vec![tag; 33]
}

fn amendment(tag: u8) -> basics::base_uint::Uint256 {
    basics::base_uint::Uint256::from_array([tag; 32])
}

#[test]
fn change_owner_amendment_branch_dispatches_and_shapes() {
    let carrier = ChangeOwnerDoApplyCarrier::amendment(
        ChangeAmendmentFacts {
            amendment: amendment(0x51),
            got_majority: false,
            lost_majority: false,
            parent_close_time: 77,
            amendment_supported: false,
        },
        Some(DecodedAmendmentsEntry::default()),
    );

    assert_eq!(carrier.txn_type(), protocol::TxType::AMENDMENT);
    match run_change_owner_do_apply(carrier) {
        ChangeOwnerDoApplyOutcome::Amendment(outcome) => {
            assert_eq!(outcome.result, Ter::TES_SUCCESS);
            assert!(outcome.amendment_blocked);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[test]
fn change_owner_fee_branch_composes_field_plan_and_mutation_plan() {
    let carrier = ChangeOwnerDoApplyCarrier::fee(
        false,
        Some(protocol::DecodedFeeSettingsEntry {
            base_fee_drops: Some(protocol::DecodedAmountField {
                drops: 1,
                native: true,
                negative: false,
            }),
            reserve_base_drops: Some(protocol::DecodedAmountField {
                drops: 2,
                native: true,
                negative: false,
            }),
            reserve_increment_drops: Some(protocol::DecodedAmountField {
                drops: 3,
                native: true,
                negative: false,
            }),
            ..protocol::DecodedFeeSettingsEntry::default()
        }),
        ChangeApplyFeeTxFields::Legacy {
            base_fee: 10,
            reference_fee_units: 11,
            reserve_base: 12,
            reserve_increment: 13,
        },
    );

    match run_change_owner_do_apply(carrier) {
        ChangeOwnerDoApplyOutcome::Fee(outcome) => {
            assert!(outcome.field_plan.fields_to_clear.is_empty());
            assert_eq!(outcome.mutation_plan.entry.base_fee, Some(10));
            assert_eq!(outcome.mutation_plan.entry.base_fee_drops.unwrap().drops, 1);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[test]
fn change_owner_unl_modify_branch_shapes() {
    let validator = validator(0x61);
    let carrier = ChangeOwnerDoApplyCarrier::unl_modify(
        ChangeUnlModifyFacts {
            is_flag_ledger: true,
            unl_modify_disabling: Some(0),
            ledger_sequence: Some(44),
            current_ledger_sequence: 44,
            validator_public_key: Some(validator.clone()),
            validator_public_key_type_known: true,
        },
        Some(DecodedNegativeUnlEntry {
            disabled_validators: vec![DecodedDisabledValidator {
                public_key: validator.clone(),
                first_ledger_sequence: 5,
            }],
            ..DecodedNegativeUnlEntry::default()
        }),
    );

    match run_change_owner_do_apply(carrier) {
        ChangeOwnerDoApplyOutcome::UnlModify(outcome) => {
            assert_eq!(outcome.result, Ter::TES_SUCCESS);
            assert_eq!(
                outcome
                    .negative_unl_entry
                    .as_ref()
                    .and_then(|entry| entry.validator_to_re_enable.as_ref()),
                Some(&validator)
            );
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}
