//! Owner/carrier wrapper for the current the reference implementation seam.
//!
//! This composes the existing `change.rs` and `change_base_fee.rs` helpers
//! into a typed top-level carrier:
//!
//! - `ttAMENDMENT` dispatches into the amendment helper,
//! - `ttFEE` dispatches into the fee-field planning and fee-mutation helpers,
//! - `ttUNL_MODIFY` dispatches into the N-UNL helper,
//! - and the returned outcome stays shaped per branch instead of collapsing to
//!   a single untyped `TER`.

use protocol::{
    DecodedAmendmentsEntry, DecodedFeeSettingsEntry, DecodedNegativeUnlEntry, Ter, TxType,
};

use crate::{
    ChangeAmendmentFacts, ChangeAmendmentOutcome, ChangeUnlModifyFacts, ChangeUnlModifyOutcome,
    change::run_change_apply_amendment,
    change::run_change_apply_unl_modify,
    change_base_fee::{
        ChangeApplyFeeFieldPlan, ChangeApplyFeeMutationPlan, ChangeApplyFeeTxFields,
        run_change_apply_fee_field_plan, run_change_apply_fee_mutation_plan,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeOwnerFeeOutcome {
    pub field_plan: ChangeApplyFeeFieldPlan,
    pub mutation_plan: ChangeApplyFeeMutationPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChangeOwnerDoApplyBranch {
    Amendment {
        facts: ChangeAmendmentFacts,
        amendments_entry: Option<DecodedAmendmentsEntry>,
    },
    Fee {
        feature_xrp_fees_enabled: bool,
        existing_entry: Option<DecodedFeeSettingsEntry>,
        tx_fields: ChangeApplyFeeTxFields,
    },
    UnlModify {
        facts: ChangeUnlModifyFacts,
        negative_unl_entry: Option<DecodedNegativeUnlEntry>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeOwnerDoApplyCarrier {
    txn_type: TxType,
    branch: ChangeOwnerDoApplyBranch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeOwnerDoApplyOutcome {
    Amendment(ChangeAmendmentOutcome),
    Fee(ChangeOwnerFeeOutcome),
    UnlModify(ChangeUnlModifyOutcome),
    Unknown(Ter),
}

impl ChangeOwnerDoApplyCarrier {
    pub fn amendment(
        facts: ChangeAmendmentFacts,
        amendments_entry: Option<DecodedAmendmentsEntry>,
    ) -> Self {
        Self {
            txn_type: TxType::AMENDMENT,
            branch: ChangeOwnerDoApplyBranch::Amendment {
                facts,
                amendments_entry,
            },
        }
    }

    pub fn fee(
        feature_xrp_fees_enabled: bool,
        existing_entry: Option<DecodedFeeSettingsEntry>,
        tx_fields: ChangeApplyFeeTxFields,
    ) -> Self {
        Self {
            txn_type: TxType::FEE,
            branch: ChangeOwnerDoApplyBranch::Fee {
                feature_xrp_fees_enabled,
                existing_entry,
                tx_fields,
            },
        }
    }

    pub fn unl_modify(
        facts: ChangeUnlModifyFacts,
        negative_unl_entry: Option<DecodedNegativeUnlEntry>,
    ) -> Self {
        Self {
            txn_type: TxType::UNL_MODIFY,
            branch: ChangeOwnerDoApplyBranch::UnlModify {
                facts,
                negative_unl_entry,
            },
        }
    }

    pub const fn txn_type(&self) -> TxType {
        self.txn_type
    }

    pub fn do_apply(self) -> ChangeOwnerDoApplyOutcome {
        match (self.txn_type, self.branch) {
            (
                TxType::AMENDMENT,
                ChangeOwnerDoApplyBranch::Amendment {
                    facts,
                    amendments_entry,
                },
            ) => ChangeOwnerDoApplyOutcome::Amendment(run_change_apply_amendment(
                &facts,
                amendments_entry.as_ref(),
            )),
            (
                TxType::FEE,
                ChangeOwnerDoApplyBranch::Fee {
                    feature_xrp_fees_enabled,
                    existing_entry,
                    tx_fields,
                },
            ) => ChangeOwnerDoApplyOutcome::Fee(ChangeOwnerFeeOutcome {
                field_plan: run_change_apply_fee_field_plan(feature_xrp_fees_enabled),
                mutation_plan: run_change_apply_fee_mutation_plan(
                    existing_entry.as_ref(),
                    tx_fields,
                ),
            }),
            (
                TxType::UNL_MODIFY,
                ChangeOwnerDoApplyBranch::UnlModify {
                    facts,
                    negative_unl_entry,
                },
            ) => ChangeOwnerDoApplyOutcome::UnlModify(run_change_apply_unl_modify(
                &facts,
                negative_unl_entry.as_ref(),
            )),
            _ => ChangeOwnerDoApplyOutcome::Unknown(Ter::TEF_FAILURE),
        }
    }
}

pub fn run_change_owner_do_apply(carrier: ChangeOwnerDoApplyCarrier) -> ChangeOwnerDoApplyOutcome {
    carrier.do_apply()
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::{
        DecodedAmendmentsEntry, DecodedDisabledValidator, DecodedFeeSettingsEntry,
        DecodedNegativeUnlEntry, Ter, trans_token,
    };

    use super::{ChangeOwnerDoApplyCarrier, ChangeOwnerDoApplyOutcome, run_change_owner_do_apply};
    use crate::change_base_fee::{ChangeApplyFeeTxFields, run_change_apply_fee_field_plan};
    use crate::{ChangeAmendmentFacts, ChangeUnlModifyFacts};

    fn test_validator(tag: u8) -> Vec<u8> {
        vec![tag; 33]
    }

    fn test_amendment(tag: u8) -> Uint256 {
        Uint256::from_array([tag; 32])
    }

    #[test]
    fn change_owner_dispatches_amendment_shell() {
        let carrier = ChangeOwnerDoApplyCarrier::amendment(
            ChangeAmendmentFacts {
                amendment: test_amendment(0x21),
                got_majority: true,
                lost_majority: false,
                parent_close_time: 9,
                amendment_supported: true,
            },
            Some(DecodedAmendmentsEntry::default()),
        );

        assert_eq!(carrier.txn_type(), protocol::TxType::AMENDMENT);
        match carrier.do_apply() {
            ChangeOwnerDoApplyOutcome::Amendment(outcome) => {
                assert_eq!(outcome.result, Ter::TES_SUCCESS);
                assert_eq!(trans_token(outcome.result), "tesSUCCESS");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn change_owner_shapes_fee_result_with_field_plan_and_mutation_plan() {
        let carrier = ChangeOwnerDoApplyCarrier::fee(
            true,
            Some(DecodedFeeSettingsEntry {
                base_fee: Some(1),
                reference_fee_units: Some(2),
                reserve_base: Some(3),
                reserve_increment: Some(4),
                ..DecodedFeeSettingsEntry::default()
            }),
            ChangeApplyFeeTxFields::XrpDrops {
                base_fee_drops: protocol::DecodedAmountField {
                    drops: 10,
                    native: true,
                    negative: false,
                },
                reserve_base_drops: protocol::DecodedAmountField {
                    drops: 20,
                    native: true,
                    negative: false,
                },
                reserve_increment_drops: protocol::DecodedAmountField {
                    drops: 30,
                    native: true,
                    negative: false,
                },
            },
        );

        match carrier.do_apply() {
            ChangeOwnerDoApplyOutcome::Fee(outcome) => {
                assert_eq!(outcome.field_plan, run_change_apply_fee_field_plan(true));
                assert_eq!(outcome.mutation_plan.entry.base_fee, None);
                assert_eq!(
                    outcome.mutation_plan.entry.base_fee_drops.unwrap().drops,
                    10
                );
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn change_owner_shapes_unl_modify_result_shell() {
        let validator = test_validator(0x44);
        let existing_validator = test_validator(0x45);
        let carrier = ChangeOwnerDoApplyCarrier::unl_modify(
            ChangeUnlModifyFacts {
                is_flag_ledger: true,
                unl_modify_disabling: Some(1),
                ledger_sequence: Some(7),
                current_ledger_sequence: 7,
                validator_public_key: Some(validator.clone()),
                validator_public_key_type_known: true,
            },
            Some(DecodedNegativeUnlEntry {
                disabled_validators: vec![DecodedDisabledValidator {
                    public_key: existing_validator,
                    first_ledger_sequence: 3,
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
                        .and_then(|entry| entry.validator_to_disable),
                    Some(validator)
                );
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
}
