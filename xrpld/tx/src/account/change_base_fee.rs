//! Current `Change::calculateBaseFee(...)` wrapper used by the
//! pseudo-transaction change family.

use protocol::{DecodedAmountField, DecodedFeeSettingsEntry};

/// Fixed field-plan shape for the current `Change::applyFee(...)` routing.
///
/// This stays narrower than a general `SLE` mutation port. It only captures
/// the deterministic field names that the reference code sets or clears for the two
/// fee layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeApplyFeeFieldPlan {
    pub fields_to_set: &'static [&'static str],
    pub fields_to_clear: &'static [&'static str],
}

pub const CHANGE_APPLY_FEE_LEGACY_FIELDS_TO_SET: [&str; 4] = [
    "sfBaseFee",
    "sfReferenceFeeUnits",
    "sfReserveBase",
    "sfReserveIncrement",
];

pub const CHANGE_APPLY_FEE_XRP_FIELDS_TO_SET: [&str; 3] = [
    "sfBaseFeeDrops",
    "sfReserveBaseDrops",
    "sfReserveIncrementDrops",
];

pub const CHANGE_APPLY_FEE_XRP_FIELDS_TO_CLEAR: [&str; 4] = [
    "sfBaseFee",
    "sfReferenceFeeUnits",
    "sfReserveBase",
    "sfReserveIncrement",
];

pub const CHANGE_APPLY_FEE_LEGACY_PLAN: ChangeApplyFeeFieldPlan = ChangeApplyFeeFieldPlan {
    fields_to_set: &CHANGE_APPLY_FEE_LEGACY_FIELDS_TO_SET,
    fields_to_clear: &[],
};

