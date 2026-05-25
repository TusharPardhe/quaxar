//! Higher loaded-state transfer and post-transfer helper for
//! the LoanSet transactor.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - reusing the already loaded broker, vault, asset, and borrower account
//!   state from the front ledger-state shell,
//! - deriving `originationFee` from the transaction only after the loaded
//!   debt/cover guard path has succeeded,
//! - selecting `preFeeBalance_` versus the loaded borrower ledger balance
//!   using the current `account_ == borrower` rule, and
//! - delegating into the landed borrower-reserve and transfer/post-transfer
//!   shell with the first failing `TER` returned unchanged.

use std::{
    fmt::Display,
    ops::{Add, Sub},
};

use protocol::Ter;

use crate::{
    LoanSetDoApplyLedgerState, LoanSetDoApplyLedgerStateTx,
    LoanSetDoApplyLoadedGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferVault,
    LoanSetDoApplyPreGuardedTransferProperties, LoanSetDoApplyPreGuardedTransferState,
    LoanSetDoApplyPreGuardedTransferTx, LoanSetRepresentabilityField,
    run_loan_set_do_apply_loaded_guarded_transfer,
    run_loan_set_do_apply_transfer_and_post_transfer,
};

pub trait LoanSetDoApplyLoadedTransferAndPostTransferTx:
    LoanSetDoApplyLedgerStateTx + LoanSetDoApplyPreGuardedTransferTx
{
    fn loan_origination_fee(&self) -> Option<&Self::Amount>;
}

pub trait LoanSetDoApplyLoadedTransferAndPostTransferAccountState {
    type Balance;

    fn balance(&self) -> &Self::Balance;
}

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_do_apply_loaded_transfer_and_post_transfer<
    Tx,
    Broker,
    AccountState,
    Vault,
    Asset,
    Amount,
    Balance,
    InterestRate,
    OwnerCount,
    Properties,
    State,
    ComputeVaultScale,
    ComputeLoanProperties,
    ConstructLoanState,
    CanRepresent,
    CheckLoanGuards,
    ComputeRequiredCover,
    IncrementBorrowerOwnerCount,
    ComputeAccountReserve,
    AddBorrowerHolding,
    CheckBorrowerAuth,
    AddOwnerHolding,
    CheckOwnerAuth,
    AccountSendMulti,
    RunPostTransfer,
