//! Top-level preclaim shell and schedule-overflow guard for
//! the reference implementation.
//!
//! This module ports the deterministic top-level ordering around:
//!
//! - the early schedule-overflow guard,
//! - broker lookup,
//! - broker-owner permission derivation,
//! - borrower lookup,
//! - the impossible missing-vault fallback to `tefBAD_LEDGER`,
//! - the vault-limit guard,
//! - the representability loop,
//! - the `canAddHolding(...)` pass-through,
//! - the vault-pseudo freeze guard,
//! - the broker-pseudo deep-freeze guard,
//! - the borrower freeze guard,
//! - the broker-owner deep-freeze guard,
//! - and the final `tesSUCCESS` tail.
//!
//! It also ports the deterministic early schedule guard around:
//!
//! - computing `timeAvailable` from the supplied start date,
//! - resolving optional payment fields through the the reference implementation defaults,
//! - checking grace-period overflow before interval overflow,
//! - checking interval overflow before total overflow,
//! - checking the final multiplied schedule bound through the current integer
//!   division test, and
//! - mapping each failure to `tecKILLED` with the current warning string.

use std::fmt::Display;

use protocol::{Ter, is_tes_success};

use crate::{
    LoanSetPreclaimBrokerTx, LoanSetPreclaimPermissionTx, LoanSetPreclaimRepresentabilityTx,
    LoanSetPreclaimVaultLimit, LoanSetRepresentabilityField, check_loan_set_preclaim_borrower,
    check_loan_set_preclaim_borrower_frozen, check_loan_set_preclaim_broker,
    check_loan_set_preclaim_broker_owner_deep_frozen,
    check_loan_set_preclaim_broker_pseudo_deep_frozen, check_loan_set_preclaim_permission,
    check_loan_set_preclaim_representability, check_loan_set_preclaim_vault_frozen,
    check_loan_set_preclaim_vault_limit, run_loan_set_preclaim_can_add_holding,
    run_loan_set_preclaim_success,
};

pub const LOAN_SET_GRACE_PERIOD_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING: &str =
    "Grace period exceeds protocol time limit.";
pub const LOAN_SET_PAYMENT_INTERVAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING: &str =
    "Payment interval exceeds protocol time limit.";
pub const LOAN_SET_PAYMENT_TOTAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING: &str =
    "Payment total exceeds protocol time limit.";
pub const LOAN_SET_LAST_PAYMENT_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING: &str =
    "Last payment due date, or grace period for last payment exceeds protocol time limit.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoanSetScheduleGuardInputs {
    pub start_date: u32,
    pub payment_interval: Option<u32>,
    pub payment_total: Option<u32>,
    pub grace_period: Option<u32>,
    pub default_payment_interval: u32,
    pub default_payment_total: u32,
    pub default_grace_period: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetScheduleGuardFailure {
    GracePeriodExceedsProtocolTimeLimit,
    PaymentIntervalExceedsProtocolTimeLimit,
    PaymentTotalExceedsProtocolTimeLimit,
    LastPaymentExceedsProtocolTimeLimit,
}

impl LoanSetScheduleGuardFailure {
    pub const fn ter(self) -> Ter {
        let _ = self;
        Ter::TEC_KILLED
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::GracePeriodExceedsProtocolTimeLimit => {
                LOAN_SET_GRACE_PERIOD_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
            }
            Self::PaymentIntervalExceedsProtocolTimeLimit => {
                LOAN_SET_PAYMENT_INTERVAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
            }
            Self::PaymentTotalExceedsProtocolTimeLimit => {
                LOAN_SET_PAYMENT_TOTAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
            }
            Self::LastPaymentExceedsProtocolTimeLimit => {
                LOAN_SET_LAST_PAYMENT_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING
            }
        }
    }
}

