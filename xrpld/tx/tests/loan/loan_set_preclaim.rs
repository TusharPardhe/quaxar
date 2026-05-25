//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! schedule-overflow guard and higher top-level shell to the current C++
//! behavior.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_GRACE_PERIOD_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING,
    LOAN_SET_LAST_PAYMENT_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING,
    LOAN_SET_PAYMENT_INTERVAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING,
    LOAN_SET_PAYMENT_TOTAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING, LoanSetPreclaimBrokerTx,
    LoanSetPreclaimLoadedBroker, LoanSetPreclaimLoadedVault, LoanSetPreclaimPermissionTx,
    LoanSetPreclaimRepresentabilityTx, LoanSetPreclaimVaultLimit, LoanSetRepresentabilityField,
    LoanSetScheduleGuardFailure, LoanSetScheduleGuardInputs, check_loan_set_schedule_guard,
    run_loan_set_preclaim,
};

fn base_inputs() -> LoanSetScheduleGuardInputs {
    LoanSetScheduleGuardInputs {
        start_date: 100,
        payment_interval: Some(60),
        payment_total: Some(1),
        grace_period: Some(0),
        default_payment_interval: 60,
        default_payment_total: 1,
        default_grace_period: 0,
    }
}

#[test]
fn tx_loan_set_preclaim_schedule_guard_returns_grace_failure_first() {
    let result = check_loan_set_schedule_guard(LoanSetScheduleGuardInputs {
        start_date: u32::MAX - 5,
        payment_interval: Some(10),
        payment_total: Some(10),
        grace_period: Some(6),
        ..base_inputs()
    });

    assert_eq!(
        result,
        Err(LoanSetScheduleGuardFailure::GracePeriodExceedsProtocolTimeLimit)
    );
    assert_eq!(
        result.unwrap_err().warning_message(),
        LOAN_SET_GRACE_PERIOD_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
    );
    assert_eq!(result.unwrap_err().ter(), Ter::TEC_KILLED);
    assert_eq!(trans_token(result.unwrap_err().ter()), "tecKILLED");
}

#[test]
fn tx_loan_set_preclaim_schedule_guard_returns_interval_failure() {
    let result = check_loan_set_schedule_guard(LoanSetScheduleGuardInputs {
        start_date: u32::MAX - 5,
        payment_interval: Some(6),
        payment_total: Some(1),
        grace_period: Some(5),
        ..base_inputs()
    });

    assert_eq!(
        result,
        Err(LoanSetScheduleGuardFailure::PaymentIntervalExceedsProtocolTimeLimit)
    );
    assert_eq!(
        result.unwrap_err().warning_message(),
        LOAN_SET_PAYMENT_INTERVAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
    );
}

#[test]
fn tx_loan_set_preclaim_schedule_guard_returns_total_failure() {
    let result = check_loan_set_schedule_guard(LoanSetScheduleGuardInputs {
        start_date: u32::MAX - 5,
        payment_interval: Some(5),
        payment_total: Some(6),
        grace_period: Some(0),
        ..base_inputs()
    });

    assert_eq!(
        result,
        Err(LoanSetScheduleGuardFailure::PaymentTotalExceedsProtocolTimeLimit)
    );
    assert_eq!(
        result.unwrap_err().warning_message(),
        LOAN_SET_PAYMENT_TOTAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
    );
}

#[test]
fn tx_loan_set_preclaim_schedule_guard_returns_last_payment_failure() {
    let result = check_loan_set_schedule_guard(LoanSetScheduleGuardInputs {
        start_date: 100,
        payment_interval: Some(1_000_000_000),
        payment_total: Some(10),
        grace_period: Some(0),
        ..base_inputs()
    });

    assert_eq!(
        result,
        Err(LoanSetScheduleGuardFailure::LastPaymentExceedsProtocolTimeLimit)
    );
    assert_eq!(
        result.unwrap_err().warning_message(),
        LOAN_SET_LAST_PAYMENT_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
    );
}

