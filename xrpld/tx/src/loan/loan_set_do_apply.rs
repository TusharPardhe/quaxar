//! Current Rust helper mirroring the top-level the LoanSet transactor shell.
//!
//! This module preserves the current deterministic composition around:
//!
//! - front broker, broker-owner, vault, borrower, and broker-pseudo loads,
//! - first-failure `tefBAD_LEDGER` return semantics for those loads, and
//! - delegating the successfully loaded facts into the landed loaded
//!   transfer-and-post-transfer shell with the first later failing `TER`
//!   returned unchanged.

use std::{
    fmt::Display,
    ops::{Add, Sub},
};

use protocol::Ter;

use crate::{
    LoanSetDoApplyLedgerStateBroker, LoanSetDoApplyLedgerStateVault,
    LoanSetDoApplyLoadedGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferBroker,
    LoanSetDoApplyLoadedPreGuardedTransferVault,
    LoanSetDoApplyLoadedTransferAndPostTransferAccountState,
    LoanSetDoApplyLoadedTransferAndPostTransferTx, LoanSetDoApplyPreGuardedTransferProperties,
    LoanSetDoApplyPreGuardedTransferState, LoanSetRepresentabilityField,
    load_loan_set_do_apply_ledger_state, run_loan_set_do_apply_loaded_transfer_and_post_transfer,
};

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_do_apply<
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
    ReadBroker,
    ReadVault,
    ReadAccount,
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
    read_broker: ReadBroker,
    read_vault: ReadVault,
    read_account: ReadAccount,
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
    Tx::AccountId: Clone + Eq,
    Broker: LoanSetDoApplyLedgerStateBroker<AccountId = Tx::AccountId>
        + LoanSetDoApplyLoadedGuardedTransferBroker<Amount = Amount>
        + LoanSetDoApplyLoadedPreGuardedTransferBroker,
    Vault: LoanSetDoApplyLedgerStateVault<AccountId = Tx::AccountId, Asset = Asset>
        + LoanSetDoApplyLoadedPreGuardedTransferVault<Amount = Amount>,
    AccountState: LoanSetDoApplyLoadedTransferAndPostTransferAccountState<Balance = Balance>,
    Asset: Clone,
    Amount: Clone + Display + PartialEq + PartialOrd + Add<Output = Amount> + Sub<Output = Amount>,
    Balance: PartialOrd,
    InterestRate: Copy + PartialEq,
    OwnerCount: Copy,
    Properties: LoanSetDoApplyPreGuardedTransferProperties<Amount = Amount>,
    State: LoanSetDoApplyPreGuardedTransferState<Amount = Amount>,
    ReadBroker: FnOnce(&Tx::BrokerId) -> Option<Broker>,
    ReadVault: FnOnce(&<Broker as LoanSetDoApplyLedgerStateBroker>::VaultId) -> Option<Vault>,
    ReadAccount: FnMut(&Tx::AccountId) -> Option<AccountState>,
    ComputeVaultScale: FnOnce(&Vault) -> i32,
    ComputeLoanProperties: FnOnce(
        &Asset,
        &Amount,
        InterestRate,
        u32,
        u32,
        <Broker as LoanSetDoApplyLoadedPreGuardedTransferBroker>::ManagementFeeRate,
        i32,
    ) -> Properties,
    ConstructLoanState: FnOnce(&Amount, &Amount, &Amount) -> State,
    CanRepresent: FnMut(LoanSetRepresentabilityField, &Tx::Value) -> bool,
    CheckLoanGuards: FnOnce(&Asset, &Amount, bool, u32, &Properties) -> Ter,
    ComputeRequiredCover:
        FnOnce(&Amount, <Broker as LoanSetDoApplyLoadedGuardedTransferBroker>::CoverRate) -> Amount,
    IncrementBorrowerOwnerCount: FnOnce() -> OwnerCount,
    ComputeAccountReserve: FnOnce(OwnerCount) -> Balance,
    AddBorrowerHolding: FnOnce() -> Ter,
    CheckBorrowerAuth: FnOnce() -> Ter,
    AddOwnerHolding: FnOnce() -> Ter,
    CheckOwnerAuth: FnOnce() -> Ter,
    AccountSendMulti: FnOnce() -> Ter,
    RunPostTransfer: FnOnce() -> Ter,
{
    let loaded =
        match load_loan_set_do_apply_ledger_state(tx, read_broker, read_vault, read_account) {
            Ok(loaded) => loaded,
            Err(err) => return err.ter(),
        };

    run_loan_set_do_apply_loaded_transfer_and_post_transfer(
        tx,
        pre_fee_balance,
        &loaded,
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
        increment_borrower_owner_count,
        compute_account_reserve,
        add_borrower_holding,
        check_borrower_auth,
        add_owner_holding,
        check_owner_auth,
        account_send_multi,
        run_post_transfer,
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply;
    use crate::{
        LoanSetDoApplyLedgerStateBroker, LoanSetDoApplyLedgerStateTx,
        LoanSetDoApplyLedgerStateVault, LoanSetDoApplyLoadedGuardedTransferBroker,
        LoanSetDoApplyLoadedPreGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferVault,
        LoanSetDoApplyLoadedTransferAndPostTransferAccountState,
        LoanSetDoApplyLoadedTransferAndPostTransferTx, LoanSetDoApplyPreGuardedTransferProperties,
        LoanSetDoApplyPreGuardedTransferState, LoanSetDoApplyPreGuardedTransferTx,
        LoanSetDoApplyRepresentabilityTx, LoanSetRepresentabilityField,
    };

    struct TestTx {
        broker_id: &'static str,
        account: &'static str,
        counterparty: Option<&'static str>,
        principal_requested: i64,
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
            self.counterparty.as_ref()
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
            None
        }

        fn payment_interval(&self) -> Option<u32> {
            None
        }

        fn payment_total(&self) -> Option<u32> {
            None
        }
    }

    impl LoanSetDoApplyLoadedTransferAndPostTransferTx for TestTx {
        fn loan_origination_fee(&self) -> Option<&Self::Amount> {
            self.loan_origination_fee.as_ref()
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        vault_id: &'static str,
        account: &'static str,
        management_fee_rate: u32,
        debt_total: i64,
        debt_maximum: i64,
        cover_available: i64,
        cover_rate_minimum: u32,
    }

    impl LoanSetDoApplyLedgerStateBroker for TestBroker {
        type AccountId = &'static str;
        type VaultId = &'static str;

        fn owner(&self) -> &Self::AccountId {
            &self.owner
        }

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }

        fn account(&self) -> &Self::AccountId {
            &self.account
        }
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
        account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        assets_maximum: i64,
    }

    impl LoanSetDoApplyLedgerStateVault for TestVault {
        type AccountId = &'static str;
        type Asset = &'static str;

        fn account(&self) -> &Self::AccountId {
            &self.account
        }

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }
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

    fn test_broker() -> TestBroker {
        TestBroker {
            owner: "broker-owner",
            vault_id: "vault-id",
            account: "broker-pseudo",
            management_fee_rate: 5,
            debt_total: 40,
            debt_maximum: 100,
            cover_available: 100,
            cover_rate_minimum: 200,
        }
    }

    fn test_vault() -> TestVault {
        TestVault {
            account: "vault-pseudo",
            asset: "USD",
            assets_available: 50,
            assets_total: 10,
            assets_maximum: 100,
        }
    }

    #[test]
    fn loan_set_do_apply_uses_current_on_success() {
        let steps = Rc::new(RefCell::new(Vec::new()));

        let result = run_loan_set_do_apply(
            &TestTx {
                broker_id: "broker-id",
                account: "borrower",
                counterparty: None,
                principal_requested: 10,
                loan_origination_fee: Some(2),
                values: BTreeMap::new(),
            },
            &30,
            {
                let steps = Rc::clone(&steps);
                move |broker_id| {
                    steps.borrow_mut().push(format!("read_broker {broker_id}"));
                    Some(test_broker())
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |vault_id| {
                    steps.borrow_mut().push(format!("read_vault {vault_id}"));
                    Some(test_vault())
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |account| {
                    steps.borrow_mut().push(format!("read_account {account}"));
                    Some(TestAccountState {
                        balance: match *account {
                            "broker-owner" => 90,
                            "borrower" => 1,
                            "broker-pseudo" => 80,
                            _ => 0,
                        },
                    })
                }
            },
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
                move |_, _, _, _, _, _, _| {
                    steps
                        .borrow_mut()
                        .push("compute_loan_properties".to_string());
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
                move |_, _, _| {
                    steps.borrow_mut().push("construct_loan_state".to_string());
                    TestState { interest_due: 5 }
                }
            },
            |_, _| true,
            {
                let steps = Rc::clone(&steps);
                move |_, _, _, _, _| {
                    steps.borrow_mut().push("check_loan_guards".to_string());
                    Ter::TES_SUCCESS
                }
            },
            {
                let steps = Rc::clone(&steps);
                move |_, _| {
                    steps
                        .borrow_mut()
                        .push("compute_required_cover".to_string());
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
                move |_| {
                    steps.borrow_mut().push("compute_reserve".to_string());
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
                "read_broker broker-id",
                "read_account broker-owner",
                "read_vault vault-id",
                "read_account borrower",
                "read_account broker-pseudo",
                "compute_vault_scale",
                "compute_loan_properties",
                "construct_loan_state",
                "check_loan_guards",
                "compute_required_cover",
                "increment_owner_count",
                "compute_reserve",
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
    fn loan_set_do_apply_returns_bad_ledger_when_broker_missing() {
        let result = run_loan_set_do_apply(
            &TestTx {
                broker_id: "missing-broker",
                account: "borrower",
                counterparty: None,
                principal_requested: 10,
                loan_origination_fee: Some(2),
                values: BTreeMap::new(),
            },
            &30,
            |_| None::<TestBroker>,
            |_| Some(test_vault()),
            |_| Some(TestAccountState { balance: 1 }),
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
            || 4,
            |_| 30,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(result), "tefBAD_LEDGER");
    }

    #[test]
    fn loan_set_do_apply_returns_loaded_reserve_failure_unchanged() {
        let result = run_loan_set_do_apply(
            &TestTx {
                broker_id: "broker-id",
                account: "txn-account",
                counterparty: Some("borrower"),
                principal_requested: 10,
                loan_origination_fee: Some(2),
                values: BTreeMap::new(),
            },
            &100,
            |_| Some(test_broker()),
            |_| Some(test_vault()),
            |account| {
                Some(TestAccountState {
                    balance: match *account {
                        "broker-owner" => 90,
                        "borrower" => 29,
                        "broker-pseudo" => 80,
                        _ => 0,
                    },
                })
            },
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
            || 4,
            |_| 30,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
    }
}
