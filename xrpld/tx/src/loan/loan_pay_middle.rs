//! Current Rust helper mirroring the remaining post-payment middle apply flow
//! inside the LoanPay transactor.
//!
//! This module preserves the the reference implementation order around:
//!
//! - the earlier vault pseudo-account balance sample before post-payment prep,
//! - the post-payment amount/debt/vault transfer facts bundle,
//! - broker debt adjustment,
//! - the first vault update before later balance snapshots,
//! - the pre-transfer snapshot reads,
//! - the composed tail mutation plus transfer,
//! - and the final post-transfer observation/assertion shell.

use protocol::{Ter, is_tes_success};

use crate::loan_pay_broker_debt::{
    LoanPayBrokerDebtAdjustmentSink, LoanPayBrokerDebtDeltaSign,
    compute_loan_pay_broker_debt_facts, run_loan_pay_broker_debt_adjustment,
};
use crate::loan_pay_post_payment_prep::{
    LoanPayPaymentParts as PostPaymentParts, LoanPayPostPaymentPrepFacts,
    LoanPayPostPaymentPrepSink, compute_loan_pay_post_payment_prep,
};
use crate::loan_pay_post_transfer_checks::{
    LoanPayPostTransferChecksFacts, LoanPayPostTransferChecksResult, LoanPayPostTransferChecksSink,
    run_loan_pay_post_transfer_checks,
};
use crate::loan_pay_pre_transfer_snapshot::{
    LoanPayPreTransferSnapshotFacts, LoanPayPreTransferSnapshotResult,
    LoanPayPreTransferSnapshotSink, compute_loan_pay_pre_transfer_snapshot,
};
use crate::loan_pay_tail::{
    LoanPayDoApplyTailFacts, LoanPayDoApplyTailSink, run_loan_pay_do_apply_tail,
};
use crate::{
    LoanPayDoApplyAmountsSink, LoanPayDoApplyBroker, LoanPayDoApplyFrontState, LoanPayDoApplyLoan,
    LoanPayDoApplySink, LoanPayDoApplyVault,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayDoApplyMiddleFacts<AccountId, Amount> {
    pub account: AccountId,
    pub amount: Amount,
    pub zero_amount: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayDoApplyMiddleResult<Asset, Amount> {
    pub post_payment_prep: LoanPayPostPaymentPrepFacts<Amount, Asset>,
    pub pre_transfer_snapshot: LoanPayPreTransferSnapshotResult<Amount>,
    pub post_transfer_checks: LoanPayPostTransferChecksResult<Amount>,
}

pub trait LoanPayDoApplyMiddleSink:
    LoanPayDoApplySink
    + LoanPayDoApplyAmountsSink<
        Vault = <Self as LoanPayDoApplySink>::Vault,
        Asset = <Self as LoanPayDoApplySink>::Asset,
        Amount = <Self as LoanPayDoApplySink>::Amount,
    >
{
}

impl<T> LoanPayDoApplyMiddleSink for T where
    T: LoanPayDoApplySink
        + LoanPayDoApplyAmountsSink<
            Vault = <T as LoanPayDoApplySink>::Vault,
            Asset = <T as LoanPayDoApplySink>::Asset,
            Amount = <T as LoanPayDoApplySink>::Amount,
        >
{
}

struct LoanPayBrokerDebtAdapter<'a, Sink, Broker> {
    sink: &'a mut Sink,
    broker: &'a mut Broker,
}

struct LoanPayTailAdapter<'a, Sink> {
    sink: &'a mut Sink,
    skip_initial_update_vault: bool,
}

impl<Sink> LoanPayBrokerDebtAdjustmentSink
    for LoanPayBrokerDebtAdapter<'_, Sink, <Sink as LoanPayDoApplySink>::Broker>
where
    Sink: LoanPayDoApplyMiddleSink,
{
    type Amount = <Sink as LoanPayDoApplySink>::Amount;
    type Asset = <Sink as LoanPayDoApplySink>::Asset;
    type Scale = i32;

    fn adjust_debt_total(
        &mut self,
        debt_delta: Self::Amount,
        asset: Self::Asset,
        vault_scale: Self::Scale,
    ) {
        self.sink
            .adjust_broker_debt_total(self.broker, &debt_delta, &asset, vault_scale);
    }
}