pub fn check_loan_set_schedule_guard(
    inputs: LoanSetScheduleGuardInputs,
) -> Result<(), LoanSetScheduleGuardFailure> {
    let time_available = u32::MAX - inputs.start_date;

    let interval = inputs
        .payment_interval
        .unwrap_or(inputs.default_payment_interval);
    let total = inputs.payment_total.unwrap_or(inputs.default_payment_total);
    let grace = inputs.grace_period.unwrap_or(inputs.default_grace_period);

    if grace > time_available {
        return Err(LoanSetScheduleGuardFailure::GracePeriodExceedsProtocolTimeLimit);
    }

    if interval > time_available {
        return Err(LoanSetScheduleGuardFailure::PaymentIntervalExceedsProtocolTimeLimit);
    }

    if total > time_available {
        return Err(LoanSetScheduleGuardFailure::PaymentTotalExceedsProtocolTimeLimit);
    }

    let time_last_payment = time_available - grace;

    if time_last_payment / interval < total {
        return Err(LoanSetScheduleGuardFailure::LastPaymentExceedsProtocolTimeLimit);
    }

    Ok(())
}

pub trait LoanSetPreclaimLoadedBroker {
    type AccountId: Clone;
    type VaultId;

    fn owner(&self) -> &Self::AccountId;
    fn pseudo_account(&self) -> &Self::AccountId;
    fn vault_id(&self) -> &Self::VaultId;
}

pub trait LoanSetPreclaimLoadedVault: LoanSetPreclaimVaultLimit {
    type AccountId: Clone;
    type Asset: Display;

    fn pseudo_account(&self) -> &Self::AccountId;
    fn asset(&self) -> &Self::Asset;
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_preclaim<
    Tx,
    AccountId,
    Broker,
    Borrower,
    Vault,
    ReadBroker,
    ReadBorrower,
    ReadVault,
    CanRepresent,
    CheckCanAddHolding,
    CheckFrozen,
    CheckDeepFrozen,
>(
    tx: &Tx,
    schedule_inputs: LoanSetScheduleGuardInputs,
    read_broker: ReadBroker,
    read_borrower: ReadBorrower,
    read_vault: ReadVault,
    can_represent: CanRepresent,
    check_can_add_holding: CheckCanAddHolding,
    mut check_frozen: CheckFrozen,
    mut check_deep_frozen: CheckDeepFrozen,
) -> Ter
where
    Tx: LoanSetPreclaimBrokerTx
        + LoanSetPreclaimPermissionTx<AccountId = AccountId>
        + LoanSetPreclaimRepresentabilityTx,
    AccountId: Clone + PartialEq + Eq,
    Broker: LoanSetPreclaimLoadedBroker<AccountId = AccountId>,
    Vault: LoanSetPreclaimLoadedVault<AccountId = AccountId>,
    ReadBroker: FnOnce(&Tx::BrokerId) -> Option<Broker>,
    ReadBorrower: FnOnce(&AccountId) -> Option<Borrower>,
    ReadVault: FnOnce(&Broker::VaultId) -> Option<Vault>,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
    CheckCanAddHolding: FnOnce(&Vault::Asset) -> Ter,
    CheckFrozen: FnMut(&AccountId, &Vault::Asset) -> Ter,
    CheckDeepFrozen: FnMut(&AccountId, &Vault::Asset) -> Ter,
{
    if let Err(err) = check_loan_set_schedule_guard(schedule_inputs) {
        return err.ter();
    }

    let broker = match check_loan_set_preclaim_broker(tx, read_broker) {
        Ok(broker) => broker,
        Err(err) => return err.ter(),
    };
    let broker_owner = broker.owner().clone();
    let broker_pseudo = broker.pseudo_account().clone();

    let permission = match check_loan_set_preclaim_permission(tx, broker_owner.clone()) {
        Ok(permission) => permission,
        Err(err) => return err.ter(),
    };

    if let Err(err) = check_loan_set_preclaim_borrower(&permission.borrower, read_borrower) {
        return err.ter();
    }

    let vault = match read_vault(broker.vault_id()) {
        Some(vault) => vault,
        None => return Ter::TEF_BAD_LEDGER,
    };
    let vault = match check_loan_set_preclaim_vault_limit(vault) {
        Ok(vault) => vault,
        Err(err) => return err.ter(),
    };
    let vault_pseudo = vault.pseudo_account().clone();
    let asset = vault.asset();

    if let Err(err) = check_loan_set_preclaim_representability(tx, asset, can_represent) {
        return err.ter();
    }

    let ter = run_loan_set_preclaim_can_add_holding(asset, check_can_add_holding);
    if !is_tes_success(ter) {
        return ter;
    }

    if let Err(err) =
        check_loan_set_preclaim_vault_frozen(&vault_pseudo, asset, |account, asset| {
            check_frozen(account, asset)
        })
    {
        return err.ter();
    }

    if let Err(err) = check_loan_set_preclaim_broker_pseudo_deep_frozen(
        &broker_pseudo,
        asset,
        |account, asset| check_deep_frozen(account, asset),
    ) {
        return err.ter();
    }

    if let Err(err) =
        check_loan_set_preclaim_borrower_frozen(&permission.borrower, asset, |account, asset| {
            check_frozen(account, asset)
        })
    {
        return err.ter();
    }

    if let Err(err) =
        check_loan_set_preclaim_broker_owner_deep_frozen(&broker_owner, asset, |account, asset| {
            check_deep_frozen(account, asset)
        })
    {
        return err.ter();
    }

    run_loan_set_preclaim_success()
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap};

