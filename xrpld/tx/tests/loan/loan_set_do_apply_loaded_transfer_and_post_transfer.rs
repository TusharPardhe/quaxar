use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    LoanSetDoApplyLedgerState, LoanSetDoApplyLedgerStateTx,
    LoanSetDoApplyLoadedGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferBroker,
    LoanSetDoApplyLoadedPreGuardedTransferVault,
    LoanSetDoApplyLoadedTransferAndPostTransferAccountState,
    LoanSetDoApplyLoadedTransferAndPostTransferTx, LoanSetDoApplyPreGuardedTransferProperties,
    LoanSetDoApplyPreGuardedTransferState, LoanSetDoApplyPreGuardedTransferTx,
    LoanSetDoApplyRepresentabilityTx, LoanSetRepresentabilityField,
    run_loan_set_do_apply_loaded_transfer_and_post_transfer,
};

struct TestTx {
    broker_id: &'static str,
    account: &'static str,
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
) -> LoanSetDoApplyLedgerState<TestBroker, TestAccountState, TestVault, &'static str, &'static str>
{
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
fn tx_loan_set_do_apply_loaded_transfer_and_post_transfer_uses_current_on_success() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let result = run_loan_set_do_apply_loaded_transfer_and_post_transfer(
        &TestTx {
            broker_id: "broker-id",
            account: "borrower",
            principal_requested: 10,
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
        |_, _, _, _, _, _, _| TestProperties {
            loan_scale: 2,
            total_value_outstanding: 20,
            management_fee_due: 1,
            periodic_payment: 3,
        },
        |_, _, _| TestState { interest_due: 5 },
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
            "compute_vault_scale",
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
fn tx_loan_set_do_apply_loaded_transfer_and_post_transfer_uses_prefee_balance_for_borrower() {
    let result = run_loan_set_do_apply_loaded_transfer_and_post_transfer(
        &TestTx {
            broker_id: "broker-id",
            account: "borrower",
            principal_requested: 10,
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
        || 4,
        |_| 30,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TEC_INSUFFICIENT_RESERVE,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_loaded_transfer_and_post_transfer_returns_reserve_failure_from_loaded_borrower_balance()
 {
    let result = run_loan_set_do_apply_loaded_transfer_and_post_transfer(
        &TestTx {
            broker_id: "broker-id",
            account: "txn-account",
            principal_requested: 10,
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
