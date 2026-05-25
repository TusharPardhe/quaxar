//! Higher pre-guarded-transfer helper for the LoanSet transactor after the
//! front ledger state is loaded.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - reading `PrincipalRequested` from the transaction first,
//! - defaulting `InterestRate`, `PaymentInterval`, and `PaymentTotal` in the
//!   the reference implementation order,
//! - running the property and loan-state math from those values,
//! - mapping the computed property values into the existing computed-values
//!   guard shell, and
//! - delegating into the landed guarded-transfer shell with the first failing
//!   `TER` returned unchanged.

use std::{fmt::Display, ops::Sub};

use protocol::Ter;

use crate::{
    LoanSetDoApplyComputedValues, LoanSetDoApplyRepresentabilityTx, LoanSetRepresentabilityField,
    run_loan_set_do_apply_guarded_transfer,
};

pub trait LoanSetDoApplyPreGuardedTransferTx: LoanSetDoApplyRepresentabilityTx {
    type Amount;
    type InterestRate: Copy;

    fn principal_requested(&self) -> &Self::Amount;
    fn interest_rate(&self) -> Option<Self::InterestRate>;
    fn payment_interval(&self) -> Option<u32>;
    fn payment_total(&self) -> Option<u32>;
}

pub trait LoanSetDoApplyPreGuardedTransferProperties {
    type Amount;

    fn loan_scale(&self) -> i32;
    fn total_value_outstanding(&self) -> &Self::Amount;
    fn management_fee_due(&self) -> &Self::Amount;
    fn periodic_payment(&self) -> &Self::Amount;
}

pub trait LoanSetDoApplyPreGuardedTransferState {
    type Amount;

