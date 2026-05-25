//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! representability loop to the current C++ behavior.

use std::collections::BTreeMap;

use protocol::trans_token;
use tx::{
    LoanSetDoApplyRepresentabilityTx, LoanSetRepresentabilityField,
    check_loan_set_do_apply_representability,
};

struct StubTx {
    values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
}

impl LoanSetDoApplyRepresentabilityTx for StubTx {
    type Value = &'static str;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
        self.values.get(&field)
    }
}

#[test]
fn tx_loan_set_do_apply_representability_checks_fields_in() {
    let tx = StubTx {
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
fn tx_loan_set_do_apply_representability_returns_precision_loss() {
    let tx = StubTx {
        values: BTreeMap::from([
            (LoanSetRepresentabilityField::PrincipalRequested, "6.5"),
            (LoanSetRepresentabilityField::LoanOriginationFee, "2.1"),
        ]),
    };

    let result = check_loan_set_do_apply_representability(&tx, &"1234.56", 4, |field, _| {
        field != LoanSetRepresentabilityField::PrincipalRequested
    });

    let err = result.expect_err("first bad field should fail");
    assert_eq!(err.ter(), protocol::Ter::TEC_PRECISION_LOSS);
    assert_eq!(trans_token(err.ter()), "tecPRECISION_LOSS");
    assert_eq!(
        err.warning_message(),
        "PrincipalRequested (6.5) has too much precision. Total loan value is 1234.56 with a scale of 4"
    );
}

#[test]
fn tx_loan_set_do_apply_representability_stops_after_first_failure() {
    let tx = StubTx {
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

    assert!(result.is_err());
    assert_eq!(seen, vec![LoanSetRepresentabilityField::PrincipalRequested]);
}
