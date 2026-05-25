//! Representability loop for the reference implementation.
//!
//! This module ports the deterministic behavior around:
//!
//! - iterating the current `LoanSet::getValueFields()` list in canonical order,
//! - skipping absent optional values,
//! - returning the first field whose value cannot be represented as the vault
//!   asset type, and
//! - mapping that failure to `tecPRECISION_LOSS` with the current warning
//!   string.

use std::fmt::Display;

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LoanSetRepresentabilityField {
    PrincipalRequested,
    LoanOriginationFee,
    LoanServiceFee,
    LatePaymentFee,
    ClosePaymentFee,
}

impl LoanSetRepresentabilityField {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::PrincipalRequested => "PrincipalRequested",
            Self::LoanOriginationFee => "LoanOriginationFee",
            Self::LoanServiceFee => "LoanServiceFee",
            Self::LatePaymentFee => "LatePaymentFee",
            Self::ClosePaymentFee => "ClosePaymentFee",
        }
    }
}

const LOAN_SET_REPRESENTABILITY_FIELDS: [LoanSetRepresentabilityField; 5] = [
    LoanSetRepresentabilityField::PrincipalRequested,
    LoanSetRepresentabilityField::LoanOriginationFee,
    LoanSetRepresentabilityField::LoanServiceFee,
    LoanSetRepresentabilityField::LatePaymentFee,
    LoanSetRepresentabilityField::ClosePaymentFee,
];

pub trait LoanSetPreclaimRepresentabilityTx {
    type Value: Display;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetPreclaimRepresentabilityFailure {
    field: LoanSetRepresentabilityField,
    value_display: String,
    asset_display: String,
}

impl LoanSetPreclaimRepresentabilityFailure {
    pub const fn ter(&self) -> Ter {
        Ter::TEC_PRECISION_LOSS
    }

    pub const fn field(&self) -> LoanSetRepresentabilityField {
        self.field
    }

    pub fn warning_message(&self) -> String {
        format!(
            "{} ({}) can not be represented as a(n) {}.",
            self.field.display_name(),
            self.value_display,
            self.asset_display
        )
    }
}

pub fn check_loan_set_preclaim_representability<Tx, Asset, CanRepresent>(
    tx: &Tx,
    asset: &Asset,
    mut can_represent: CanRepresent,
) -> Result<(), LoanSetPreclaimRepresentabilityFailure>
where
    Tx: LoanSetPreclaimRepresentabilityTx,
    Asset: Display,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
{
    for field in LOAN_SET_REPRESENTABILITY_FIELDS {
        let Some(value) = tx.value(field) else {
            continue;
        };

        if !can_represent(field, value) {
            return Err(LoanSetPreclaimRepresentabilityFailure {
                field,
                value_display: value.to_string(),
                asset_display: asset.to_string(),
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
        LoanSetPreclaimRepresentabilityFailure, LoanSetPreclaimRepresentabilityTx,
        LoanSetRepresentabilityField, check_loan_set_preclaim_representability,
    };

    struct TestTx {
        values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
    }

    impl LoanSetPreclaimRepresentabilityTx for TestTx {
        type Value = &'static str;

        fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
            self.values.get(&field)
        }
    }

    #[test]
    fn loan_set_preclaim_representability_checks_fields_in() {
        let tx = TestTx {
            values: BTreeMap::from([
                (LoanSetRepresentabilityField::ClosePaymentFee, "5"),
                (LoanSetRepresentabilityField::PrincipalRequested, "1"),
                (LoanSetRepresentabilityField::LoanServiceFee, "3"),
            ]),
        };
        let mut seen = Vec::new();

        let result = check_loan_set_preclaim_representability(&tx, &"XRP", |field, _| {
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
    fn loan_set_preclaim_representability_skips_absent_fields() {
        let tx = TestTx {
            values: BTreeMap::new(),
        };

        let result = check_loan_set_preclaim_representability(&tx, &"XRP", |_, _| false);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_preclaim_representability_returns_precision_loss_for_first_bad_field() {
        let tx = TestTx {
            values: BTreeMap::from([
                (LoanSetRepresentabilityField::PrincipalRequested, "6.5"),
                (LoanSetRepresentabilityField::LoanOriginationFee, "2.1"),
            ]),
        };

        let result = check_loan_set_preclaim_representability(&tx, &"XRP", |field, _| {
            field != LoanSetRepresentabilityField::PrincipalRequested
        });

        let err = result.unwrap_err();
        assert_eq!(
            err.field(),
            LoanSetRepresentabilityField::PrincipalRequested
        );
        assert_eq!(err.ter(), protocol::Ter::TEC_PRECISION_LOSS);
        assert_eq!(trans_token(err.ter()), "tecPRECISION_LOSS");
        assert_eq!(
            err.warning_message(),
            "PrincipalRequested (6.5) can not be represented as a(n) XRP."
        );
    }

    #[test]
    fn loan_set_preclaim_representability_stops_after_first_failure() {
        let tx = TestTx {
            values: BTreeMap::from([
                (LoanSetRepresentabilityField::PrincipalRequested, "6.5"),
                (LoanSetRepresentabilityField::LoanOriginationFee, "2.1"),
            ]),
        };
        let mut seen = Vec::new();

        let result = check_loan_set_preclaim_representability(&tx, &"XRP", |field, _| {
            seen.push(field);
            field != LoanSetRepresentabilityField::PrincipalRequested
        });

        assert_eq!(
            result,
            Err(LoanSetPreclaimRepresentabilityFailure {
                field: LoanSetRepresentabilityField::PrincipalRequested,
                value_display: "6.5".to_string(),
                asset_display: "XRP".to_string(),
            })
        );
        assert_eq!(seen, vec![LoanSetRepresentabilityField::PrincipalRequested]);
    }
}