impl<Sink> LoanPayDoApplyTailSink for LoanPayTailAdapter<'_, Sink>
where
    Sink: LoanPayDoApplyMiddleSink,
{
    type Loan = <Sink as LoanPayDoApplySink>::Loan;
    type Broker = <Sink as LoanPayDoApplySink>::Broker;
    type Vault = <Sink as LoanPayDoApplySink>::Vault;
    type AccountId = <Sink as LoanPayDoApplySink>::AccountId;
    type Asset = <Sink as LoanPayDoApplySink>::Asset;
    type Amount = <Sink as LoanPayDoApplySink>::Amount;
    type VaultId = <Sink as LoanPayDoApplySink>::VaultId;

    fn update_vault(&mut self, vault: &Self::Vault) {
        if self.skip_initial_update_vault {
            self.skip_initial_update_vault = false;
            return;
        }

        self.sink.update_vault(vault);
    }

    fn require_auth(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Ter {
        self.sink.require_auth(account, asset)
    }

    fn broker_payee_balance_for_empty_holding(
        &mut self,
        account: &Self::AccountId,
    ) -> Self::Amount {
        self.sink.broker_payee_balance_for_empty_holding(account)
    }

    fn add_empty_holding(
        &mut self,
        account: &Self::AccountId,
        balance: &Self::Amount,
        asset: &Self::Asset,
    ) -> Ter {
        self.sink.add_empty_holding(account, balance, asset)
    }

    fn account_send_multi(
        &mut self,
        source: &Self::AccountId,
        asset: &Self::Asset,
        outputs: [(Self::AccountId, Self::Amount); 2],
    ) -> Ter {
        let [(vault_pseudo, vault_amount), (broker_payee, broker_amount)] = outputs;

        self.sink.account_send_multi(
            source,
            asset,
            &vault_pseudo,
            &vault_amount,
            &broker_payee,
            &broker_amount,
        )
    }
}

impl<Sink> LoanPayPostPaymentPrepSink for Sink
where
    Sink: LoanPayDoApplyMiddleSink,
{
    type Vault = <Sink as LoanPayDoApplyAmountsSink>::Vault;
    type Asset = <Sink as LoanPayDoApplyAmountsSink>::Asset;
    type Amount = <Sink as LoanPayDoApplyAmountsSink>::Amount;

    fn vault_scale(&mut self, vault: &Self::Vault) -> i32 {
        LoanPayDoApplyAmountsSink::vault_scale(self, vault)
    }

    fn round_to_asset_downward(
        &mut self,
        asset: &Self::Asset,
        value: &Self::Amount,
        scale: i32,
    ) -> Self::Amount {
        LoanPayDoApplyAmountsSink::round_to_asset_downward(self, asset, value, scale)
    }

    fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
        LoanPayDoApplyAmountsSink::asset_is_integral(self, asset)
    }

    fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool {
        LoanPayDoApplyAmountsSink::is_rounded(self, asset, value, scale)
    }
}

