//! Public-surface parity checks for the `LoanSet` property helper.

use basics::number::NumberParts as RuntimeNumber;
use protocol::{Asset, TenthBips16, TenthBips32, xrp_issue};
use tx::{compute_loan_set_properties, construct_loan_set_state};

#[test]
fn tx_loan_set_state_derives_interest() {
    let state = construct_loan_set_state(
        RuntimeNumber::from_i64(200),
        RuntimeNumber::from_i64(150),
        RuntimeNumber::from_i64(10),
    );

    assert_eq!(state.interest_due, RuntimeNumber::from_i64(40));
}

#[test]
fn tx_loan_set_properties_zero_interest_keeps_equal_installments() {
    let props = compute_loan_set_properties(
        &protocol::Rules::new(std::iter::empty()),
        Asset::Issue(xrp_issue()),
        RuntimeNumber::from_i64(120),
        TenthBips32::new(0),
        30,
        12,
        TenthBips16::new(0),
        0,
    );

    assert_eq!(props.periodic_payment, RuntimeNumber::from_i64(10));
    assert_eq!(props.first_payment_principal, RuntimeNumber::from_i64(10));
}