    use protocol::trans_token;

    use super::{
        LOAN_SET_GRACE_PERIOD_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING,
        LOAN_SET_LAST_PAYMENT_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING,
        LOAN_SET_PAYMENT_INTERVAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING,
        LOAN_SET_PAYMENT_TOTAL_EXCEEDS_PROTOCOL_TIME_LIMIT_WARNING, LoanSetPreclaimLoadedBroker,
        LoanSetPreclaimLoadedVault, LoanSetScheduleGuardFailure, LoanSetScheduleGuardInputs,
        check_loan_set_schedule_guard, run_loan_set_preclaim,
    };
    use crate::{
        LoanSetPreclaimBrokerTx, LoanSetPreclaimPermissionTx, LoanSetPreclaimRepresentabilityTx,
        LoanSetPreclaimVaultLimit, LoanSetRepresentabilityField,
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
    fn loan_set_schedule_guard_returns_grace_failure_first() {
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
        assert_eq!(trans_token(result.unwrap_err().ter()), "tecKILLED");
    }

    #[test]
    fn loan_set_schedule_guard_returns_interval_failure_after_grace() {
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
    fn loan_set_schedule_guard_returns_total_failure_after_interval() {
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
    fn loan_set_schedule_guard_returns_last_payment_failure_for_multiplied_overflow() {
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
    fn loan_set_schedule_guard_uses_defaults() {
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

    #[test]
    fn loan_set_schedule_guard_accepts_exact_boundary() {
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
    fn loan_set_schedule_guard_accepts_exact_last_payment_division_boundary() {
        let result = check_loan_set_schedule_guard(LoanSetScheduleGuardInputs {
            start_date: u32::MAX - 120,
            payment_interval: Some(60),
            payment_total: Some(2),
            grace_period: Some(0),
            ..base_inputs()
        });

        assert_eq!(result, Ok(()));
    }

    struct TestTx {
        broker_id: &'static str,
        account: &'static str,
        counterparty: Option<&'static str>,
        values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
    }

    impl LoanSetPreclaimBrokerTx for TestTx {
        type BrokerId = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }
    }

    impl LoanSetPreclaimPermissionTx for TestTx {
        type AccountId = &'static str;

        fn account(&self) -> Self::AccountId {
            self.account
        }

        fn counterparty(&self) -> Option<Self::AccountId> {
            self.counterparty
        }
    }

    impl LoanSetPreclaimRepresentabilityTx for TestTx {
        type Value = &'static str;

        fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
            self.values.get(&field)
        }
    }

    #[derive(Clone, Copy)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
    }

    impl LoanSetPreclaimLoadedBroker for TestBroker {
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
    struct TestVault {
        assets_maximum: u32,
        assets_total: u32,
        pseudo_account: &'static str,
        asset: &'static str,
    }

    impl LoanSetPreclaimVaultLimit for TestVault {
        type Amount = u32;

        fn assets_maximum(&self) -> &Self::Amount {
            &self.assets_maximum
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }
    }

    impl LoanSetPreclaimLoadedVault for TestVault {
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
    fn loan_set_preclaim_short_circuits_schedule_guard_before_object_reads() {
        let result = run_loan_set_preclaim(
            &TestTx {
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
            |_| -> Option<TestBroker> { panic!("schedule failure should skip broker lookup") },
            |_| -> Option<&'static str> { panic!("schedule failure should skip borrower lookup") },
            |_| -> Option<TestVault> { panic!("schedule failure should skip vault lookup") },
            |_, _| panic!("schedule failure should skip representability"),
            |_| panic!("schedule failure should skip canAddHolding"),
            |_, _| panic!("schedule failure should skip frozen checks"),
            |_, _| panic!("schedule failure should skip deep-frozen checks"),
        );

        assert_eq!(result, protocol::Ter::TEC_KILLED);
    }

    #[test]
    fn loan_set_preclaim_preserves_current_cpp_step_order() {
        let trace = RefCell::new(Vec::new());

        let result = run_loan_set_preclaim(
            &TestTx {
                broker_id: "broker-id",
                account: "borrower",
                counterparty: None,
                values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "10")]),
            },
            base_inputs(),
            |broker_id| {
                trace.borrow_mut().push("broker");
                assert_eq!(*broker_id, "broker-id");
                Some(TestBroker {
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
                Some(TestVault {
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
                protocol::Ter::TES_SUCCESS
            },
            |account, asset| {
                trace.borrow_mut().push("frozen");
                assert_eq!(*asset, "USD");
                assert!(*account == "vault-pseudo" || *account == "borrower");
                protocol::Ter::TES_SUCCESS
            },
            |account, asset| {
                trace.borrow_mut().push("deep-frozen");
                assert_eq!(*asset, "USD");
                assert!(*account == "broker-pseudo" || *account == "broker-owner");
                protocol::Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
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
    fn loan_set_preclaim_returns_first_failure_unchanged() {
        let representability_failure = run_loan_set_preclaim(
            &TestTx {
                broker_id: "broker-id",
                account: "borrower",
                counterparty: None,
                values: BTreeMap::from([(
                    LoanSetRepresentabilityField::PrincipalRequested,
                    "10.5",
                )]),
            },
            base_inputs(),
            |_| {
                Some(TestBroker {
                    owner: "broker-owner",
                    pseudo_account: "broker-pseudo",
                    vault_id: "vault-id",
                })
            },
            |_| Some("borrower-sle"),
            |_| {
                Some(TestVault {
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
            &TestTx {
                broker_id: "broker-id",
                account: "borrower",
                counterparty: None,
                values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "10")]),
            },
            base_inputs(),
            |_| {
                Some(TestBroker {
                    owner: "broker-owner",
                    pseudo_account: "broker-pseudo",
                    vault_id: "vault-id",
                })
            },
            |_| Some("borrower-sle"),
            |_| {
                Some(TestVault {
                    assets_maximum: 100,
                    assets_total: 10,
                    pseudo_account: "vault-pseudo",
                    asset: "USD",
                })
            },
            |_, _| true,
            |_| protocol::Ter::TER_NO_RIPPLE,
            |_, _| panic!("canAddHolding failure should skip frozen checks"),
            |_, _| panic!("canAddHolding failure should skip deep-frozen checks"),
        );
        let missing_vault = run_loan_set_preclaim(
            &TestTx {
                broker_id: "broker-id",
                account: "borrower",
                counterparty: None,
                values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "10")]),
            },
            base_inputs(),
            |_| {
                Some(TestBroker {
                    owner: "broker-owner",
                    pseudo_account: "broker-pseudo",
                    vault_id: "vault-id",
                })
            },
            |_| Some("borrower-sle"),
            |_| None::<TestVault>,
            |_, _| true,
            |_| protocol::Ter::TES_SUCCESS,
            |_, _| panic!("missing vault should skip frozen checks"),
            |_, _| panic!("missing vault should skip deep-frozen checks"),
        );

        assert_eq!(representability_failure, protocol::Ter::TEC_PRECISION_LOSS);
        assert_eq!(holding_failure, protocol::Ter::TER_NO_RIPPLE);
        assert_eq!(missing_vault, protocol::Ter::TEF_BAD_LEDGER);
    }
}