#[test]
fn tx_loan_set_preclaim_schedule_guard_accepts_exact_boundary() {
    let result = check_loan_set_schedule_guard(LoanSetScheduleGuardInputs {
        start_date: u32::MAX - 100,
        payment_interval: Some(100),
        payment_total: Some(1),
        grace_period: Some(0),
        ..base_inputs()
    });

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_preclaim_schedule_guard_uses_defaults() {
    let result = check_loan_set_schedule_guard(LoanSetScheduleGuardInputs {
        start_date: u32::MAX - 5,
        payment_interval: None,
        payment_total: None,
        grace_period: None,
        default_payment_interval: 6,
        default_payment_total: 1,
        default_grace_period: 0,
    });

    assert_eq!(
        result,
        Err(LoanSetScheduleGuardFailure::PaymentIntervalExceedsProtocolTimeLimit)
    );
}

struct StubTx {
    broker_id: &'static str,
    account: &'static str,
    counterparty: Option<&'static str>,
    values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
}

impl LoanSetPreclaimBrokerTx for StubTx {
    type BrokerId = &'static str;

    fn broker_id(&self) -> &Self::BrokerId {
        &self.broker_id
    }
}

impl LoanSetPreclaimPermissionTx for StubTx {
    type AccountId = &'static str;

    fn account(&self) -> Self::AccountId {
        self.account
    }

    fn counterparty(&self) -> Option<Self::AccountId> {
        self.counterparty
    }
}

impl LoanSetPreclaimRepresentabilityTx for StubTx {
    type Value = &'static str;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
        self.values.get(&field)
    }
}

#[derive(Clone, Copy)]
struct StubBroker {
    owner: &'static str,
    pseudo_account: &'static str,
    vault_id: &'static str,
}

impl LoanSetPreclaimLoadedBroker for StubBroker {
    type AccountId = &'static str;
    type VaultId = &'static str;

    fn owner(&self) -> &Self::AccountId {
        &self.owner
    }

    fn pseudo_account(&self) -> &Self::AccountId {
        &self.pseudo_account
    }

    fn vault_id(&self) -> &Self::VaultId {
        &self.vault_id
    }
}

#[derive(Clone, Copy)]
struct StubVault {
    assets_maximum: u32,
    assets_total: u32,
    pseudo_account: &'static str,
    asset: &'static str,
}

impl LoanSetPreclaimVaultLimit for StubVault {
    type Amount = u32;

    fn assets_maximum(&self) -> &Self::Amount {
        &self.assets_maximum
    }

    fn assets_total(&self) -> &Self::Amount {
        &self.assets_total
    }
}

impl LoanSetPreclaimLoadedVault for StubVault {
    type AccountId = &'static str;
    type Asset = &'static str;

