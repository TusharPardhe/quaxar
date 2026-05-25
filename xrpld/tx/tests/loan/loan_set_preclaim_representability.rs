//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! representability loop to the current C++ behavior.

use std::collections::BTreeMap;

use protocol::{Ter, trans_token};
use tx::{
    LoanSetPreclaimRepresentabilityTx, LoanSetRepresentabilityField,
    check_loan_set_preclaim_representability,
};

struct StubTx {
    values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
}

impl LoanSetPreclaimRepresentabilityTx for StubTx {
    type Value = &'static str;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
        self.values.get(&field)
    }
}

#[test]
fn tx_loan_set_preclaim_representability_checks_current_cpp_field_order() {
    let tx = StubTx {
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
fn tx_loan_set_preclaim_representability_skips_absent_fields() {
    let tx = StubTx {
        values: BTreeMap::new(),
    };

    let result = check_loan_set_preclaim_representability(&tx, &"XRP", |_, _| false);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_preclaim_representability_returns_precision_loss() {
    let tx = StubTx {
        values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "6.5")]),
    };

    let result = check_loan_set_preclaim_representability(&tx, &"XRP", |field, _| {
        field != LoanSetRepresentabilityField::PrincipalRequested
    });

    let err = result.unwrap_err();
    assert_eq!(
        err.field(),
        LoanSetRepresentabilityField::PrincipalRequested
    );
    assert_eq!(err.ter(), Ter::TEC_PRECISION_LOSS);
    assert_eq!(trans_token(err.ter()), "tecPRECISION_LOSS");
    assert_eq!(
        err.warning_message(),
        "PrincipalRequested (6.5) can not be represented as a(n) XRP."
    );
}

#[test]
fn tx_loan_set_preclaim_representability_returns_first_failure_unchanged() {
    let tx = StubTx {
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
        result.unwrap_err().field(),
        LoanSetRepresentabilityField::PrincipalRequested
    );
    assert_eq!(seen, vec![LoanSetRepresentabilityField::PrincipalRequested]);
}
