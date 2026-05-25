//! Higher loaded-state helper for the LoanSet transactor after the front
//! ledger state is loaded and before the later debt, cover, and transfer
//! stages begin.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - reading `AssetsAvailable`, then `AssetsTotal`, then
//!   `getAssetsTotalScale(...)`,
//! - running the landed pre-guarded-transfer property/state derivation from
//!   the loaded broker management-fee rate,
//! - reading `AssetsMaximum` only after that derived state exists, and
//! - delegating into the landed guarded-transfer shell with the first failing
//!   `TER` returned unchanged.

use std::{fmt::Display, ops::Sub};

use protocol::Ter;

use crate::loan_set_do_apply_pre_guarded_transfer::load_loan_set_do_apply_pre_guarded_transfer_derived;
use crate::{
    LoanSetDoApplyPreGuardedTransferProperties, LoanSetDoApplyPreGuardedTransferState,
    LoanSetDoApplyPreGuardedTransferTx, LoanSetRepresentabilityField,
    run_loan_set_do_apply_guarded_transfer,
};

pub trait LoanSetDoApplyLoadedPreGuardedTransferBroker {
    type ManagementFeeRate: Copy;

    fn management_fee_rate(&self) -> Self::ManagementFeeRate;
}

pub trait LoanSetDoApplyLoadedPreGuardedTransferVault {
    type Amount;

