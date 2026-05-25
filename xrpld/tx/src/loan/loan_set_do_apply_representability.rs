//! Representability loop for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - iterating the current `LoanSet::getValueFields()` list in canonical order,
//! - skipping absent optional values,
//! - returning the first field whose value cannot be represented, and
//! - mapping that failure to `tecPRECISION_LOSS` with the current `doApply()`
//!   warning string that includes total loan value and loan scale.

use std::fmt::Display;

use protocol::Ter;

pub use crate::{
    LoanSetPreclaimRepresentabilityTx as LoanSetDoApplyRepresentabilityTx,
    LoanSetRepresentabilityField,
};

const LOAN_SET_DO_APPLY_REPRESENTABILITY_FIELDS: [LoanSetRepresentabilityField; 5] = [
    LoanSetRepresentabilityField::PrincipalRequested,
    LoanSetRepresentabilityField::LoanOriginationFee,
    LoanSetRepresentabilityField::LoanServiceFee,
    LoanSetRepresentabilityField::LatePaymentFee,
    LoanSetRepresentabilityField::ClosePaymentFee,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetDoApplyRepresentabilityFailure {
    field: LoanSetRepresentabilityField,
    value_display: String,
    total_value_display: String,
    loan_scale: i32,
}

impl LoanSetDoApplyRepresentabilityFailure {
    pub const fn ter(&self) -> Ter {
        Ter::TEC_PRECISION_LOSS
    }

    pub const fn field(&self) -> LoanSetRepresentabilityField {
        self.field
    }

    pub const fn loan_scale(&self) -> i32 {
        self.loan_scale
    }

    pub fn warning_message(&self) -> String {
        format!(
            "{} ({}) has too much precision. Total loan value is {} with a scale of {}",
            self.field.display_name(),
            self.value_display,
            self.total_value_display,
            self.loan_scale
        )
    }
}

pub fn check_loan_set_do_apply_representability<Tx, TotalValue, CanRepresent>(
    tx: &Tx,
    total_value: &TotalValue,
    loan_scale: i32,
    mut can_represent: CanRepresent,
) -> Result<(), LoanSetDoApplyRepresentabilityFailure>
where
    Tx: LoanSetDoApplyRepresentabilityTx,
    TotalValue: Display,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
{
    for field in LOAN_SET_DO_APPLY_REPRESENTABILITY_FIELDS {
        let Some(value) = tx.value(field) else {
            continue;
        };

        if !can_represent(field, value) {
            return Err(LoanSetDoApplyRepresentabilityFailure {
                field,
                value_display: value.to_string(),
                total_value_display: total_value.to_string(),
                loan_scale,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use protocol::trans_token;

    use super::{
        LoanSetDoApplyRepresentabilityFailure, LoanSetDoApplyRepresentabilityTx,
        LoanSetRepresentabilityField, check_loan_set_do_apply_representability,
    };

    struct TestTx {
        values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
    }

    impl LoanSetDoApplyRepresentabilityTx for TestTx {
        type Value = &'static str;

        fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
            self.values.get(&field)
        }
    }

    #[test]
    fn loan_set_do_apply_representability_checks_fields_in() {
        let tx = TestTx {
            values: BTreeMap::from([
                (LoanSetRepresentabilityField::ClosePaymentFee, "5"),
                (LoanSetRepresentabilityField::PrincipalRequested, "1"),
                (LoanSetRepresentabilityField::LoanServiceFee, "3"),
            ]),
        };
        let mut seen = Vec::new();

        let result = check_loan_set_do_apply_representability(&tx, &"1200", 2, |field, _| {
            seen.push(field);
            true
        });

        assert_eq!(result, Ok(()));
        assert_eq!(
            seen,
            vec![
                LoanSetRepresentabilityField::PrincipalRequested,
                LoanSetRepresentabilityField::LoanServiceFee,
                LoanSetRepresentabilityField::ClosePaymentFee
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_representability_returns_precision_loss_for_first_bad_field() {
        let tx = TestTx {
            values: BTreeMap::from([
                (LoanSetRepresentabilityField::PrincipalRequested, "6.5"),
                (LoanSetRepresentabilityField::LoanOriginationFee, "2.1"),
            ]),
        };

        let result = check_loan_set_do_apply_representability(&tx, &"1234.56", 4, |field, _| {
            field != LoanSetRepresentabilityField::PrincipalRequested
        });

        let err = result.expect_err("first bad field should fail");
        assert_eq!(
            err.field(),
            LoanSetRepresentabilityField::PrincipalRequested
        );
        assert_eq!(err.loan_scale(), 4);
        assert_eq!(err.ter(), protocol::Ter::TEC_PRECISION_LOSS);
        assert_eq!(trans_token(err.ter()), "tecPRECISION_LOSS");
        assert_eq!(
            err.warning_message(),
            "PrincipalRequested (6.5) has too much precision. Total loan value is 1234.56 with a scale of 4"
        );
    }

    #[test]
    fn loan_set_do_apply_representability_stops_after_first_failure() {
        let tx = TestTx {
            values: BTreeMap::from([
                (LoanSetRepresentabilityField::PrincipalRequested, "6.5"),
                (LoanSetRepresentabilityField::LoanOriginationFee, "2.1"),
            ]),
        };
        let mut seen = Vec::new();

        let result = check_loan_set_do_apply_representability(&tx, &"1234.56", 4, |field, _| {
            seen.push(field);
            field != LoanSetRepresentabilityField::PrincipalRequested
        });

        assert_eq!(
            result,
            Err(LoanSetDoApplyRepresentabilityFailure {
                field: LoanSetRepresentabilityField::PrincipalRequested,
                value_display: "6.5".to_string(),
                total_value_display: "1234.56".to_string(),
                loan_scale: 4,
            })
        );
        assert_eq!(seen, vec![LoanSetRepresentabilityField::PrincipalRequested]);
    }
}