    fn interest_due(&self) -> &Self::Amount;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoanSetDoApplyPreGuardedTransferDerived<
    'a,
    Amount,
    InterestRate,
    Properties,
    State,
> {
    pub principal_requested: &'a Amount,
    pub interest_rate: InterestRate,
    pub payment_interval: u32,
    pub payment_total: u32,
    pub properties: Properties,
    pub state: State,
    pub computed_values: LoanSetDoApplyComputedValues<Amount>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn load_loan_set_do_apply_pre_guarded_transfer_derived<
    'a,
    Tx,
    Asset,
    Amount,
    InterestRate,
    ManagementFeeRate,
    Properties,
    State,
    ComputeLoanProperties,
    ConstructLoanState,
>(
    tx: &'a Tx,
    vault_asset: &Asset,
    vault_scale: i32,
    management_fee_rate: ManagementFeeRate,
    default_interest_rate: InterestRate,
    default_payment_interval: u32,
    default_payment_total: u32,
    compute_loan_properties: ComputeLoanProperties,
    construct_loan_state: ConstructLoanState,
) -> LoanSetDoApplyPreGuardedTransferDerived<'a, Amount, InterestRate, Properties, State>
where
    Tx: LoanSetDoApplyPreGuardedTransferTx<Amount = Amount, InterestRate = InterestRate>,
    Amount: Clone,
    InterestRate: Copy,
    Properties: LoanSetDoApplyPreGuardedTransferProperties<Amount = Amount>,
    State: LoanSetDoApplyPreGuardedTransferState<Amount = Amount>,
    ComputeLoanProperties:
        FnOnce(&Asset, &Amount, InterestRate, u32, u32, ManagementFeeRate, i32) -> Properties,
    ConstructLoanState: FnOnce(&Amount, &Amount, &Amount) -> State,
{
    let principal_requested = tx.principal_requested();
    let interest_rate = tx.interest_rate().unwrap_or(default_interest_rate);
    let payment_interval = tx.payment_interval().unwrap_or(default_payment_interval);
    let payment_total = tx.payment_total().unwrap_or(default_payment_total);

    let properties = compute_loan_properties(
        vault_asset,
        principal_requested,
        interest_rate,
        payment_interval,
        payment_total,
        management_fee_rate,
        vault_scale,
    );
    let state = construct_loan_state(
        properties.total_value_outstanding(),
        principal_requested,
        properties.management_fee_due(),
    );

    let computed_values = LoanSetDoApplyComputedValues {
        management_fee_due: properties.management_fee_due().clone(),
        total_value_outstanding: properties.total_value_outstanding().clone(),
        periodic_payment: properties.periodic_payment().clone(),
    };

    LoanSetDoApplyPreGuardedTransferDerived {
        principal_requested,
        interest_rate,
        payment_interval,
        payment_total,
        properties,
        state,
        computed_values,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_do_apply_pre_guarded_transfer<
    Tx,
    Asset,
    Amount,
    InterestRate,
    ManagementFeeRate,
    CoverRate,
    Properties,
    State,
    ComputeLoanProperties,
    ConstructLoanState,
    CanRepresent,
    CheckLoanGuards,
    ComputeRequiredCover,
    RunTransferAndPostTransfer,
>(
    tx: &Tx,
    vault_asset: &Asset,
    vault_available: &Amount,
    vault_maximum: &Amount,
    vault_total: &Amount,
    vault_scale: i32,
    management_fee_rate: ManagementFeeRate,
    default_interest_rate: InterestRate,
    default_payment_interval: u32,
    default_payment_total: u32,
    zero: &Amount,
    debt_maximum: &Amount,
    new_debt_total: &Amount,
    cover_available: &Amount,
    cover_rate_minimum: CoverRate,
    compute_loan_properties: ComputeLoanProperties,
    construct_loan_state: ConstructLoanState,
    can_represent: CanRepresent,
    check_loan_guards: CheckLoanGuards,
    compute_required_cover: ComputeRequiredCover,
    run_transfer_and_post_transfer: RunTransferAndPostTransfer,
) -> Ter
where
    Tx: LoanSetDoApplyPreGuardedTransferTx<Amount = Amount, InterestRate = InterestRate>,
    Amount: Clone + Display + PartialEq + PartialOrd + Sub<Output = Amount>,
    InterestRate: Copy + PartialEq,
    CoverRate: Copy,
    Properties: LoanSetDoApplyPreGuardedTransferProperties<Amount = Amount>,
    State: LoanSetDoApplyPreGuardedTransferState<Amount = Amount>,
    ComputeLoanProperties:
        FnOnce(&Asset, &Amount, InterestRate, u32, u32, ManagementFeeRate, i32) -> Properties,
    ConstructLoanState: FnOnce(&Amount, &Amount, &Amount) -> State,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
    CheckLoanGuards: FnOnce(&Asset, &Amount, bool, u32, &Properties) -> Ter,
    ComputeRequiredCover: FnOnce(&Amount, CoverRate) -> Amount,
    RunTransferAndPostTransfer: FnOnce() -> Ter,
{
    let derived = load_loan_set_do_apply_pre_guarded_transfer_derived(
        tx,
        vault_asset,
        vault_scale,
        management_fee_rate,
        default_interest_rate,
        default_payment_interval,
        default_payment_total,
        compute_loan_properties,
        construct_loan_state,
    );

    run_loan_set_do_apply_guarded_transfer(
        tx,
        vault_asset,
        derived.principal_requested,
        vault_available,
        vault_maximum,
        vault_total,
        derived.state.interest_due(),
        derived.properties.total_value_outstanding(),
        derived.properties.loan_scale(),
        derived.interest_rate != default_interest_rate,
        derived.payment_total,
        &derived.properties,
        &derived.computed_values,
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
    use std::{cell::RefCell, collections::BTreeMap};

    use protocol::{Ter, trans_token};

    use super::{
        LoanSetDoApplyPreGuardedTransferProperties, LoanSetDoApplyPreGuardedTransferState,
        LoanSetDoApplyPreGuardedTransferTx, load_loan_set_do_apply_pre_guarded_transfer_derived,
        run_loan_set_do_apply_pre_guarded_transfer,
    };
    use crate::{
        LoanSetDoApplyComputedValues, LoanSetDoApplyRepresentabilityTx,
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
    fn loan_set_do_apply_pre_guarded_transfer_derived_uses_current() {
        let steps = RefCell::new(Vec::new());
        let tx = valid_tx();

        let derived = load_loan_set_do_apply_pre_guarded_transfer_derived(
            &tx,
            &"USD",
            4,
            12_u16,
            0_u32,
            30,
            12,
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
        );

        assert_eq!(derived.principal_requested, &100_i64);
        assert_eq!(derived.interest_rate, 250_u32);
        assert_eq!(derived.payment_interval, 45);
        assert_eq!(derived.payment_total, 9);
        assert_eq!(derived.state.interest_due(), &25_i64);
        assert_eq!(
            derived.computed_values,
            LoanSetDoApplyComputedValues {
                management_fee_due: 25,
                total_value_outstanding: 1_250,
                periodic_payment: 150,
            }
        );
        assert_eq!(
            steps.into_inner(),
            vec![
                "compute_properties=USD:100:250:45:9:12:4",
                "construct_state=1250:100:25",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_pre_guarded_transfer_derived_uses_cpp_defaults() {
        let tx = TestTx {
            principal_requested: 100,
            interest_rate: None,
            payment_interval: None,
            payment_total: None,
            values: BTreeMap::new(),
        };

        let derived = load_loan_set_do_apply_pre_guarded_transfer_derived(
            &tx,
            &"USD",
            4,
            12_u16,
            0_u32,
            30,
            12,
            |_, _, interest, interval, total, _, _| {
                assert_eq!(interest, 0_u32);
                assert_eq!(interval, 30);
                assert_eq!(total, 12);
                valid_properties()
            },
            |_, _, _| TestState { interest_due: 25 },
        );

        assert_eq!(derived.interest_rate, 0_u32);
        assert_eq!(derived.payment_interval, 30);
        assert_eq!(derived.payment_total, 12);
    }

    #[test]
    fn loan_set_do_apply_pre_guarded_transfer_uses_current_on_success() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_pre_guarded_transfer(
            &valid_tx(),
            &"USD",
            &200_i64,
            &500_i64,
            &100_i64,
            4,
            12_u16,
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
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
            steps.into_inner(),
            vec![
                "compute_properties=USD:100:250:45:9:12:4",
                "construct_state=1250:100:25",
                "representability=PrincipalRequested",
                "representability=LoanOriginationFee",
                "loan_guards=USD:100:true:9:1250",
                "compute_required_cover=1250:10000",
                "transfer_shell",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_pre_guarded_transfer_uses_cpp_defaults_before_loan_guards() {
        let steps = RefCell::new(Vec::new());
        let tx = TestTx {
            principal_requested: 100,
            interest_rate: None,
            payment_interval: None,
            payment_total: None,
            values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "100")]),
        };

        let result = run_loan_set_do_apply_pre_guarded_transfer(
            &tx,
            &"USD",
            &200_i64,
            &500_i64,
            &100_i64,
            4,
            12_u16,
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
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
            steps.into_inner(),
            vec![
                "compute_properties=USD:100:0:30:12:12:4",
                "construct_state",
                "representability=PrincipalRequested",
                "loan_guards=false:12",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_pre_guarded_transfer_returns_precision_loss_before_loan_guards() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_pre_guarded_transfer(
            &valid_tx(),
            &"USD",
            &200_i64,
            &500_i64,
            &100_i64,
            4,
            12_u16,
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
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
            steps.into_inner(),
            vec![
                "compute_properties",
                "construct_state",
                "representability=PrincipalRequested",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_pre_guarded_transfer_maps_property_values_into_computed_value_guards() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_pre_guarded_transfer(
            &valid_tx(),
            &"USD",
            &200_i64,
            &500_i64,
            &100_i64,
            4,
            12_u16,
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
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
            steps.into_inner(),
            vec![
                "compute_properties",
                "construct_state",
                "representability=PrincipalRequested",
                "representability=LoanOriginationFee",
                "loan_guards",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_pre_guarded_transfer_returns_vault_limit_before_representability() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_pre_guarded_transfer(
            &valid_tx(),
            &"USD",
            &200_i64,
            &120_i64,
            &100_i64,
            4,
            12_u16,
            0_u32,
            30,
            12,
            &0_i64,
            &2_000_i64,
            &1_250_i64,
            &500_i64,
            10_000_u32,
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
            steps.into_inner(),
            vec!["compute_properties", "construct_state"]
        );
    }
}