pub const CHANGE_APPLY_FEE_XRP_PLAN: ChangeApplyFeeFieldPlan = ChangeApplyFeeFieldPlan {
    fields_to_set: &CHANGE_APPLY_FEE_XRP_FIELDS_TO_SET,
    fields_to_clear: &CHANGE_APPLY_FEE_XRP_FIELDS_TO_CLEAR,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeApplyFeeTxFields {
    Legacy {
        base_fee: u64,
        reference_fee_units: u32,
        reserve_base: u32,
        reserve_increment: u32,
    },
    XrpDrops {
        base_fee_drops: DecodedAmountField,
        reserve_base_drops: DecodedAmountField,
        reserve_increment_drops: DecodedAmountField,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeApplyFeeMutationPlan {
    pub create_if_missing: bool,
    pub field_plan: ChangeApplyFeeFieldPlan,
    pub entry: DecodedFeeSettingsEntry,
}

pub fn run_change_calculate_base_fee<Fee>(zero_fee: Fee) -> Fee
where
    Fee: Copy,
{
    zero_fee
}

pub const fn run_change_apply_fee_field_plan(
    feature_xrp_fees_enabled: bool,
) -> ChangeApplyFeeFieldPlan {
    if feature_xrp_fees_enabled {
        CHANGE_APPLY_FEE_XRP_PLAN
    } else {
        CHANGE_APPLY_FEE_LEGACY_PLAN
    }
}

pub fn run_change_apply_fee_mutation_plan(
    existing: Option<&DecodedFeeSettingsEntry>,
    tx_fields: ChangeApplyFeeTxFields,
) -> ChangeApplyFeeMutationPlan {
    let create_if_missing = existing.is_none();
    let mut entry = existing.cloned().unwrap_or_default();

    match tx_fields {
        ChangeApplyFeeTxFields::Legacy {
            base_fee,
            reference_fee_units,
            reserve_base,
            reserve_increment,
        } => {
            entry.base_fee = Some(base_fee);
            entry.reference_fee_units = Some(reference_fee_units);
            entry.reserve_base = Some(reserve_base);
            entry.reserve_increment = Some(reserve_increment);
        }
        ChangeApplyFeeTxFields::XrpDrops {
            base_fee_drops,
            reserve_base_drops,
            reserve_increment_drops,
        } => {
            entry.base_fee_drops = Some(base_fee_drops);
            entry.reserve_base_drops = Some(reserve_base_drops);
            entry.reserve_increment_drops = Some(reserve_increment_drops);
            entry.base_fee = None;
            entry.reference_fee_units = None;
            entry.reserve_base = None;
            entry.reserve_increment = None;
        }
    }

    ChangeApplyFeeMutationPlan {
        create_if_missing,
        field_plan: match tx_fields {
            ChangeApplyFeeTxFields::Legacy { .. } => CHANGE_APPLY_FEE_LEGACY_PLAN,
            ChangeApplyFeeTxFields::XrpDrops { .. } => CHANGE_APPLY_FEE_XRP_PLAN,
        },
        entry,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHANGE_APPLY_FEE_LEGACY_FIELDS_TO_SET, CHANGE_APPLY_FEE_LEGACY_PLAN,
        CHANGE_APPLY_FEE_XRP_FIELDS_TO_CLEAR, CHANGE_APPLY_FEE_XRP_FIELDS_TO_SET,
        CHANGE_APPLY_FEE_XRP_PLAN, ChangeApplyFeeTxFields, run_change_apply_fee_field_plan,
        run_change_apply_fee_mutation_plan, run_change_calculate_base_fee,
    };
    use protocol::{DecodedAmountField, DecodedFeeSettingsEntry};

    fn amount(drops: u64) -> DecodedAmountField {
        DecodedAmountField {
            drops,
            native: true,
            negative: false,
        }
    }

    #[test]
    fn change_calculate_base_fee_returns_zero() {
        let fee = run_change_calculate_base_fee(0_u64);

        assert_eq!(fee, 0);
    }

    #[test]
    fn change_apply_fee_field_plan_uses_legacy_shape_without_xrp_fees() {
        let plan = run_change_apply_fee_field_plan(false);

        assert_eq!(plan.fields_to_set, &CHANGE_APPLY_FEE_LEGACY_FIELDS_TO_SET);
        assert!(plan.fields_to_clear.is_empty());
    }

    #[test]
    fn change_apply_fee_field_plan_uses_xrp_shape_and_clears_legacy_fields() {
        let plan = run_change_apply_fee_field_plan(true);

        assert_eq!(plan.fields_to_set, &CHANGE_APPLY_FEE_XRP_FIELDS_TO_SET);
        assert_eq!(plan.fields_to_clear, &CHANGE_APPLY_FEE_XRP_FIELDS_TO_CLEAR);
    }

    #[test]
    fn change_apply_fee_mutation_plan_creates_legacy_fee_object() {
        let plan = run_change_apply_fee_mutation_plan(
            None,
            ChangeApplyFeeTxFields::Legacy {
                base_fee: 10,
                reference_fee_units: 11,
                reserve_base: 12,
                reserve_increment: 13,
            },
        );

        assert!(plan.create_if_missing);
        assert_eq!(plan.field_plan, CHANGE_APPLY_FEE_LEGACY_PLAN);
        assert_eq!(
            plan.entry,
            DecodedFeeSettingsEntry {
                base_fee: Some(10),
                reference_fee_units: Some(11),
                reserve_base: Some(12),
                reserve_increment: Some(13),
                ..DecodedFeeSettingsEntry::default()
            }
        );
    }

    #[test]
    fn change_apply_fee_mutation_plan_clears_legacy_fields_for_xrp_fees() {
        let plan = run_change_apply_fee_mutation_plan(
            Some(&DecodedFeeSettingsEntry {
                base_fee: Some(1),
                reference_fee_units: Some(2),
                reserve_base: Some(3),
                reserve_increment: Some(4),
                previous_txn_lgr_seq: Some(9),
                ..DecodedFeeSettingsEntry::default()
            }),
            ChangeApplyFeeTxFields::XrpDrops {
                base_fee_drops: amount(20),
                reserve_base_drops: amount(21),
                reserve_increment_drops: amount(22),
            },
        );

        assert!(!plan.create_if_missing);
        assert_eq!(plan.field_plan, CHANGE_APPLY_FEE_XRP_PLAN);
        assert_eq!(plan.entry.base_fee, None);
        assert_eq!(plan.entry.reference_fee_units, None);
        assert_eq!(plan.entry.reserve_base, None);
        assert_eq!(plan.entry.reserve_increment, None);
        assert_eq!(plan.entry.base_fee_drops, Some(amount(20)));
        assert_eq!(plan.entry.reserve_base_drops, Some(amount(21)));
        assert_eq!(plan.entry.reserve_increment_drops, Some(amount(22)));
        assert_eq!(plan.entry.previous_txn_lgr_seq, Some(9));
    }

    #[test]
    fn change_apply_fee_mutation_plan_preserves_xrp_fields_on_legacy_path() {
        let plan = run_change_apply_fee_mutation_plan(
            Some(&DecodedFeeSettingsEntry {
                base_fee_drops: Some(amount(30)),
                reserve_base_drops: Some(amount(31)),
                reserve_increment_drops: Some(amount(32)),
                previous_txn_lgr_seq: Some(77),
                ..DecodedFeeSettingsEntry::default()
            }),
            ChangeApplyFeeTxFields::Legacy {
                base_fee: 40,
                reference_fee_units: 41,
                reserve_base: 42,
                reserve_increment: 43,
            },
        );

        assert_eq!(plan.field_plan, CHANGE_APPLY_FEE_LEGACY_PLAN);
        assert_eq!(plan.entry.base_fee, Some(40));
        assert_eq!(plan.entry.reference_fee_units, Some(41));
        assert_eq!(plan.entry.reserve_base, Some(42));
        assert_eq!(plan.entry.reserve_increment, Some(43));
        assert_eq!(plan.entry.base_fee_drops, Some(amount(30)));
        assert_eq!(plan.entry.reserve_base_drops, Some(amount(31)));
        assert_eq!(plan.entry.reserve_increment_drops, Some(amount(32)));
        assert_eq!(plan.entry.previous_txn_lgr_seq, Some(77));
    }
}