>(
    tx: &Tx,
    pre_fee_balance: &Balance,
    loaded: &LoanSetDoApplyLedgerState<Broker, AccountState, Vault, Tx::AccountId, Asset>,
    default_interest_rate: InterestRate,
    default_payment_interval: u32,
    default_payment_total: u32,
    zero: &Amount,
    compute_vault_scale: ComputeVaultScale,
    compute_loan_properties: ComputeLoanProperties,
    construct_loan_state: ConstructLoanState,
    can_represent: CanRepresent,
    check_loan_guards: CheckLoanGuards,
    compute_required_cover: ComputeRequiredCover,
    increment_borrower_owner_count: IncrementBorrowerOwnerCount,
    compute_account_reserve: ComputeAccountReserve,
    add_borrower_holding: AddBorrowerHolding,
    check_borrower_auth: CheckBorrowerAuth,
    add_owner_holding: AddOwnerHolding,
    check_owner_auth: CheckOwnerAuth,
    account_send_multi: AccountSendMulti,
    run_post_transfer: RunPostTransfer,
) -> Ter
where
    Tx: LoanSetDoApplyLoadedTransferAndPostTransferTx<Amount = Amount, InterestRate = InterestRate>,
    Tx::AccountId: Eq,
    Broker: LoanSetDoApplyLoadedGuardedTransferBroker<Amount = Amount>,
    Vault: LoanSetDoApplyLoadedPreGuardedTransferVault<Amount = Amount>,
    AccountState: LoanSetDoApplyLoadedTransferAndPostTransferAccountState<Balance = Balance>,
    Amount: Clone + Display + PartialEq + PartialOrd + Add<Output = Amount> + Sub<Output = Amount>,
    Balance: PartialOrd,
    InterestRate: Copy + PartialEq,
    OwnerCount: Copy,
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
    ComputeRequiredCover: FnOnce(&Amount, Broker::CoverRate) -> Amount,
    IncrementBorrowerOwnerCount: FnOnce() -> OwnerCount,
    ComputeAccountReserve: FnOnce(OwnerCount) -> Balance,
    AddBorrowerHolding: FnOnce() -> Ter,
    CheckBorrowerAuth: FnOnce() -> Ter,
    AddOwnerHolding: FnOnce() -> Ter,
    CheckOwnerAuth: FnOnce() -> Ter,
    AccountSendMulti: FnOnce() -> Ter,
    RunPostTransfer: FnOnce() -> Ter,
{
    let account_is_borrower = tx.account() == &loaded.borrower;
    let origination_fee = tx.loan_origination_fee().unwrap_or(zero);

    run_loan_set_do_apply_loaded_guarded_transfer(
        tx,
        &loaded.broker,
        &loaded.vault,
        &loaded.vault_asset,
        default_interest_rate,
        default_payment_interval,
        default_payment_total,
        zero,
        compute_vault_scale,
        compute_loan_properties,
        construct_loan_state,
        can_represent,
        check_loan_guards,
        compute_required_cover,
        || {
            run_loan_set_do_apply_transfer_and_post_transfer(
                account_is_borrower,
                pre_fee_balance,
                loaded.borrower_state.balance(),
                origination_fee,
                zero,
                increment_borrower_owner_count,
                compute_account_reserve,
                add_borrower_holding,
                check_borrower_auth,
                add_owner_holding,
                check_owner_auth,
                account_send_multi,
                run_post_transfer,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        LoanSetDoApplyLoadedTransferAndPostTransferAccountState,
        LoanSetDoApplyLoadedTransferAndPostTransferTx,
        run_loan_set_do_apply_loaded_transfer_and_post_transfer,
    };
    use crate::{
        LoanSetDoApplyLedgerState, LoanSetDoApplyLedgerStateTx,
        LoanSetDoApplyLoadedGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferBroker,
        LoanSetDoApplyLoadedPreGuardedTransferVault, LoanSetDoApplyPreGuardedTransferProperties,
        LoanSetDoApplyPreGuardedTransferState, LoanSetDoApplyPreGuardedTransferTx,
        LoanSetDoApplyRepresentabilityTx, LoanSetRepresentabilityField,
    };

    struct TestTx {
        broker_id: &'static str,
        account: &'static str,
        principal_requested: i64,
        interest_rate: Option<u32>,
        payment_interval: Option<u32>,
        payment_total: Option<u32>,
        loan_origination_fee: Option<i64>,
        values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
    }

    impl LoanSetDoApplyLedgerStateTx for TestTx {
        type BrokerId = &'static str;
        type AccountId = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn account(&self) -> &Self::AccountId {
            &self.account
        }

        fn counterparty(&self) -> Option<&Self::AccountId> {
            None
        }
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

    impl LoanSetDoApplyLoadedTransferAndPostTransferTx for TestTx {
        fn loan_origination_fee(&self) -> Option<&Self::Amount> {
            self.loan_origination_fee.as_ref()
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        management_fee_rate: u32,
        debt_total: i64,
        debt_maximum: i64,
        cover_available: i64,
        cover_rate_minimum: u32,
    }

    impl LoanSetDoApplyLoadedPreGuardedTransferBroker for TestBroker {
        type ManagementFeeRate = u32;

        fn management_fee_rate(&self) -> Self::ManagementFeeRate {
            self.management_fee_rate
        }
    }

    impl LoanSetDoApplyLoadedGuardedTransferBroker for TestBroker {
        type Amount = i64;
        type CoverRate = u32;

        fn debt_total(&self) -> &Self::Amount {
            &self.debt_total
        }

        fn debt_maximum(&self) -> &Self::Amount {
            &self.debt_maximum
        }

        fn cover_available(&self) -> &Self::Amount {
            &self.cover_available
        }

        fn cover_rate_minimum(&self) -> Self::CoverRate {
            self.cover_rate_minimum
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        assets_available: i64,
        assets_total: i64,
        assets_maximum: i64,
    }

    impl LoanSetDoApplyLoadedPreGuardedTransferVault for TestVault {
        type Amount = i64;

        fn assets_available(&self) -> &Self::Amount {
            &self.assets_available
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }

        fn assets_maximum(&self) -> &Self::Amount {
            &self.assets_maximum
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestAccountState {
        balance: i64,
    }

    impl LoanSetDoApplyLoadedTransferAndPostTransferAccountState for TestAccountState {
        type Balance = i64;

        fn balance(&self) -> &Self::Balance {
            &self.balance
        }
    }

    fn test_loaded_state(
        borrower: &'static str,
        borrower_balance: i64,
    ) -> LoanSetDoApplyLedgerState<
        TestBroker,
        TestAccountState,
        TestVault,
        &'static str,
        &'static str,
    > {
        LoanSetDoApplyLedgerState {
            broker: TestBroker {
                management_fee_rate: 5,
                debt_total: 40,
                debt_maximum: 100,
                cover_available: 100,
                cover_rate_minimum: 200,
            },
            broker_owner: "broker-owner",
            broker_owner_state: TestAccountState { balance: 90 },
            vault: TestVault {
                assets_available: 50,
                assets_total: 10,
                assets_maximum: 100,
            },
            vault_pseudo: "vault-pseudo",
            vault_asset: "USD",
            counterparty: "broker-owner",
            borrower,
            borrower_state: TestAccountState {
                balance: borrower_balance,
            },
            broker_pseudo: "broker-pseudo",
            broker_pseudo_state: TestAccountState { balance: 80 },
        }
    }

    #[test]
    fn loan_set_do_apply_loaded_transfer_and_post_transfer_uses_current_on_success() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let result = run_loan_set_do_apply_loaded_transfer_and_post_transfer(
            &TestTx {
                broker_id: "broker-id",
                account: "borrower",
                principal_requested: 10,
                interest_rate: None,
                payment_interval: None,
                payment_total: None,
                loan_origination_fee: Some(2),
                values: BTreeMap::new(),
            },
            &30,
            &test_loaded_state("borrower", 1),
            0,
            30,
            12,
            &0,
            {
                let steps = Rc::clone(&steps);
                move |_| {
                    steps.borrow_mut().push("compute_vault_scale".to_string());
                    2
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_,
                      principal_requested,
                      interest_rate,
                      payment_interval,
                      payment_total,
                      management_fee_rate,
                      vault_scale| {
                    steps.borrow_mut().push(format!(
                        "compute_loan_properties principal={principal_requested} interest_rate={interest_rate} payment_interval={payment_interval} payment_total={payment_total} management_fee_rate={management_fee_rate} vault_scale={vault_scale}"
                    ));
                    TestProperties {
                        loan_scale: 2,
                        total_value_outstanding: 20,
                        management_fee_due: 1,
                        periodic_payment: 3,
                    }
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |value_outstanding, principal_requested, management_fee_due| {
                    steps.borrow_mut().push(format!(
                        "construct_loan_state value_outstanding={value_outstanding} principal={principal_requested} management_fee_due={management_fee_due}"
                    ));
                    TestState { interest_due: 5 }
                }
            },
            |_, _| true,
            {
                let steps = Rc::clone(&steps);
                move |_, _, has_interest, payment_total, _| {
                    steps.borrow_mut().push(format!(
                        "check_loan_guards has_interest={has_interest} payment_total={payment_total}"
                    ));
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |new_debt_total, cover_rate_minimum| {
                    steps.borrow_mut().push(format!(
                        "compute_required_cover new_debt_total={new_debt_total} cover_rate_minimum={cover_rate_minimum}"
                    ));
                    90
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("increment_owner_count".to_string());
                    4
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |owner_count| {
                    steps
                        .borrow_mut()
                        .push(format!("compute_reserve owner_count={owner_count}"));
                    30
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps
                        .borrow_mut()
                        .push("borrower_add_empty_holding".to_string());
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("borrower_require_auth".to_string());
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps
                        .borrow_mut()
                        .push("owner_add_empty_holding".to_string());
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("owner_require_auth".to_string());
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("account_send_multi".to_string());
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move || {
                    steps.borrow_mut().push("post_transfer".to_string());
                    Ter::TES_SUCCESS
                }
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "compute_vault_scale",
                "compute_loan_properties principal=10 interest_rate=0 payment_interval=30 payment_total=12 management_fee_rate=5 vault_scale=2",
                "construct_loan_state value_outstanding=20 principal=10 management_fee_due=1",
                "check_loan_guards has_interest=false payment_total=12",
                "compute_required_cover new_debt_total=55 cover_rate_minimum=200",
                "increment_owner_count",
                "compute_reserve owner_count=4",
                "borrower_add_empty_holding",
                "borrower_require_auth",
                "owner_add_empty_holding",
                "owner_require_auth",
                "account_send_multi",
                "post_transfer",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_loaded_transfer_and_post_transfer_uses_prefee_balance_for_borrower() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let result = run_loan_set_do_apply_loaded_transfer_and_post_transfer(
            &TestTx {
                broker_id: "broker-id",
                account: "borrower",
                principal_requested: 10,
                interest_rate: None,
                payment_interval: None,
                payment_total: None,
                loan_origination_fee: None,
                values: BTreeMap::new(),
            },
            &30,
            &test_loaded_state("borrower", 1),
            0,
            30,
            12,
            &0,
            |_| 2,
            |_, _, _, _, _, _, _| TestProperties {
                loan_scale: 2,
                total_value_outstanding: 20,
                management_fee_due: 1,
                periodic_payment: 3,
            },
            |_, _, _| TestState { interest_due: 5 },
            |_, _| true,
            |_, _, _, _, _| Ter::TES_SUCCESS,
            |_, _| 90,
            || {
                steps.borrow_mut().push("increment_owner_count".to_string());
                4
            },
            |owner_count| {
                steps
                    .borrow_mut()
                    .push(format!("compute_reserve owner_count={owner_count}"));
                30
            },
            || {
                steps
                    .borrow_mut()
                    .push("borrower_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TEC_INSUFFICIENT_RESERVE,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "increment_owner_count",
                "compute_reserve owner_count=4",
                "borrower_add_empty_holding",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_loaded_transfer_and_post_transfer_returns_reserve_failure_from_loaded_borrower_balance()
     {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let result = run_loan_set_do_apply_loaded_transfer_and_post_transfer(
            &TestTx {
                broker_id: "broker-id",
                account: "txn-account",
                principal_requested: 10,
                interest_rate: None,
                payment_interval: None,
                payment_total: None,
                loan_origination_fee: Some(2),
                values: BTreeMap::new(),
            },
            &100,
            &test_loaded_state("borrower", 29),
            0,
            30,
            12,
            &0,
            |_| 2,
            |_, _, _, _, _, _, _| TestProperties {
                loan_scale: 2,
                total_value_outstanding: 20,
                management_fee_due: 1,
                periodic_payment: 3,
            },
            |_, _, _| TestState { interest_due: 5 },
            |_, _| true,
            |_, _, _, _, _| Ter::TES_SUCCESS,
            |_, _| 90,
            || {
                steps.borrow_mut().push("increment_owner_count".to_string());
                4
            },
            |owner_count| {
                steps
                    .borrow_mut()
                    .push(format!("compute_reserve owner_count={owner_count}"));
                30
            },
            || {
                steps
                    .borrow_mut()
                    .push("borrower_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
        assert_eq!(
            steps.borrow().as_slice(),
            ["increment_owner_count", "compute_reserve owner_count=4"]
        );
    }
}
