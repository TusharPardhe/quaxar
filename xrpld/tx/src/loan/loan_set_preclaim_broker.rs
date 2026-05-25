//! Broker-existence branch for the reference implementation.
//!
//! This module ports the deterministic behavior around:
//!
//! - reading the broker id from the transaction,
//! - attempting exactly one broker lookup through an injected callback,
//! - returning the loaded broker object unchanged when present, and
//! - mapping a missing broker to `tecNO_ENTRY` with the current warning text.

use protocol::Ter;

pub const LOAN_SET_BROKER_DOES_NOT_EXIST_WARNING: &str = "LoanBroker does not exist.";

pub trait LoanSetPreclaimBrokerTx {
    type BrokerId;

    fn broker_id(&self) -> &Self::BrokerId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetPreclaimBrokerFailure {
    BrokerDoesNotExist,
}

impl LoanSetPreclaimBrokerFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::BrokerDoesNotExist => Ter::TEC_NO_ENTRY,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::BrokerDoesNotExist => LOAN_SET_BROKER_DOES_NOT_EXIST_WARNING,
        }
    }
}

pub fn check_loan_set_preclaim_broker<Tx, Broker, ReadBroker>(
    tx: &Tx,
    read_broker: ReadBroker,
) -> Result<Broker, LoanSetPreclaimBrokerFailure>
where
    Tx: LoanSetPreclaimBrokerTx,
    ReadBroker: FnOnce(&Tx::BrokerId) -> Option<Broker>,
{
    read_broker(tx.broker_id()).ok_or(LoanSetPreclaimBrokerFailure::BrokerDoesNotExist)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::trans_token;

    use super::{
        LOAN_SET_BROKER_DOES_NOT_EXIST_WARNING, LoanSetPreclaimBrokerFailure,
        LoanSetPreclaimBrokerTx, check_loan_set_preclaim_broker,
    };

    struct TestTx {
        broker_id: &'static str,
    }

    impl LoanSetPreclaimBrokerTx for TestTx {
        type BrokerId = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }
    }

    #[test]
    fn loan_set_preclaim_broker_returns_loaded_broker_unchanged() {
        let result = check_loan_set_preclaim_broker(
            &TestTx {
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
    fn loan_set_preclaim_broker_returns_no_entry_when_broker_is_missing() {
        let result = check_loan_set_preclaim_broker(
            &TestTx {
                broker_id: "missing-broker",
            },
            |_| None::<&'static str>,
        );

        assert_eq!(
            result,
            Err(LoanSetPreclaimBrokerFailure::BrokerDoesNotExist)
        );
        assert_eq!(result.unwrap_err().ter(), protocol::Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result.unwrap_err().ter()), "tecNO_ENTRY");
        assert_eq!(
            result.unwrap_err().warning_message(),
            LOAN_SET_BROKER_DOES_NOT_EXIST_WARNING
        );
    }

    #[test]
    fn loan_set_preclaim_broker_reads_broker_exactly_once() {
        let seen = RefCell::new(Vec::new());

        let result = check_loan_set_preclaim_broker(
            &TestTx {
                broker_id: "broker-id",
            },
            |broker_id| {
                seen.borrow_mut().push(*broker_id);
                Some("broker-sle")
            },
        );

        assert_eq!(result, Ok("broker-sle"));
        assert_eq!(*seen.borrow(), vec!["broker-id"]);
    }
}
