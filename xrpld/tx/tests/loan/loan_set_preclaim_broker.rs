//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! broker-existence branch to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_BROKER_DOES_NOT_EXIST_WARNING, LoanSetPreclaimBrokerFailure, LoanSetPreclaimBrokerTx,
    check_loan_set_preclaim_broker,
};

struct StubTx {
    broker_id: &'static str,
}

impl LoanSetPreclaimBrokerTx for StubTx {
    type BrokerId = &'static str;

    fn broker_id(&self) -> &Self::BrokerId {
        &self.broker_id
    }
}

#[test]
fn tx_loan_set_preclaim_broker_returns_loaded_broker() {
    let result = check_loan_set_preclaim_broker(
        &StubTx {
            broker_id: "broker-id",
        },
        |broker_id| {
            assert_eq!(*broker_id, "broker-id");
            Some("broker-sle")
        },
    );

    assert_eq!(result, Ok("broker-sle"));
}

#[test]
fn tx_loan_set_preclaim_broker_returns_no_entry_when_missing() {
    let result = check_loan_set_preclaim_broker(
        &StubTx {
            broker_id: "missing-broker",
        },
        |_| None::<&'static str>,
    );

    assert_eq!(
        result,
        Err(LoanSetPreclaimBrokerFailure::BrokerDoesNotExist)
    );
    assert_eq!(result.unwrap_err().ter(), Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result.unwrap_err().ter()), "tecNO_ENTRY");
    assert_eq!(
        result.unwrap_err().warning_message(),
        LOAN_SET_BROKER_DOES_NOT_EXIST_WARNING
    );
}