    fn assets_available(&self) -> &Self::Amount;
    fn assets_total(&self) -> &Self::Amount;
    fn assets_maximum(&self) -> &Self::Amount;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoanSetDoApplyLoadedPreGuardedTransferDerived<
    'a,
    Amount,
    InterestRate,
    Properties,
    State,
> {
    pub vault_available: &'a Amount,
    pub vault_total: &'a Amount,
    pub vault_maximum: &'a Amount,
    pub pre_guarded:
        crate::loan_set_do_apply_pre_guarded_transfer::LoanSetDoApplyPreGuardedTransferDerived<
            'a,
            Amount,
            InterestRate,
            Properties,
            State,
        >,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn load_loan_set_do_apply_loaded_pre_guarded_transfer_derived<
    'a,
    Tx,
    Broker,
    Vault,
    Asset,
    Amount,
    InterestRate,
    Properties,
    State,
    ComputeVaultScale,
    ComputeLoanProperties,
    ConstructLoanState,
>(
    tx: &'a Tx,
    broker: &Broker,
    vault: &'a Vault,
    vault_asset: &Asset,
    default_interest_rate: InterestRate,
    default_payment_interval: u32,
    default_payment_total: u32,
    compute_vault_scale: ComputeVaultScale,
    compute_loan_properties: ComputeLoanProperties,
    construct_loan_state: ConstructLoanState,
) -> LoanSetDoApplyLoadedPreGuardedTransferDerived<'a, Amount, InterestRate, Properties, State>
where
    Tx: LoanSetDoApplyPreGuardedTransferTx<Amount = Amount, InterestRate = InterestRate>,
    Broker: LoanSetDoApplyLoadedPreGuardedTransferBroker,
    Vault: LoanSetDoApplyLoadedPreGuardedTransferVault<Amount = Amount>,
    Amount: Clone,
    InterestRate: Copy,
    Properties: LoanSetDoApplyPreGuardedTransferProperties<Amount = Amount>,
    State: LoanSetDoApplyPreGuardedTransferState<Amount = Amount>,
    ComputeVaultScale: FnOnce(&Vault) -> i32,
    ComputeLoanProperties: FnOnce(
        &Asset,
        &Amount,
        InterestRate,
        u32,
        u32,
        Broker::ManagementFeeRate,
        i32,
    ) -> Properties,
    ConstructLoanState: FnOnce(&Amount, &Amount, &Amount) -> State,
{
    let vault_available = vault.assets_available();
    let vault_total = vault.assets_total();
    let vault_scale = compute_vault_scale(vault);

    let pre_guarded = load_loan_set_do_apply_pre_guarded_transfer_derived(
        tx,
        vault_asset,
        vault_scale,
        broker.management_fee_rate(),
        default_interest_rate,
        default_payment_interval,
        default_payment_total,
        compute_loan_properties,
        construct_loan_state,
    );

    let vault_maximum = vault.assets_maximum();

    LoanSetDoApplyLoadedPreGuardedTransferDerived {
        vault_available,
        vault_total,
        vault_maximum,
        pre_guarded,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_do_apply_loaded_pre_guarded_transfer<
    Tx,
    Broker,
    Vault,
    Asset,
    Amount,
    InterestRate,
    CoverRate,
    Properties,
    State,
    ComputeVaultScale,
    ComputeLoanProperties,
    ConstructLoanState,
    CanRepresent,
    CheckLoanGuards,
    ComputeRequiredCover,
    RunTransferAndPostTransfer,
>(
    tx: &Tx,
    broker: &Broker,
    vault: &Vault,
    vault_asset: &Asset,
    default_interest_rate: InterestRate,
    default_payment_interval: u32,
    default_payment_total: u32,
    zero: &Amount,
    debt_maximum: &Amount,
    new_debt_total: &Amount,
    cover_available: &Amount,
    cover_rate_minimum: CoverRate,
    compute_vault_scale: ComputeVaultScale,
    compute_loan_properties: ComputeLoanProperties,
    construct_loan_state: ConstructLoanState,
    can_represent: CanRepresent,
    check_loan_guards: CheckLoanGuards,
    compute_required_cover: ComputeRequiredCover,
    run_transfer_and_post_transfer: RunTransferAndPostTransfer,
) -> Ter
where
    Tx: LoanSetDoApplyPreGuardedTransferTx<Amount = Amount, InterestRate = InterestRate>,
    Broker: LoanSetDoApplyLoadedPreGuardedTransferBroker,
    Vault: LoanSetDoApplyLoadedPreGuardedTransferVault<Amount = Amount>,
    Amount: Clone + Display + PartialEq + PartialOrd + Sub<Output = Amount>,
    InterestRate: Copy + PartialEq,
    CoverRate: Copy,
    Properties: LoanSetDoApplyPreGuardedTransferProperties<Amount = Amount>,
    State: LoanSetDoApplyPreGuardedTransferState<Amount = Amount>,
    ComputeVaultScale: FnOnce(&Vault) -> i32,
    ComputeLoanProperties: FnOnce(
        &Asset,
        &Amount,
        InterestRate,
        u32,
        u32,
        Broker::ManagementFeeRate,
        i32,
    ) -> Properties,
    ConstructLoanState: FnOnce(&Amount, &Amount, &Amount) -> State,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
    CheckLoanGuards: FnOnce(&Asset, &Amount, bool, u32, &Properties) -> Ter,
    ComputeRequiredCover: FnOnce(&Amount, CoverRate) -> Amount,
    RunTransferAndPostTransfer: FnOnce() -> Ter,
{
    let derived = load_loan_set_do_apply_loaded_pre_guarded_transfer_derived(
        tx,
        broker,
        vault,
        vault_asset,
        default_interest_rate,
        default_payment_interval,
        default_payment_total,
        compute_vault_scale,
        compute_loan_properties,
        construct_loan_state,
    );

    run_loan_set_do_apply_guarded_transfer(
        tx,
        vault_asset,
        derived.pre_guarded.principal_requested,
        derived.vault_available,
        derived.vault_maximum,
        derived.vault_total,
        derived.pre_guarded.state.interest_due(),
        derived.pre_guarded.properties.total_value_outstanding(),
        derived.pre_guarded.properties.loan_scale(),
        derived.pre_guarded.interest_rate != default_interest_rate,
        derived.pre_guarded.payment_total,
        &derived.pre_guarded.properties,
        &derived.pre_guarded.computed_values,
        zero,
        debt_maximum,
        new_debt_total,
        cover_available,
        cover_rate_minimum,
        can_represent,
        check_loan_guards,
        compute_required_cover,
        run_transfer_and_post_transfer,
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        LoanSetDoApplyLoadedPreGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferVault,
        run_loan_set_do_apply_loaded_pre_guarded_transfer,
    };
    use crate::{
        LoanSetDoApplyPreGuardedTransferProperties, LoanSetDoApplyPreGuardedTransferState,
        LoanSetDoApplyPreGuardedTransferTx, LoanSetDoApplyRepresentabilityTx,
        LoanSetRepresentabilityField,
    };

    struct TestTx {
        principal_requested: i64,
        interest_rate: Option<u32>,
        payment_interval: Option<u32>,
        payment_total: Option<u32>,
        values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
    }

    impl LoanSetDoApplyRepresentabilityTx for TestTx {
        type Value = &'static str;

        fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
            self.values.get(&field)
        }
    }

    impl LoanSetDoApplyPreGuardedTransferTx for TestTx {
        type Amount = i64;
        type InterestRate = u32;

        fn principal_requested(&self) -> &Self::Amount {
            &self.principal_requested
        }

        fn interest_rate(&self) -> Option<Self::InterestRate> {
            self.interest_rate
        }

        fn payment_interval(&self) -> Option<u32> {
            self.payment_interval
        }

        fn payment_total(&self) -> Option<u32> {
            self.payment_total
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestProperties {
        loan_scale: i32,
        total_value_outstanding: i64,
        management_fee_due: i64,
        periodic_payment: i64,
    }

    impl LoanSetDoApplyPreGuardedTransferProperties for TestProperties {
        type Amount = i64;

        fn loan_scale(&self) -> i32 {
            self.loan_scale
        }

        fn total_value_outstanding(&self) -> &Self::Amount {
            &self.total_value_outstanding
        }

        fn management_fee_due(&self) -> &Self::Amount {
            &self.management_fee_due
        }

        fn periodic_payment(&self) -> &Self::Amount {
            &self.periodic_payment
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestState {
        interest_due: i64,
    }

    impl LoanSetDoApplyPreGuardedTransferState for TestState {
        type Amount = i64;

        fn interest_due(&self) -> &Self::Amount {
            &self.interest_due
        }
    }

    struct TestBroker {
        management_fee_rate: u16,
        steps: Rc<RefCell<Vec<String>>>,
    }

    impl LoanSetDoApplyLoadedPreGuardedTransferBroker for TestBroker {
        type ManagementFeeRate = u16;

        fn management_fee_rate(&self) -> Self::ManagementFeeRate {
            self.steps
                .borrow_mut()
                .push("management_fee_rate".to_string());
            self.management_fee_rate
        }
    }

    struct TestVault {
        assets_available: i64,
        assets_total: i64,
        assets_maximum: i64,
        steps: Rc<RefCell<Vec<String>>>,
    }

    impl LoanSetDoApplyLoadedPreGuardedTransferVault for TestVault {
        type Amount = i64;

        fn assets_available(&self) -> &Self::Amount {
            self.steps.borrow_mut().push("assets_available".to_string());
            &self.assets_available
        }

        fn assets_total(&self) -> &Self::Amount {
            self.steps.borrow_mut().push("assets_total".to_string());
            &self.assets_total
        }

        fn assets_maximum(&self) -> &Self::Amount {
            self.steps.borrow_mut().push("assets_maximum".to_string());
            &self.assets_maximum
        }
    }

    fn valid_tx() -> TestTx {
        TestTx {
            principal_requested: 100,
            interest_rate: Some(250),
            payment_interval: Some(45),
            payment_total: Some(9),
            values: BTreeMap::from([
                (LoanSetRepresentabilityField::PrincipalRequested, "100"),
                (LoanSetRepresentabilityField::LoanOriginationFee, "5"),
            ]),
        }
    }

    fn valid_properties() -> TestProperties {
        TestProperties {
            loan_scale: 4,
            total_value_outstanding: 1_250,
            management_fee_due: 25,
            periodic_payment: 150,
        }
    }

    #[test]
    fn loan_set_do_apply_loaded_pre_guarded_transfer_uses_current_on_success() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let broker = TestBroker {
            management_fee_rate: 12,
            steps: Rc::clone(&steps),
        };
        let vault = TestVault {
            assets_available: 200,
            assets_total: 100,
            assets_maximum: 500,
            steps: Rc::clone(&steps),
        };

        let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
            &valid_tx(),
            &broker,
            &vault,
            &"USD",
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
            |vault| {
                vault.steps.borrow_mut().push("vault_scale".to_string());
                4
            },
            |asset, principal, interest, interval, total, management_fee_rate, scale| {
                steps.borrow_mut().push(format!(
                    "compute_properties={asset}:{principal}:{interest}:{interval}:{total}:{management_fee_rate}:{scale}"
                ));
                valid_properties()
            },
            |value_outstanding, principal, management_fee_due| {
                steps.borrow_mut().push(format!(
                    "construct_state={value_outstanding}:{principal}:{management_fee_due}"
                ));
                TestState { interest_due: 25 }
            },
            |field, _| {
                steps
                    .borrow_mut()
                    .push(format!("representability={field:?}"));
                true
            },
            |asset, principal, expect_interest, payment_total, properties| {
                steps.borrow_mut().push(format!(
                    "loan_guards={asset}:{principal}:{expect_interest}:{payment_total}:{}",
                    properties.total_value_outstanding
                ));
                Ter::TES_SUCCESS
            },
            |new_debt_total, cover_rate_minimum| {
                steps.borrow_mut().push(format!(
                    "compute_required_cover={new_debt_total}:{cover_rate_minimum}"
                ));
                400_i64
            },
            || {
                steps.borrow_mut().push("transfer_shell".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "assets_available",
                "assets_total",
                "vault_scale",
                "management_fee_rate",
                "compute_properties=USD:100:250:45:9:12:4",
                "construct_state=1250:100:25",
                "assets_maximum",
                "representability=PrincipalRequested",
                "representability=LoanOriginationFee",
                "loan_guards=USD:100:true:9:1250",
                "compute_required_cover=1250:10000",
                "transfer_shell",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_loaded_pre_guarded_transfer_uses_cpp_defaults_before_loan_guards() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let broker = TestBroker {
            management_fee_rate: 12,
            steps: Rc::clone(&steps),
        };
        let vault = TestVault {
            assets_available: 200,
            assets_total: 100,
            assets_maximum: 500,
            steps: Rc::clone(&steps),
        };
        let tx = TestTx {
            principal_requested: 100,
            interest_rate: None,
            payment_interval: None,
            payment_total: None,
            values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "100")]),
        };

        let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
            &tx,
            &broker,
            &vault,
            &"USD",
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
            |vault| {
                vault.steps.borrow_mut().push("vault_scale".to_string());
                4
            },
            |asset, principal, interest, interval, total, management_fee_rate, scale| {
                steps.borrow_mut().push(format!(
                    "compute_properties={asset}:{principal}:{interest}:{interval}:{total}:{management_fee_rate}:{scale}"
                ));
                valid_properties()
            },
            |_, _, _| {
                steps.borrow_mut().push("construct_state".to_string());
                TestState { interest_due: 25 }
            },
            |field, _| {
                steps
                    .borrow_mut()
                    .push(format!("representability={field:?}"));
                true
            },
            |_, _, expect_interest, payment_total, _| {
                steps
                    .borrow_mut()
                    .push(format!("loan_guards={expect_interest}:{payment_total}"));
                Ter::TEC_INTERNAL
            },
            |_, _| {
                steps
                    .borrow_mut()
                    .push("compute_required_cover".to_string());
                400_i64
            },
            || {
                steps.borrow_mut().push("transfer_shell".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "assets_available",
                "assets_total",
                "vault_scale",
                "management_fee_rate",
                "compute_properties=USD:100:0:30:12:12:4",
                "construct_state",
                "assets_maximum",
                "representability=PrincipalRequested",
                "loan_guards=false:12",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_loaded_pre_guarded_transfer_returns_vault_limit_before_representability() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let broker = TestBroker {
            management_fee_rate: 12,
            steps: Rc::clone(&steps),
        };
        let vault = TestVault {
            assets_available: 200,
            assets_total: 100,
            assets_maximum: 120,
            steps: Rc::clone(&steps),
        };

        let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
            &valid_tx(),
            &broker,
            &vault,
            &"USD",
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
            |vault| {
                vault.steps.borrow_mut().push("vault_scale".to_string());
                4
            },
            |_, _, _, _, _, _, _| {
                steps.borrow_mut().push("compute_properties".to_string());
                valid_properties()
            },
            |_, _, _| {
                steps.borrow_mut().push("construct_state".to_string());
                TestState { interest_due: 21 }
            },
            |field, _| {
                steps
                    .borrow_mut()
                    .push(format!("representability={field:?}"));
                true
            },
            |_, _, _, _, _| {
                steps.borrow_mut().push("loan_guards".to_string());
                Ter::TES_SUCCESS
            },
            |_, _| {
                steps
                    .borrow_mut()
                    .push("compute_required_cover".to_string());
                400_i64
            },
            || {
                steps.borrow_mut().push("transfer_shell".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
        assert_eq!(trans_token(result), "tecLIMIT_EXCEEDED");
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "assets_available",
                "assets_total",
                "vault_scale",
                "management_fee_rate",
                "compute_properties",
                "construct_state",
                "assets_maximum",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_loaded_pre_guarded_transfer_returns_precision_loss_after_assets_maximum() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let broker = TestBroker {
            management_fee_rate: 12,
            steps: Rc::clone(&steps),
        };
        let vault = TestVault {
            assets_available: 200,
            assets_total: 100,
            assets_maximum: 500,
            steps: Rc::clone(&steps),
        };

        let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
            &valid_tx(),
            &broker,
            &vault,
            &"USD",
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
            |vault| {
                vault.steps.borrow_mut().push("vault_scale".to_string());
                4
            },
            |_, _, _, _, _, _, _| {
                steps.borrow_mut().push("compute_properties".to_string());
                valid_properties()
            },
            |_, _, _| {
                steps.borrow_mut().push("construct_state".to_string());
                TestState { interest_due: 25 }
            },
            |field, _| {
                steps
                    .borrow_mut()
                    .push(format!("representability={field:?}"));
                field != LoanSetRepresentabilityField::PrincipalRequested
            },
            |_, _, _, _, _| {
                steps.borrow_mut().push("loan_guards".to_string());
                Ter::TES_SUCCESS
            },
            |_, _| {
                steps
                    .borrow_mut()
                    .push("compute_required_cover".to_string());
                400_i64
            },
            || {
                steps.borrow_mut().push("transfer_shell".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_PRECISION_LOSS);
        assert_eq!(trans_token(result), "tecPRECISION_LOSS");
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "assets_available",
                "assets_total",
                "vault_scale",
                "management_fee_rate",
                "compute_properties",
                "construct_state",
                "assets_maximum",
                "representability=PrincipalRequested",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_loaded_pre_guarded_transfer_maps_property_values_into_computed_value_guards()
     {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let broker = TestBroker {
            management_fee_rate: 12,
            steps: Rc::clone(&steps),
        };
        let vault = TestVault {
            assets_available: 200,
            assets_total: 100,
            assets_maximum: 500,
            steps: Rc::clone(&steps),
        };

        let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
            &valid_tx(),
            &broker,
            &vault,
            &"USD",
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
            |vault| {
                vault.steps.borrow_mut().push("vault_scale".to_string());
                4
            },
            |_, _, _, _, _, _, _| {
                steps.borrow_mut().push("compute_properties".to_string());
                TestProperties {
                    periodic_payment: 0,
                    ..valid_properties()
                }
            },
            |_, _, _| {
                steps.borrow_mut().push("construct_state".to_string());
                TestState { interest_due: 25 }
            },
            |field, _| {
                steps
                    .borrow_mut()
                    .push(format!("representability={field:?}"));
                true
            },
            |_, _, _, _, _| {
                steps.borrow_mut().push("loan_guards".to_string());
                Ter::TES_SUCCESS
            },
            |_, _| {
                steps
                    .borrow_mut()
                    .push("compute_required_cover".to_string());
                400_i64
            },
            || {
                steps.borrow_mut().push("transfer_shell".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "assets_available",
                "assets_total",
                "vault_scale",
                "management_fee_rate",
                "compute_properties",
                "construct_state",
                "assets_maximum",
                "representability=PrincipalRequested",
                "representability=LoanOriginationFee",
                "loan_guards",
            ]
        );
    }
}