    fn pseudo_account(&self) -> &Self::AccountId {
        &self.pseudo_account
    }

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

#[test]
fn tx_loan_set_preclaim_short_circuits_schedule_guard_before_reads() {
    let result = run_loan_set_preclaim(
        &StubTx {
            broker_id: "broker-id",
            account: "borrower",
            counterparty: None,
            values: BTreeMap::new(),
        },
        LoanSetScheduleGuardInputs {
            start_date: u32::MAX - 5,
            payment_interval: Some(10),
            payment_total: Some(10),
            grace_period: Some(6),
            ..base_inputs()
        },
        |_| -> Option<StubBroker> { panic!("schedule failure should skip broker lookup") },
        |_| -> Option<&'static str> { panic!("schedule failure should skip borrower lookup") },
        |_| -> Option<StubVault> { panic!("schedule failure should skip vault lookup") },
        |_, _| panic!("schedule failure should skip representability"),
        |_| panic!("schedule failure should skip canAddHolding"),
        |_, _| panic!("schedule failure should skip frozen checks"),
        |_, _| panic!("schedule failure should skip deep-frozen checks"),
    );

    assert_eq!(result, Ter::TEC_KILLED);
}

#[test]
fn tx_loan_set_preclaim_preserves_current_cpp_step_order() {
    let trace = RefCell::new(Vec::new());

    let result = run_loan_set_preclaim(
        &StubTx {
            broker_id: "broker-id",
            account: "borrower",
            counterparty: None,
            values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "10")]),
        },
        base_inputs(),
        |broker_id| {
            trace.borrow_mut().push("broker");
            assert_eq!(*broker_id, "broker-id");
            Some(StubBroker {
                owner: "broker-owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault-id",
            })
        },
        |borrower| {
            trace.borrow_mut().push("borrower");
            assert_eq!(*borrower, "borrower");
            Some("borrower-sle")
        },
        |vault_id| {
            trace.borrow_mut().push("vault");
            assert_eq!(*vault_id, "vault-id");
            Some(StubVault {
                assets_maximum: 100,
                assets_total: 10,
                pseudo_account: "vault-pseudo",
                asset: "USD",
            })
        },
        |field, value| {
            trace.borrow_mut().push("representability");
            assert_eq!(field, LoanSetRepresentabilityField::PrincipalRequested);
            assert_eq!(*value, "10");
            true
        },
        |asset| {
            trace.borrow_mut().push("can-add-holding");
            assert_eq!(*asset, "USD");
            Ter::TES_SUCCESS
        },
        |account, asset| {
            trace.borrow_mut().push("frozen");
            assert_eq!(*asset, "USD");
            assert!(*account == "vault-pseudo" || *account == "borrower");
            Ter::TES_SUCCESS
        },
        |account, asset| {
            trace.borrow_mut().push("deep-frozen");
            assert_eq!(*asset, "USD");
            assert!(*account == "broker-pseudo" || *account == "broker-owner");
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        trace.into_inner(),
        vec![
            "broker",
            "borrower",
            "vault",
            "representability",
            "can-add-holding",
            "frozen",
            "deep-frozen",
            "frozen",
            "deep-frozen"
        ]
    );
}

#[test]
fn tx_loan_set_preclaim_returns_first_failure_unchanged() {
    let representability_failure = run_loan_set_preclaim(
        &StubTx {
            broker_id: "broker-id",
            account: "borrower",
            counterparty: None,
            values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "10.5")]),
        },
        base_inputs(),
        |_| {
            Some(StubBroker {
                owner: "broker-owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault-id",
            })
        },
        |_| Some("borrower-sle"),
        |_| {
            Some(StubVault {
                assets_maximum: 100,
                assets_total: 10,
                pseudo_account: "vault-pseudo",
                asset: "USD",
            })
        },
        |_, _| false,
        |_| panic!("representability failure should skip canAddHolding"),
        |_, _| panic!("representability failure should skip frozen checks"),
        |_, _| panic!("representability failure should skip deep-frozen checks"),
    );
    let holding_failure = run_loan_set_preclaim(
        &StubTx {
            broker_id: "broker-id",
            account: "borrower",
            counterparty: None,
            values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "10")]),
        },
        base_inputs(),
        |_| {
            Some(StubBroker {
                owner: "broker-owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault-id",
            })
        },
        |_| Some("borrower-sle"),
        |_| {
            Some(StubVault {
                assets_maximum: 100,
                assets_total: 10,
                pseudo_account: "vault-pseudo",
                asset: "USD",
            })
        },
        |_, _| true,
        |_| Ter::TER_NO_RIPPLE,
        |_, _| panic!("canAddHolding failure should skip frozen checks"),
        |_, _| panic!("canAddHolding failure should skip deep-frozen checks"),
    );

    assert_eq!(representability_failure, Ter::TEC_PRECISION_LOSS);
    assert_eq!(holding_failure, Ter::TER_NO_RIPPLE);
}
