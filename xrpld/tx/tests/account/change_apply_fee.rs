//! Integration tests that pin the narrowed `Change.cpp::applyFee(...)`
//! routing to the current C++ field-shape split.

use protocol::{DecodedAmountField, DecodedFeeSettingsEntry};
use tx::change_base_fee::{
    CHANGE_APPLY_FEE_LEGACY_FIELDS_TO_SET, CHANGE_APPLY_FEE_XRP_FIELDS_TO_CLEAR,
    CHANGE_APPLY_FEE_XRP_FIELDS_TO_SET, ChangeApplyFeeTxFields, run_change_apply_fee_field_plan,
    run_change_apply_fee_mutation_plan,
};

fn amount(drops: u64) -> DecodedAmountField {
    DecodedAmountField {
        drops,
        native: true,
        negative: false,
    }
}

#[test]
fn change_apply_fee_plan_uses_legacy_fields_without_xrp_fees() {
    let plan = run_change_apply_fee_field_plan(false);

    assert_eq!(plan.fields_to_set, &CHANGE_APPLY_FEE_LEGACY_FIELDS_TO_SET);
    assert!(plan.fields_to_clear.is_empty());
}

#[test]
fn change_apply_fee_plan_sets_xrp_fields_and_clears_legacy_fields() {
    let plan = run_change_apply_fee_field_plan(true);

    assert_eq!(plan.fields_to_set, &CHANGE_APPLY_FEE_XRP_FIELDS_TO_SET);
    assert_eq!(plan.fields_to_clear, &CHANGE_APPLY_FEE_XRP_FIELDS_TO_CLEAR);
}

#[test]
fn change_apply_fee_mutation_plan_creates_missing_object_and_sets_legacy_fields() {
    let plan = run_change_apply_fee_mutation_plan(
        None,
        ChangeApplyFeeTxFields::Legacy {
            base_fee: 10,
            reference_fee_units: 20,
            reserve_base: 30,
            reserve_increment: 40,
        },
    );

    assert!(plan.create_if_missing);
    assert_eq!(plan.entry.base_fee, Some(10));
    assert_eq!(plan.entry.reference_fee_units, Some(20));
    assert_eq!(plan.entry.reserve_base, Some(30));
    assert_eq!(plan.entry.reserve_increment, Some(40));
}

#[test]
fn change_apply_fee_mutation_plan_clears_legacy_fields_only_on_xrp_fees_path() {
    let xrp = run_change_apply_fee_mutation_plan(
        Some(&DecodedFeeSettingsEntry {
            base_fee: Some(1),
            reference_fee_units: Some(2),
            reserve_base: Some(3),
            reserve_increment: Some(4),
            ..DecodedFeeSettingsEntry::default()
        }),
        ChangeApplyFeeTxFields::XrpDrops {
            base_fee_drops: amount(50),
            reserve_base_drops: amount(60),
            reserve_increment_drops: amount(70),
        },
    );
    let legacy = run_change_apply_fee_mutation_plan(
        Some(&DecodedFeeSettingsEntry {
            base_fee_drops: Some(amount(5)),
            reserve_base_drops: Some(amount(6)),
            reserve_increment_drops: Some(amount(7)),
            ..DecodedFeeSettingsEntry::default()
        }),
        ChangeApplyFeeTxFields::Legacy {
            base_fee: 11,
            reference_fee_units: 12,
            reserve_base: 13,
            reserve_increment: 14,
        },
    );

    assert_eq!(xrp.entry.base_fee, None);
    assert_eq!(xrp.entry.reference_fee_units, None);
    assert_eq!(xrp.entry.reserve_base, None);
    assert_eq!(xrp.entry.reserve_increment, None);
    assert_eq!(legacy.entry.base_fee_drops, Some(amount(5)));
    assert_eq!(legacy.entry.reserve_base_drops, Some(amount(6)));
    assert_eq!(legacy.entry.reserve_increment_drops, Some(amount(7)));
}