impl<Sink> LoanPayPreTransferSnapshotSink for Sink
where
    Sink: LoanPayDoApplyMiddleSink,
{
    type AccountId = <Sink as LoanPayDoApplySink>::AccountId;
    type Asset = <Sink as LoanPayDoApplySink>::Asset;
    type Amount = <Sink as LoanPayDoApplySink>::Amount;

    fn sample_balance(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Self::Amount {
        LoanPayDoApplySink::sample_balance(self, account, asset)
    }
}

impl<Sink> LoanPayPostTransferChecksSink for Sink
where
    Sink: LoanPayDoApplyMiddleSink,
{
    type AccountId = <Sink as LoanPayDoApplySink>::AccountId;
    type Asset = <Sink as LoanPayDoApplySink>::Asset;
    type Amount = <Sink as LoanPayDoApplySink>::Amount;

    fn sample_balance(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Self::Amount {
        LoanPayDoApplySink::sample_balance(self, account, asset)
    }

    fn account_is_asset_issuer(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> bool {
        LoanPayDoApplySink::account_is_asset_issuer(self, account, asset)
    }
}

pub fn run_loan_pay_do_apply_middle<Sink>(
    sink: &mut Sink,
    state: &mut LoanPayDoApplyFrontState<
        <Sink as LoanPayDoApplySink>::Loan,
        <Sink as LoanPayDoApplySink>::Broker,
        <Sink as LoanPayDoApplySink>::Vault,
        <Sink as LoanPayDoApplySink>::AccountId,
        <Sink as LoanPayDoApplySink>::Asset,
        <Sink as LoanPayDoApplySink>::Amount,
    >,
    facts: LoanPayDoApplyMiddleFacts<
        <Sink as LoanPayDoApplySink>::AccountId,
        <Sink as LoanPayDoApplySink>::Amount,
    >,
) -> Result<
    LoanPayDoApplyMiddleResult<
        <Sink as LoanPayDoApplySink>::Asset,
        <Sink as LoanPayDoApplySink>::Amount,
    >,
    Ter,
>
where
    Sink: LoanPayDoApplyMiddleSink,
    <Sink as LoanPayDoApplySink>::Loan:
        LoanPayDoApplyLoan<Asset = <Sink as LoanPayDoApplySink>::Asset>,
    <Sink as LoanPayDoApplySink>::Broker: LoanPayDoApplyBroker<
            AccountId = <Sink as LoanPayDoApplySink>::AccountId,
            VaultId = <Sink as LoanPayDoApplySink>::VaultId,
            Amount = <Sink as LoanPayDoApplySink>::Amount,
            Asset = <Sink as LoanPayDoApplySink>::Asset,
        >,
    <Sink as LoanPayDoApplySink>::Vault: LoanPayDoApplyVault<
            AccountId = <Sink as LoanPayDoApplySink>::AccountId,
            Asset = <Sink as LoanPayDoApplySink>::Asset,
            Amount = <Sink as LoanPayDoApplySink>::Amount,
        >,
    <Sink as LoanPayDoApplySink>::AccountId: Clone + PartialEq,
    <Sink as LoanPayDoApplySink>::Asset: Clone,
    <Sink as LoanPayDoApplySink>::Amount: Clone
        + PartialEq
        + PartialOrd
        + std::ops::Neg<Output = <Sink as LoanPayDoApplySink>::Amount>
        + std::ops::Add<Output = <Sink as LoanPayDoApplySink>::Amount>
        + std::ops::Sub<Output = <Sink as LoanPayDoApplySink>::Amount>,
{
    let assets_available_before = state.vault.assets_available().clone();
    let pseudo_account_balance_before =
        sink.sample_balance(state.vault.pseudo_account(), &state.asset);

    let post_payment_prep = compute_loan_pay_post_payment_prep(
        sink,
        &state.asset,
        &state.vault,
        &PostPaymentParts {
            principal_paid: state.payment_parts.principal_paid.clone(),
            interest_paid: state.payment_parts.interest_paid.clone(),
            fee_paid: state.payment_parts.fee_paid.clone(),
            value_change: state.payment_parts.value_change.clone(),
        },
        &facts.zero_amount,
        &facts.amount,
        state.loan.scale(),
        &assets_available_before,
        state.vault.assets_total(),
    );

    let amount_facts = &post_payment_prep.amount_facts;
    debug_assert!(amount_facts.total_paid_is_positive);
    debug_assert!(amount_facts.paid_parts_sum_matches_outputs);
    debug_assert!(amount_facts.integral_asset_rounding_matches_raw);
    debug_assert!(amount_facts.rounded_amount_is_not_greater_than_raw);
    debug_assert!(amount_facts.debt_amount_is_rounded);
    debug_assert!(amount_facts.rounded_and_broker_not_greater_than_amount);

    let broker_debt_facts = compute_loan_pay_broker_debt_facts(
        LoanPayBrokerDebtDeltaSign::Decrease,
        post_payment_prep
            .broker_debt_facts
            .total_paid_to_vault_for_debt
            .clone(),
        post_payment_prep.broker_debt_facts.asset.clone(),
        post_payment_prep.broker_debt_facts.vault_scale,
    );
    let _broker_debt = run_loan_pay_broker_debt_adjustment(
        &mut LoanPayBrokerDebtAdapter {
            sink,
            broker: &mut state.broker,
        },
        broker_debt_facts,
    );

    let vault_state = &post_payment_prep.vault_state_facts;
    debug_assert_eq!(
        vault_state.duplicate_post_rounding_check_holds,
        vault_state.assets_available_not_greater_than_total
    );

    sink.update_vault(&state.vault);

    let pre_transfer_snapshot = compute_loan_pay_pre_transfer_snapshot(
        sink,
        LoanPayPreTransferSnapshotFacts {
            account: facts.account.clone(),
            vault_pseudo_account: state.vault.pseudo_account().clone(),
            broker_payee: state.broker_payee.clone(),
            asset: state.asset.clone(),
            pseudo_account_balance_before,
        },
    );

    debug_assert!(
        post_payment_prep
            .transfer_delivery_facts
            .amount_covers_outputs
    );

    let transfer = run_loan_pay_do_apply_tail(
        &mut LoanPayTailAdapter {
            sink,
            skip_initial_update_vault: true,
        },
        &facts.account,
        state,
        LoanPayDoApplyTailFacts {
            zero_amount: facts.zero_amount.clone(),
            total_paid_to_vault_rounded: amount_facts.total_paid_to_vault_rounded.clone(),
            total_paid_to_broker: amount_facts.total_paid_to_broker.clone(),
        },
    );
    if !is_tes_success(transfer) {
        return Err(transfer);
    }

    let assets_available_after = state.vault.assets_available().clone();
    let post_transfer_checks = run_loan_pay_post_transfer_checks(
        sink,
        LoanPayPostTransferChecksFacts {
            account: facts.account,
            vault_pseudo_account: state.vault.pseudo_account().clone(),
            broker_payee: state.broker_payee.clone(),
            asset: state.asset.clone(),
            zero_amount: facts.zero_amount,
            assets_available_before,
            pseudo_account_balance_before: pre_transfer_snapshot
                .pseudo_account_balance_before
                .clone(),
            borrower_balance_before: pre_transfer_snapshot.borrower_balance_before.clone(),
            vault_balance_before: pre_transfer_snapshot.vault_balance_before.clone(),
            broker_balance_before: pre_transfer_snapshot.broker_balance_before.clone(),
            assets_available_after,
        },
    );

    let assertion_facts = &post_transfer_checks.assertion_facts;
    debug_assert!(assertion_facts.vault_pseudo_balance_agrees_before);
    debug_assert!(assertion_facts.vault_pseudo_balance_agrees_after);
    debug_assert!(assertion_facts.funds_conserved);
    debug_assert!(assertion_facts.borrower_balance_non_negative);
    debug_assert!(assertion_facts.vault_balance_non_negative);
    debug_assert!(assertion_facts.broker_balance_non_negative);
    debug_assert!(assertion_facts.borrower_balance_decreased_unless_issuer);
    debug_assert!(assertion_facts.vault_balance_did_not_decrease);
    debug_assert!(assertion_facts.broker_balance_did_not_decrease);
    debug_assert!(assertion_facts.vault_or_broker_increased);

    Ok(LoanPayDoApplyMiddleResult {
        post_payment_prep,
        pre_transfer_snapshot,
        post_transfer_checks,
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    use super::{LoanPayDoApplyMiddleFacts, run_loan_pay_do_apply_middle};
    use crate::{
        LoanPayDoApplyAmountsSink, LoanPayDoApplyBroker, LoanPayDoApplyFrontState,
        LoanPayDoApplyLoan, LoanPayDoApplySink, LoanPayDoApplyVault, LoanPayPaymentParts,
    };
    use protocol::Ter;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        scale: i32,
        impaired: bool,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyLoan for TestLoan {
        type BrokerId = &'static str;
        type Asset = &'static str;

        fn broker_id(&self) -> &Self::BrokerId {
            &self.broker_id
        }

        fn scale(&self) -> i32 {
            self.scale
        }

        fn is_impaired(&self) -> bool {
            self.impaired
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_loan_asset");
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestBroker {
        owner: &'static str,
        pseudo_account: &'static str,
        vault_id: &'static str,
        cover_available: i64,
        debt_total: i64,
        cover_rate_minimum: u32,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyBroker for TestBroker {
        type AccountId = &'static str;
        type VaultId = &'static str;
        type Amount = i64;
        type Asset = &'static str;

        fn owner(&self) -> &Self::AccountId {
            &self.owner
        }

        fn pseudo_account(&self) -> &Self::AccountId {
            &self.pseudo_account
        }

        fn vault_id(&self) -> &Self::VaultId {
            &self.vault_id
        }

        fn cover_available(&self) -> &Self::Amount {
            &self.cover_available
        }

        fn debt_total(&self) -> &Self::Amount {
            &self.debt_total
        }

        fn cover_rate_minimum(&self) -> u32 {
            self.cover_rate_minimum
        }

        fn add_cover_available(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_cover_available");
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, delta: Self::Amount) {
            self.steps.borrow_mut().push("adjust_broker_debt_total");
            self.debt_total -= delta;
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_broker_asset");
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        scale: i32,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl LoanPayDoApplyVault for TestVault {
        type AccountId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn pseudo_account(&self) -> &Self::AccountId {
            &self.pseudo_account
        }

        fn asset(&self) -> &Self::Asset {
            &self.asset
        }

        fn assets_available(&self) -> &Self::Amount {
            &self.assets_available
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }

        fn add_assets_available(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_assets_available");
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.steps.borrow_mut().push("add_assets_total");
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, _asset: &Self::Asset) {
            self.steps.borrow_mut().push("associate_vault_asset");
        }
    }

    struct TestSink {
        balances: HashMap<&'static str, i64>,
        steps: Rc<RefCell<Vec<&'static str>>>,
        broker_auth_result: Ter,
        send_result: Ter,
    }

    impl LoanPayDoApplySink for TestSink {
        type Loan = TestLoan;
        type Broker = TestBroker;
        type Vault = TestVault;
        type AccountId = &'static str;
        type BrokerId = &'static str;
        type VaultId = &'static str;
        type Asset = &'static str;
        type Amount = i64;

        fn read_loan(&mut self) -> Option<Self::Loan> {
            unreachable!()
        }

        fn read_broker(&mut self, _broker_id: &Self::BrokerId) -> Option<Self::Broker> {
            unreachable!()
        }

        fn read_vault(&mut self, _vault_id: &Self::VaultId) -> Option<Self::Vault> {
            unreachable!()
        }

        fn compute_required_cover_threshold(
            &mut self,
            _asset: &Self::Asset,
            _debt_total: &Self::Amount,
            _cover_rate_minimum: u32,
            _loan_scale: i32,
        ) -> Self::Amount {
            unreachable!()
        }

        fn broker_owner_is_deep_frozen(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            unreachable!()
        }

        fn broker_owner_requires_auth(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            unreachable!()
        }

        fn check_deep_frozen(&mut self, _account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            unreachable!()
        }

        fn unimpair_loan(
            &mut self,
            _loan: &mut Self::Loan,
            _vault: &Self::Vault,
            _asset: &Self::Asset,
        ) -> Ter {
            unreachable!()
        }

        fn make_payment(
            &mut self,
            _asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            _amount: &Self::Amount,
            _payment_type: crate::LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            unreachable!()
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            unreachable!()
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            unreachable!()
        }

        fn adjust_broker_debt_total(
            &mut self,
            broker: &mut Self::Broker,
            debt_delta: &Self::Amount,
            _asset: &Self::Asset,
            _vault_scale: i32,
        ) {
            self.steps.borrow_mut().push("adjust_broker_debt_total");
            broker.debt_total += *debt_delta;
        }

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.borrow_mut().push("update_vault");
        }

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.borrow_mut().push(match *account {
                "vault-pseudo" => "require_auth_vault",
                "borrower" => "require_auth_broker",
                _ => "require_auth_other",
            });
            if *account == "borrower" {
                self.broker_auth_result
            } else {
                Ter::TES_SUCCESS
            }
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("broker_payee_balance");
            *self.balances.get(account).unwrap_or(&0)
        }

        fn add_empty_holding(
            &mut self,
            _account: &Self::AccountId,
            _balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.borrow_mut().push("add_empty_holding");
            Ter::TES_SUCCESS
        }

        fn account_send_multi(
            &mut self,
            from: &Self::AccountId,
            _asset: &Self::Asset,
            vault_pseudo: &Self::AccountId,
            vault_amount: &Self::Amount,
            broker_payee: &Self::AccountId,
            broker_amount: &Self::Amount,
        ) -> Ter {
            self.steps.borrow_mut().push("account_send_multi");
            *self.balances.entry(*from).or_insert(0) -= *vault_amount + *broker_amount;
            *self.balances.entry(*vault_pseudo).or_insert(0) += *vault_amount;
            *self.balances.entry(*broker_payee).or_insert(0) += *broker_amount;
            self.send_result
        }

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.steps.borrow_mut().push(match *account {
                "vault-pseudo" => "sample_vault_pseudo",
                "borrower" => "sample_borrower",
                _ => "sample_other",
            });
            *self.balances.get(account).unwrap_or(&0)
        }

        fn account_is_asset_issuer(
            &mut self,
            _account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.borrow_mut().push("account_is_issuer");
            false
        }
    }

    impl LoanPayDoApplyAmountsSink for TestSink {
        type Vault = TestVault;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, vault: &Self::Vault) -> i32 {
            self.steps.borrow_mut().push("vault_scale");
            vault.scale
        }

        fn round_to_asset_downward(
            &mut self,
            _asset: &Self::Asset,
            value: &Self::Amount,
            _scale: i32,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("round_to_asset_downward");
            *value
        }

        fn asset_is_integral(&mut self, _asset: &Self::Asset) -> bool {
            self.steps.borrow_mut().push("asset_is_integral");
            true
        }

        fn is_rounded(&mut self, _asset: &Self::Asset, _value: &Self::Amount, _scale: i32) -> bool {
            self.steps.borrow_mut().push("is_rounded");
            true
        }
    }

    fn make_state(
        steps: &Rc<RefCell<Vec<&'static str>>>,
    ) -> LoanPayDoApplyFrontState<TestLoan, TestBroker, TestVault, &'static str, &'static str, i64>
    {
        LoanPayDoApplyFrontState {
            loan: TestLoan {
                broker_id: "broker",
                scale: 6,
                impaired: false,
                steps: Rc::clone(steps),
            },
            broker: TestBroker {
                owner: "owner",
                pseudo_account: "broker-pseudo",
                vault_id: "vault",
                cover_available: 10,
                debt_total: 100,
                cover_rate_minimum: 0,
                steps: Rc::clone(steps),
            },
            vault: TestVault {
                pseudo_account: "vault-pseudo",
                asset: "USD",
                assets_available: 10,
                assets_total: 50,
                scale: 6,
                steps: Rc::clone(steps),
            },
            asset: "USD",
            broker_payee: "borrower",
            send_broker_fee_to_owner: true,
            payment_type: crate::LoanPayPaymentType::Regular,
            payment_parts: LoanPayPaymentParts {
                principal_paid: 7,
                interest_paid: 3,
                fee_paid: 2,
                value_change: 5,
            },
        }
    }

    #[test]
    fn loan_pay_middle_runs_cpp_post_payment_order() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut state = make_state(&steps);
        let mut sink = TestSink {
            balances: HashMap::from([("vault-pseudo", 10), ("borrower", 40)]),
            steps: Rc::clone(&steps),
            broker_auth_result: Ter::TES_SUCCESS,
            send_result: Ter::TES_SUCCESS,
        };

        let result = run_loan_pay_do_apply_middle(
            &mut sink,
            &mut state,
            LoanPayDoApplyMiddleFacts {
                account: "borrower",
                amount: 20,
                zero_amount: 0,
            },
        )
        .expect("success");

        assert_eq!(
            steps.borrow().as_slice(),
            &[
                "sample_vault_pseudo",
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
                "adjust_broker_debt_total",
                "update_vault",
                "sample_borrower",
                "sample_vault_pseudo",
                "sample_borrower",
                "add_assets_available",
                "add_assets_total",
                "associate_loan_asset",
                "associate_broker_asset",
                "associate_vault_asset",
                "require_auth_vault",
                "broker_payee_balance",
                "add_empty_holding",
                "require_auth_broker",
                "account_send_multi",
                "sample_vault_pseudo",
                "sample_borrower",
                "sample_vault_pseudo",
                "sample_borrower",
                "account_is_issuer",
            ]
        );
        assert_eq!(
            result
                .post_payment_prep
                .broker_debt_facts
                .total_paid_to_vault_for_debt,
            5
        );
        assert_eq!(result.pre_transfer_snapshot.borrower_balance_before, 40);
        assert!(
            result
                .post_transfer_checks
                .assertion_facts
                .all_assertions_hold
        );
    }

    #[test]
    fn loan_pay_middle_returns_tail_auth_failure_unchanged() {
        let steps = Rc::new(RefCell::new(Vec::new()));
        let mut state = make_state(&steps);
        let mut sink = TestSink {
            balances: HashMap::from([("vault-pseudo", 10), ("borrower", 40)]),
            steps: Rc::clone(&steps),
            broker_auth_result: Ter::TER_NO_AUTH,
            send_result: Ter::TES_SUCCESS,
        };

        let result = run_loan_pay_do_apply_middle(
            &mut sink,
            &mut state,
            LoanPayDoApplyMiddleFacts {
                account: "borrower",
                amount: 20,
                zero_amount: 0,
            },
        );

        assert_eq!(result, Err(Ter::TER_NO_AUTH));
    }
}
