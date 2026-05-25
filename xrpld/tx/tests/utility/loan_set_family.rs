//! Integration tests that pin the owner-facing Rust `LoanSet` family surface
//! to the current composed C++ staging order.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use protocol::Ter;
use tx::{
    LoanSetBaseFeeTx, LoanSetCounterpartySignature, LoanSetDoApplyLedgerStateBroker,
    LoanSetDoApplyLedgerStateTx, LoanSetDoApplyLedgerStateVault,
    LoanSetDoApplyLoadedGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferBroker,
    LoanSetDoApplyLoadedPreGuardedTransferVault,
    LoanSetDoApplyLoadedTransferAndPostTransferAccountState,
    LoanSetDoApplyLoadedTransferAndPostTransferTx, LoanSetDoApplyPreGuardedTransferProperties,
    LoanSetDoApplyPreGuardedTransferState, LoanSetDoApplyPreGuardedTransferTx,
    LoanSetDoApplyRepresentabilityTx, LoanSetPreclaimBrokerTx, LoanSetPreclaimLoadedBroker,
    LoanSetPreclaimLoadedVault, LoanSetPreclaimPermissionTx, LoanSetPreclaimRepresentabilityTx,
    LoanSetPreclaimVaultLimit, LoanSetPreflightTx, LoanSetRepresentabilityField,
    LoanSetScheduleGuardInputs, LoanSetSignTx, run_loan_set_family_calculate_base_fee,
    run_loan_set_family_do_apply, run_loan_set_family_preclaim, run_loan_set_family_preflight,
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

struct PreflightTx {
    is_inner_batch_txn: bool,
    has_counterparty: bool,
    counterparty_signature: Option<&'static str>,
}

impl LoanSetPreflightTx for PreflightTx {
    type CounterpartySignature = &'static str;

    fn is_inner_batch_txn(&self) -> bool {
        self.is_inner_batch_txn
    }

    fn has_counterparty(&self) -> bool {
        self.has_counterparty
    }

    fn counterparty_signature(&self) -> Option<&Self::CounterpartySignature> {
        self.counterparty_signature.as_ref()
    }
}

#[test]
fn tx_loan_set_family_preflight_preserves_cpp_stage_order() {
    let trace = RefCell::new(Vec::new());

    let result = run_loan_set_family_preflight(
        &PreflightTx {
            is_inner_batch_txn: false,
            has_counterparty: true,
            counterparty_signature: Some("sig"),
        },
        true,
        true,
        true,
        || {
            trace.borrow_mut().push("extra-features");
            true
        },
        |_| {
            trace.borrow_mut().push("preflight1");
            Ter::TES_SUCCESS
        },
        |_| {
            trace.borrow_mut().push("signing-key");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("data-length");
            true
        },
        || {
            trace.borrow_mut().push("loan-service-fee");
            true
        },
        || {
            trace.borrow_mut().push("late-payment-fee");
            true
        },
        || {
            trace.borrow_mut().push("close-payment-fee");
            true
        },
        || {
            trace.borrow_mut().push("principal-requested");
            true
        },
        || {
            trace.borrow_mut().push("loan-origination-fee");
            true
        },
        || {
            trace.borrow_mut().push("interest-rate");
            true
        },
        || {
            trace.borrow_mut().push("overpayment-fee");
            true
        },
        || {
            trace.borrow_mut().push("late-interest-rate");
            true
        },
        || {
            trace.borrow_mut().push("close-interest-rate");
            true
        },
        || {
            trace.borrow_mut().push("overpayment-interest-rate");
            true
        },
        || {
            trace.borrow_mut().push("payment-total");
            true
        },
        || {
            trace.borrow_mut().push("payment-interval");
            true
        },
        || {
            trace.borrow_mut().push("grace-period");
            true
        },
        |_| {
            trace.borrow_mut().push("simulate-keys");
            Ter::TES_SUCCESS
        },
        || {
            trace.borrow_mut().push("broker-id");
            true
        },
        || {
            trace.borrow_mut().push("preflight2");
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        trace.into_inner(),
        vec![
            "extra-features",
            "preflight1",
            "signing-key",
            "data-length",
            "loan-service-fee",
            "late-payment-fee",
            "close-payment-fee",
            "principal-requested",
            "loan-origination-fee",
            "interest-rate",
            "overpayment-fee",
            "late-interest-rate",
            "close-interest-rate",
            "overpayment-interest-rate",
            "payment-total",
            "payment-interval",
            "grace-period",
            "simulate-keys",
            "broker-id",
            "preflight2",
        ]
    );
}

struct PreclaimTx {
    broker_id: &'static str,
    account: &'static str,
    counterparty: Option<&'static str>,
    has_counterparty_signature: bool,
    counterparty_signature: &'static str,
    values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
}

impl LoanSetSignTx for PreclaimTx {
    type AccountId = &'static str;
    type CounterpartySignature = &'static str;

    fn counterparty(&self) -> Option<Self::AccountId> {
        self.counterparty
    }

    fn has_counterparty_signature(&self) -> bool {
        self.has_counterparty_signature
    }

    fn counterparty_signature(&self) -> &Self::CounterpartySignature {
        &self.counterparty_signature
    }
}

impl LoanSetPreclaimBrokerTx for PreclaimTx {
    type BrokerId = &'static str;

    fn broker_id(&self) -> &Self::BrokerId {
        &self.broker_id
    }
}

impl LoanSetPreclaimPermissionTx for PreclaimTx {
    type AccountId = &'static str;

    fn account(&self) -> Self::AccountId {
        self.account
    }

    fn counterparty(&self) -> Option<Self::AccountId> {
        self.counterparty
    }
}

impl LoanSetPreclaimRepresentabilityTx for PreclaimTx {
    type Value = &'static str;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
        self.values.get(&field)
    }
}

#[derive(Clone, Copy)]
struct PreclaimBroker {
    owner: &'static str,
    pseudo_account: &'static str,
    vault_id: &'static str,
}

impl LoanSetPreclaimLoadedBroker for PreclaimBroker {
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
struct PreclaimVault {
    assets_maximum: u32,
    assets_total: u32,
    pseudo_account: &'static str,
    asset: &'static str,
}

impl LoanSetPreclaimVaultLimit for PreclaimVault {
    type Amount = u32;

    fn assets_maximum(&self) -> &Self::Amount {
        &self.assets_maximum
    }

    fn assets_total(&self) -> &Self::Amount {
        &self.assets_total
    }
}

impl LoanSetPreclaimLoadedVault for PreclaimVault {
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
fn tx_loan_set_family_preclaim_preserves_cpp_stage_order() {
    let trace = Rc::new(RefCell::new(Vec::<String>::new()));

    let result = run_loan_set_family_preclaim(
        &PreclaimTx {
            broker_id: "broker-id",
            account: "borrower",
            counterparty: None,
            has_counterparty_signature: true,
            counterparty_signature: "sig",
            values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "10")]),
        },
        false,
        base_inputs(),
        {
            let trace = Rc::clone(&trace);
            move || {
                trace.borrow_mut().push("seq".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move || {
                trace.borrow_mut().push("prior".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move || {
                trace.borrow_mut().push("permission".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move || {
                trace.borrow_mut().push("broker-owner".to_string());
                Some("broker-owner")
            }
        },
        {
            let trace = Rc::clone(&trace);
            move || {
                trace.borrow_mut().push("primary-sign".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |account, signature| {
                trace
                    .borrow_mut()
                    .push(format!("counterparty-sign {account} {signature}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move || {
                trace.borrow_mut().push("base-fee".to_string());
                20_u64
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |fee| {
                trace.borrow_mut().push(format!("fee {fee}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |broker_id| {
                trace.borrow_mut().push(format!("read-broker {broker_id}"));
                Some(PreclaimBroker {
                    owner: "broker-owner",
                    pseudo_account: "broker-pseudo",
                    vault_id: "vault-id",
                })
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |borrower| {
                trace.borrow_mut().push(format!("read-borrower {borrower}"));
                Some(())
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |vault_id| {
                trace.borrow_mut().push(format!("read-vault {vault_id}"));
                Some(PreclaimVault {
                    assets_maximum: 100,
                    assets_total: 10,
                    pseudo_account: "vault-pseudo",
                    asset: "USD",
                })
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |field, value| {
                trace
                    .borrow_mut()
                    .push(format!("representability {} {value}", field.display_name()));
                true
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |asset| {
                trace.borrow_mut().push(format!("can-add-holding {asset}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |account, asset| {
                trace.borrow_mut().push(format!("frozen {account} {asset}"));
                Ter::TES_SUCCESS
            }
        },
        {
            let trace = Rc::clone(&trace);
            move |account, asset| {
                trace
                    .borrow_mut()
                    .push(format!("deep-frozen {account} {asset}"));
                Ter::TES_SUCCESS
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        trace.borrow().as_slice(),
        [
            "seq".to_string(),
            "prior".to_string(),
            "permission".to_string(),
            "primary-sign".to_string(),
            "broker-owner".to_string(),
            "counterparty-sign broker-owner sig".to_string(),
            "base-fee".to_string(),
            "fee 20".to_string(),
            "read-broker broker-id".to_string(),
            "read-borrower borrower".to_string(),
            "read-vault vault-id".to_string(),
            "representability PrincipalRequested 10".to_string(),
            "can-add-holding USD".to_string(),
            "frozen vault-pseudo USD".to_string(),
            "deep-frozen broker-pseudo USD".to_string(),
            "frozen borrower USD".to_string(),
            "deep-frozen broker-owner USD".to_string(),
        ]
    );
}

#[derive(Clone, Copy)]
struct FeeSignature {
    has_signers: bool,
    signers_len: usize,
    has_txn_signature: bool,
}

impl LoanSetCounterpartySignature for FeeSignature {
    fn has_signers(&self) -> bool {
        self.has_signers
    }

    fn signers_len(&self) -> usize {
        self.signers_len
    }

    fn has_txn_signature(&self) -> bool {
        self.has_txn_signature
    }
}

struct FeeTx {
    counterparty_signature: FeeSignature,
}

impl LoanSetBaseFeeTx for FeeTx {
    type CounterpartySignature = FeeSignature;

    fn counterparty_signature(&self) -> &Self::CounterpartySignature {
        &self.counterparty_signature
    }
}

#[test]
fn tx_loan_set_family_calculate_base_fee_reuses_current_base_fee_wrapper() {
    let fee = run_loan_set_family_calculate_base_fee(
        &FeeTx {
            counterparty_signature: FeeSignature {
                has_signers: true,
                signers_len: 3,
                has_txn_signature: false,
            },
        },
        10_u64,
        2_u64,
    );

    assert_eq!(fee, 16);
}

struct ApplyTx {
    broker_id: &'static str,
    account: &'static str,
    counterparty: Option<&'static str>,
    principal_requested: i64,
    loan_origination_fee: Option<i64>,
    values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
}

impl LoanSetDoApplyLedgerStateTx for ApplyTx {
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

impl LoanSetDoApplyRepresentabilityTx for ApplyTx {
    type Value = &'static str;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
        self.values.get(&field)
    }
}

impl LoanSetDoApplyPreGuardedTransferTx for ApplyTx {
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

impl LoanSetDoApplyLoadedTransferAndPostTransferTx for ApplyTx {
    fn loan_origination_fee(&self) -> Option<&Self::Amount> {
        self.loan_origination_fee.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplyBroker {
    owner: &'static str,
    vault_id: &'static str,
    account: &'static str,
    management_fee_rate: u32,
    debt_total: i64,
    debt_maximum: i64,
    cover_available: i64,
    cover_rate_minimum: u32,
}

impl LoanSetDoApplyLedgerStateBroker for ApplyBroker {
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

impl LoanSetDoApplyLoadedPreGuardedTransferBroker for ApplyBroker {
    type ManagementFeeRate = u32;

    fn management_fee_rate(&self) -> Self::ManagementFeeRate {
        self.management_fee_rate
    }
}

impl LoanSetDoApplyLoadedGuardedTransferBroker for ApplyBroker {
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
struct ApplyVault {
    account: &'static str,
    asset: &'static str,
    assets_available: i64,
    assets_total: i64,
    assets_maximum: i64,
}

impl LoanSetDoApplyLedgerStateVault for ApplyVault {
    type AccountId = &'static str;
    type Asset = &'static str;

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

impl LoanSetDoApplyLoadedPreGuardedTransferVault for ApplyVault {
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
struct ApplyAccountState {
    balance: i64,
}

impl LoanSetDoApplyLoadedTransferAndPostTransferAccountState for ApplyAccountState {
    type Balance = i64;

    fn balance(&self) -> &Self::Balance {
        &self.balance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplyProperties {
    loan_scale: i32,
    total_value_outstanding: i64,
    management_fee_due: i64,
    periodic_payment: i64,
}

impl LoanSetDoApplyPreGuardedTransferProperties for ApplyProperties {
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
struct ApplyState {
    interest_due: i64,
}

impl LoanSetDoApplyPreGuardedTransferState for ApplyState {
    type Amount = i64;

    fn interest_due(&self) -> &Self::Amount {
        &self.interest_due
    }
}

fn test_apply_broker() -> ApplyBroker {
    ApplyBroker {
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

fn test_apply_vault() -> ApplyVault {
    ApplyVault {
        account: "vault-pseudo",
        asset: "USD",
        assets_available: 50,
        assets_total: 10,
        assets_maximum: 100,
    }
}

#[test]
fn tx_loan_set_family_do_apply_reuses_current_top_shell_order() {
    let steps = Rc::new(RefCell::new(Vec::new()));

    let result = run_loan_set_family_do_apply(
        &ApplyTx {
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
                Some(test_apply_broker())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |vault_id| {
                steps.borrow_mut().push(format!("read_vault {vault_id}"));
                Some(test_apply_vault())
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |account| {
                steps.borrow_mut().push(format!("read_account {account}"));
                Some(ApplyAccountState {
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
                ApplyProperties {
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
                ApplyState { interest_due: 5 }
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
