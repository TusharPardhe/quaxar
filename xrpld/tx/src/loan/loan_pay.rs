//! Current Rust helper mirroring the top-level the reference implementation metadata,
//! `preflight(...)`, `preclaim(...)`, payment-mode selection, and `doApply()`
//! control flow.
//!
//! This module preserves the current top-level control flow around:
//!
//! - the zero loan-id malformed guard,
//! - the positive-amount malformed guard,
//! - the mutually-exclusive loan-payment flag check,
//! - borrower ownership and optional overpayment permission gates,
//! - the Security 3.1.3 branch that switches overpayment failures from
//!   `temINVALID_FLAG` to `tecNO_PERMISSION`,
//! - the paid-off loan rejection,
//! - the impossible missing-broker and missing-vault fallbacks to
//!   `tefBAD_LEDGER`,
//! - the vault-asset match check,
//! - borrower freeze, vault pseudo deep-freeze, and auth pass-throughs,
//! - the final borrower balance check, and
//! - the current payment-type selection order used by `doApply()`.
//!
//! The landed `doApply()` slice now also covers the current top-level control
//! flow through:
//!
//! - the apply-time loan, broker, and vault load order,
//! - the broker-fee payee selection and fallback pseudo-account deep-freeze
//!   guard,
//! - optional unimpair-before-payment ordering,
//! - `LoanManage::unimpairLoan(...)` passthrough through an explicit helper,
//! - `loanMakePayment(...)`, `view.update(loanSle)`, and the negative-paid-part
//!   guard through an explicit helper,
//! - broker update immediately after the front payment computation,
//! - the current `adjustImpreciseNumber(...)` broker-debt handoff around the
//!   signed debt delta, asset, and vault scale,
//! - the current post-payment vault/broker amount shaping around
//!   `totalPaidToVaultRaw`, `roundToAsset(...)`, `totalPaidToVaultForDebt`,
//!   and `feePaid`,
//! - the current pre-transfer balance snapshot reads before auth and send,
//! - the vault assets-available/assets-total mutation and overflow guard,
//! - the fallback cover increment branch,
//! - the current asset-association order,
//! - the current vault and broker auth plus duplicate-tolerant
//!   add-empty-holding setup around non-zero transfers,
//! - and the final `accountSendMulti(...)` passthrough.
//!
//! The amount-shaping helper now owns the raw/rounded/debt/broker fact
//! derivation through caller-supplied rounding policy. The live ledger
//! objects and `accountSendMulti(...)` transfer implementation remain
//! injected at the call boundary.

use protocol::{NotTec, Ter, is_tes_success};

use crate::loan_pay_amounts::LoanPayDoApplyAmountsSink;
use crate::loan_pay_cover::{
    LoanPayBrokerFeeDestinationFacts, decide_loan_pay_broker_fee_destination,
};
use crate::loan_pay_cover_threshold::{
    LoanPayCoverThresholdSink, compute_loan_pay_cover_threshold_facts,
};
use crate::loan_pay_middle::{LoanPayDoApplyMiddleFacts, run_loan_pay_do_apply_middle};
use crate::loan_pay_payment_apply::{
    LoanPayPaymentApplyFacts, LoanPayPaymentApplySink, run_loan_pay_payment_apply,
};
use crate::loan_pay_unimpair::{LoanPayUnimpairFacts, run_loan_pay_unimpair};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanPayPreflightFacts {
    pub loan_id_is_zero: bool,
    pub amount_is_positive: bool,
    pub tx_specific_flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoanPayPreclaimFacts {
    pub loan_exists: bool,
    pub submitter_is_borrower: bool,
    pub tx_requests_overpayment: bool,
    pub loan_allows_overpayment: bool,
    pub security_fix_3_1_3_enabled: bool,
    pub principal_outstanding_is_zero: bool,
    pub payment_remaining_is_zero: bool,
    pub broker_exists: bool,
    pub vault_exists: bool,
    pub amount_matches_vault_asset: bool,
    pub frozen_result: Ter,
    pub deep_frozen_result: Ter,
    pub require_auth_result: Ter,
    pub balance_is_less_than_amount: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanPayPaymentType {
    Regular,
    Late,
    Full,
    Overpayment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayDoApplyFrontFacts<Amount> {
    pub amount: Amount,
    pub zero_amount: Amount,
    pub tx_requests_late_payment: bool,
    pub tx_requests_full_payment: bool,
    pub tx_requests_overpayment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayDoApplyFacts<AccountId, Amount> {
    pub account: AccountId,
    pub amount: Amount,
    pub zero_amount: Amount,
    pub tx_requests_late_payment: bool,
    pub tx_requests_full_payment: bool,
    pub tx_requests_overpayment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPaymentParts<Amount> {
    pub principal_paid: Amount,
    pub interest_paid: Amount,
    pub fee_paid: Amount,
    pub value_change: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayDoApplyFrontState<Loan, Broker, Vault, AccountId, Asset, Amount> {
    pub loan: Loan,
    pub broker: Broker,
    pub vault: Vault,
    pub asset: Asset,
    pub broker_payee: AccountId,
    pub send_broker_fee_to_owner: bool,
    pub payment_type: LoanPayPaymentType,
    pub payment_parts: LoanPayPaymentParts<Amount>,
}

pub trait LoanPayDoApplyLoan {
    type BrokerId;
    type Asset;

    fn broker_id(&self) -> &Self::BrokerId;
    fn scale(&self) -> i32;
    fn is_impaired(&self) -> bool;
    fn associate_asset(&mut self, asset: &Self::Asset);
}

pub trait LoanPayDoApplyBroker {
    type AccountId;
    type VaultId;
    type Amount;
    type Asset;

    fn owner(&self) -> &Self::AccountId;
    fn pseudo_account(&self) -> &Self::AccountId;
    fn vault_id(&self) -> &Self::VaultId;
    fn cover_available(&self) -> &Self::Amount;
    fn debt_total(&self) -> &Self::Amount;
    fn cover_rate_minimum(&self) -> u32;
    fn add_cover_available(&mut self, amount: Self::Amount);
    fn adjust_debt_total(&mut self, delta: Self::Amount);
    fn associate_asset(&mut self, asset: &Self::Asset);
}

pub trait LoanPayDoApplyVault {
    type AccountId;
    type Asset;
    type Amount;

    fn pseudo_account(&self) -> &Self::AccountId;
    fn asset(&self) -> &Self::Asset;
    fn assets_available(&self) -> &Self::Amount;
    fn assets_total(&self) -> &Self::Amount;
    fn add_assets_available(&mut self, amount: Self::Amount);
    fn add_assets_total(&mut self, amount: Self::Amount);
    fn assets_available_exceeds_total(&self) -> bool;
    fn associate_asset(&mut self, asset: &Self::Asset);
}

pub trait LoanPayDoApplySink {
    type Loan: LoanPayDoApplyLoan<BrokerId = Self::BrokerId, Asset = Self::Asset>;
    type Broker: LoanPayDoApplyBroker<
            AccountId = Self::AccountId,
            VaultId = Self::VaultId,
            Amount = Self::Amount,
            Asset = Self::Asset,
        >;
    type Vault: LoanPayDoApplyVault<AccountId = Self::AccountId, Asset = Self::Asset, Amount = Self::Amount>;
    type AccountId;
    type BrokerId;
    type VaultId;
    type Asset;
    type Amount;

    fn read_loan(&mut self) -> Option<Self::Loan>;
    fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker>;
    fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault>;
    fn compute_required_cover_threshold(
        &mut self,
        asset: &Self::Asset,
        debt_total: &Self::Amount,
        cover_rate_minimum: u32,
        loan_scale: i32,
    ) -> Self::Amount;
    fn broker_owner_is_deep_frozen(&mut self, owner: &Self::AccountId, asset: &Self::Asset)
    -> bool;
    fn broker_owner_requires_auth(&mut self, owner: &Self::AccountId, asset: &Self::Asset) -> bool;
    fn check_deep_frozen(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Ter;
    fn unimpair_loan(
        &mut self,
        loan: &mut Self::Loan,
        vault: &Self::Vault,
        asset: &Self::Asset,
    ) -> Ter;
    fn make_payment(
        &mut self,
        asset: &Self::Asset,
        loan: &mut Self::Loan,
        broker: &mut Self::Broker,
        amount: &Self::Amount,
        payment_type: LoanPayPaymentType,
    ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter>;
    fn update_loan(&mut self, loan: &Self::Loan);
    fn update_broker(&mut self, broker: &Self::Broker);
    fn adjust_broker_debt_total(
        &mut self,
        broker: &mut Self::Broker,
        debt_delta: &Self::Amount,
        asset: &Self::Asset,
        vault_scale: i32,
    );
    fn update_vault(&mut self, vault: &Self::Vault);
    fn require_auth(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Ter;
    fn broker_payee_balance_for_empty_holding(&mut self, account: &Self::AccountId)
    -> Self::Amount;
    fn add_empty_holding(
        &mut self,
        account: &Self::AccountId,
        balance: &Self::Amount,
        asset: &Self::Asset,
    ) -> Ter;
    fn account_send_multi(
        &mut self,
        from: &Self::AccountId,
        asset: &Self::Asset,
        vault_pseudo: &Self::AccountId,
        vault_amount: &Self::Amount,
        broker_payee: &Self::AccountId,
        broker_amount: &Self::Amount,
    ) -> Ter;
    fn sample_balance(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> Self::Amount;
    fn account_is_asset_issuer(&mut self, account: &Self::AccountId, asset: &Self::Asset) -> bool;
}

struct LoanPayCoverThresholdAdapter<'a, Sink> {
    sink: &'a mut Sink,
}

impl<Sink> LoanPayCoverThresholdSink for LoanPayCoverThresholdAdapter<'_, Sink>
where
    Sink: LoanPayDoApplySink,
{
    type Amount = Sink::Amount;
    type Asset = Sink::Asset;
    type CoverRateMinimum = u32;
    type Scale = i32;

    fn compute_required_cover_threshold(
        &mut self,
        asset: &Self::Asset,
        debt_total: &Self::Amount,
        cover_rate_minimum: Self::CoverRateMinimum,
        loan_scale: Self::Scale,
    ) -> Self::Amount {
        self.sink.compute_required_cover_threshold(
            asset,
            debt_total,
            cover_rate_minimum,
            loan_scale,
        )
    }
}

struct LoanPayPaymentApplyAdapter<'a, Sink> {
    sink: &'a mut Sink,
}

impl<Sink> LoanPayPaymentApplySink for LoanPayPaymentApplyAdapter<'_, Sink>
where
    Sink: LoanPayDoApplySink,
{
    type Loan = Sink::Loan;
    type Broker = Sink::Broker;
    type Asset = Sink::Asset;
    type Amount = Sink::Amount;

    fn make_payment(
        &mut self,
        asset: &Self::Asset,
        loan: &mut Self::Loan,
        broker: &mut Self::Broker,
        amount: &Self::Amount,
        payment_type: LoanPayPaymentType,
    ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
        self.sink
            .make_payment(asset, loan, broker, amount, payment_type)
    }

    fn update_loan(&mut self, loan: &Self::Loan) {
        self.sink.update_loan(loan);
    }
}

pub fn run_loan_pay_preflight(facts: LoanPayPreflightFacts) -> NotTec {
    if facts.loan_id_is_zero {
        return Ter::TEM_INVALID;
    }

    if !facts.amount_is_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.tx_specific_flags.count_ones() > 1 {
        return Ter::TEM_INVALID_FLAG;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_pay_preclaim(facts: LoanPayPreclaimFacts) -> Ter {
    if !facts.loan_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.submitter_is_borrower {
        return Ter::TEC_NO_PERMISSION;
    }

    if facts.tx_requests_overpayment && !facts.loan_allows_overpayment {
        return if facts.security_fix_3_1_3_enabled {
            Ter::TEC_NO_PERMISSION
        } else {
            Ter::TEM_INVALID_FLAG
        };
    }

    if facts.payment_remaining_is_zero || facts.principal_outstanding_is_zero {
        return Ter::TEC_KILLED;
    }

    if !facts.broker_exists {
        return Ter::TEF_BAD_LEDGER;
    }

    if !facts.vault_exists {
        return Ter::TEF_BAD_LEDGER;
    }

    if !facts.amount_matches_vault_asset {
        return Ter::TEC_WRONG_ASSET;
    }

    if !is_tes_success(facts.frozen_result) {
        return facts.frozen_result;
    }

    if !is_tes_success(facts.deep_frozen_result) {
        return facts.deep_frozen_result;
    }

    if !is_tes_success(facts.require_auth_result) {
        return facts.require_auth_result;
    }

    if facts.balance_is_less_than_amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    Ter::TES_SUCCESS
}

pub fn run_loan_pay_payment_type(
    tx_requests_late_payment: bool,
    tx_requests_full_payment: bool,
    tx_requests_overpayment: bool,
) -> LoanPayPaymentType {
    if tx_requests_late_payment {
        return LoanPayPaymentType::Late;
    }

    if tx_requests_full_payment {
        return LoanPayPaymentType::Full;
    }

    if tx_requests_overpayment {
        return LoanPayPaymentType::Overpayment;
    }

    LoanPayPaymentType::Regular
}

pub fn run_loan_pay_do_apply_front<Sink>(
    sink: &mut Sink,
    facts: LoanPayDoApplyFrontFacts<Sink::Amount>,
) -> Result<
    LoanPayDoApplyFrontState<
        Sink::Loan,
        Sink::Broker,
        Sink::Vault,
        Sink::AccountId,
        Sink::Asset,
        Sink::Amount,
    >,
    Ter,
>
where
    Sink: LoanPayDoApplySink,
    Sink::AccountId: Clone,
    Sink::Asset: Clone,
    Sink::Amount: Clone + PartialOrd + std::ops::Add<Output = Sink::Amount>,
{
    let mut loan = match sink.read_loan() {
        Some(loan) => loan,
        None => return Err(Ter::TEF_BAD_LEDGER),
    };

    let mut broker = match sink.read_broker(loan.broker_id()) {
        Some(broker) => broker,
        None => return Err(Ter::TEF_BAD_LEDGER),
    };

    let vault = match sink.read_vault(broker.vault_id()) {
        Some(vault) => vault,
        None => return Err(Ter::TEF_BAD_LEDGER),
    };

    let asset = vault.asset().clone();
    let cover_threshold = compute_loan_pay_cover_threshold_facts(
        &mut LoanPayCoverThresholdAdapter { sink },
        broker.cover_available(),
        &asset,
        broker.debt_total(),
        broker.cover_rate_minimum(),
        loan.scale(),
    );
    let cover_is_sufficient = cover_threshold.cover_available_meets_minimum;
    let owner_is_deep_frozen = if cover_is_sufficient {
        sink.broker_owner_is_deep_frozen(broker.owner(), &asset)
    } else {
        false
    };
    let owner_requires_auth = if cover_is_sufficient && !owner_is_deep_frozen {
        sink.broker_owner_requires_auth(broker.owner(), &asset)
    } else {
        false
    };
    let send_broker_fee_to_owner =
        decide_loan_pay_broker_fee_destination(LoanPayBrokerFeeDestinationFacts {
            cover_is_sufficient,
            owner_is_deep_frozen,
            owner_requires_auth,
        });
    let broker_payee = if send_broker_fee_to_owner {
        broker.owner().clone()
    } else {
        broker.pseudo_account().clone()
    };

    if !send_broker_fee_to_owner {
        let deep_frozen = sink.check_deep_frozen(&broker_payee, &asset);
        if !is_tes_success(deep_frozen) {
            return Err(deep_frozen);
        }
    }

    run_loan_pay_unimpair(
        LoanPayUnimpairFacts {
            loan_is_impaired: loan.is_impaired(),
        },
        || {
            let ter = sink.unimpair_loan(&mut loan, &vault, &asset);
            if is_tes_success(ter) {
                Ok(())
            } else {
                Err(ter)
            }
        },
    )?;

    let payment_type = run_loan_pay_payment_type(
        facts.tx_requests_late_payment,
        facts.tx_requests_full_payment,
        facts.tx_requests_overpayment,
    );
    let payment_apply = run_loan_pay_payment_apply(
        &mut LoanPayPaymentApplyAdapter { sink },
        &asset,
        &mut loan,
        &mut broker,
        LoanPayPaymentApplyFacts {
            amount: facts.amount.clone(),
            payment_type,
            zero_amount: facts.zero_amount.clone(),
        },
    )?;

    let payment_parts = payment_apply.payment_parts;
    let payment_validity = payment_apply.payment_validity;
    if !payment_validity.principal_paid_non_negative
        || !payment_validity.interest_paid_non_negative
        || !payment_validity.fee_paid_non_negative
    {
        return Err(Ter::TEC_LIMIT_EXCEEDED);
    }

    debug_assert!(payment_validity.principal_and_interest_positive);
    debug_assert!(payment_validity.aggregate_relation_holds);

    sink.update_broker(&broker);

    Ok(LoanPayDoApplyFrontState {
        loan,
        broker,
        vault,
        asset,
        broker_payee,
        send_broker_fee_to_owner,
        payment_type,
        payment_parts,
    })
}

pub fn run_loan_pay_do_apply<Sink>(
    sink: &mut Sink,
    facts: LoanPayDoApplyFacts<Sink::AccountId, <Sink as LoanPayDoApplySink>::Amount>,
) -> Result<
    LoanPayDoApplyFrontState<
        Sink::Loan,
        Sink::Broker,
        <Sink as LoanPayDoApplySink>::Vault,
        Sink::AccountId,
        <Sink as LoanPayDoApplySink>::Asset,
        <Sink as LoanPayDoApplySink>::Amount,
    >,
    Ter,
>
where
    Sink: LoanPayDoApplySink
        + LoanPayDoApplyAmountsSink<
            Vault = <Sink as LoanPayDoApplySink>::Vault,
            Asset = <Sink as LoanPayDoApplySink>::Asset,
            Amount = <Sink as LoanPayDoApplySink>::Amount,
        >,
    Sink::AccountId: Clone + PartialEq,
    <Sink as LoanPayDoApplySink>::Asset: Clone,
    <Sink as LoanPayDoApplySink>::Amount: PartialEq
        + PartialOrd
        + Clone
        + std::ops::Neg<Output = <Sink as LoanPayDoApplySink>::Amount>
        + std::ops::Add<Output = <Sink as LoanPayDoApplySink>::Amount>
        + std::ops::Sub<Output = <Sink as LoanPayDoApplySink>::Amount>,
{
    let mut state = run_loan_pay_do_apply_front(
        sink,
        LoanPayDoApplyFrontFacts {
            amount: facts.amount.clone(),
            zero_amount: facts.zero_amount.clone(),
            tx_requests_late_payment: facts.tx_requests_late_payment,
            tx_requests_full_payment: facts.tx_requests_full_payment,
            tx_requests_overpayment: facts.tx_requests_overpayment,
        },
    )?;

    run_loan_pay_do_apply_middle(
        sink,
        &mut state,
        LoanPayDoApplyMiddleFacts {
            account: facts.account.clone(),
            amount: facts.amount.clone(),
            zero_amount: facts.zero_amount.clone(),
        },
    )?;

    Ok(state)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use protocol::{Ter, trans_token};

    use super::{
        LoanPayDoApplyBroker, LoanPayDoApplyFacts, LoanPayDoApplyFrontFacts, LoanPayDoApplyLoan,
        LoanPayDoApplySink, LoanPayDoApplyVault, LoanPayPaymentParts, LoanPayPaymentType,
        LoanPayPreclaimFacts, LoanPayPreflightFacts, run_loan_pay_do_apply,
        run_loan_pay_do_apply_front, run_loan_pay_payment_type, run_loan_pay_preclaim,
        run_loan_pay_preflight,
    };
    use crate::loan_pay_amounts::LoanPayDoApplyAmountsSink;

    fn base() -> LoanPayPreclaimFacts {
        LoanPayPreclaimFacts {
            loan_exists: true,
            submitter_is_borrower: true,
            tx_requests_overpayment: false,
            loan_allows_overpayment: true,
            security_fix_3_1_3_enabled: true,
            principal_outstanding_is_zero: false,
            payment_remaining_is_zero: false,
            broker_exists: true,
            vault_exists: true,
            amount_matches_vault_asset: true,
            frozen_result: Ter::TES_SUCCESS,
            deep_frozen_result: Ter::TES_SUCCESS,
            require_auth_result: Ter::TES_SUCCESS,
            balance_is_less_than_amount: false,
        }
    }

    #[test]
    fn loan_pay_preflight_rejects_zero_loan_id() {
        let result = run_loan_pay_preflight(LoanPayPreflightFacts {
            loan_id_is_zero: true,
            amount_is_positive: true,
            tx_specific_flags: 0,
        });

        assert_eq!(result, Ter::TEM_INVALID);
    }

    #[test]
    fn loan_pay_preflight_rejects_non_positive_amount() {
        let result = run_loan_pay_preflight(LoanPayPreflightFacts {
            loan_id_is_zero: false,
            amount_is_positive: false,
            tx_specific_flags: 0,
        });

        assert_eq!(result, Ter::TEM_BAD_AMOUNT);
    }

    #[test]
    fn loan_pay_preflight_rejects_multiple_payment_flags() {
        let result = run_loan_pay_preflight(LoanPayPreflightFacts {
            loan_id_is_zero: false,
            amount_is_positive: true,
            tx_specific_flags: 0x0001_0000 | 0x0002_0000,
        });

        assert_eq!(result, Ter::TEM_INVALID_FLAG);
        assert_eq!(trans_token(result), "temINVALID_FLAG");
    }

    #[test]
    fn loan_pay_preclaim_rejects_missing_loan() {
        assert_eq!(
            run_loan_pay_preclaim(LoanPayPreclaimFacts::default()),
            Ter::TEC_NO_ENTRY
        );
    }

    #[test]
    fn loan_pay_preclaim_rejects_wrong_borrower() {
        assert_eq!(
            run_loan_pay_preclaim(LoanPayPreclaimFacts {
                submitter_is_borrower: false,
                ..base()
            }),
            Ter::TEC_NO_PERMISSION
        );
    }

    #[test]
    fn loan_pay_preclaim_preserves_security_fix_overpayment_switch() {
        let legacy = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            tx_requests_overpayment: true,
            loan_allows_overpayment: false,
            security_fix_3_1_3_enabled: false,
            ..base()
        });
        let fixed = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            tx_requests_overpayment: true,
            loan_allows_overpayment: false,
            security_fix_3_1_3_enabled: true,
            ..base()
        });

        assert_eq!(legacy, Ter::TEM_INVALID_FLAG);
        assert_eq!(fixed, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn loan_pay_preclaim_rejects_paid_off_loan() {
        let payment_remaining = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            payment_remaining_is_zero: true,
            ..base()
        });
        let principal = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            principal_outstanding_is_zero: true,
            ..base()
        });

        assert_eq!(payment_remaining, Ter::TEC_KILLED);
        assert_eq!(principal, Ter::TEC_KILLED);
    }

    #[test]
    fn loan_pay_preclaim_maps_missing_broker_and_vault_to_bad_ledger() {
        let broker = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            broker_exists: false,
            ..base()
        });
        let vault = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            vault_exists: false,
            ..base()
        });

        assert_eq!(broker, Ter::TEF_BAD_LEDGER);
        assert_eq!(vault, Ter::TEF_BAD_LEDGER);
    }

    #[test]
    fn loan_pay_preclaim_rejects_wrong_asset() {
        let result = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            amount_matches_vault_asset: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_WRONG_ASSET);
        assert_eq!(trans_token(result), "tecWRONG_ASSET");
    }

    #[test]
    fn loan_pay_preclaim_returns_freeze_auth_and_balance_failures_unchanged() {
        let frozen = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            frozen_result: Ter::TEC_FROZEN,
            ..base()
        });
        let deep_frozen = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            deep_frozen_result: Ter::TEC_FROZEN,
            ..base()
        });
        let auth = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            require_auth_result: Ter::TER_NO_AUTH,
            ..base()
        });
        let balance = run_loan_pay_preclaim(LoanPayPreclaimFacts {
            balance_is_less_than_amount: true,
            ..base()
        });

        assert_eq!(frozen, Ter::TEC_FROZEN);
        assert_eq!(deep_frozen, Ter::TEC_FROZEN);
        assert_eq!(auth, Ter::TER_NO_AUTH);
        assert_eq!(balance, Ter::TEC_INSUFFICIENT_FUNDS);
    }

    #[test]
    fn loan_pay_preclaim_accepts_valid_payment() {
        assert_eq!(run_loan_pay_preclaim(base()), Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_pay_payment_type_prefers_late_then_full_then_overpayment() {
        assert_eq!(
            run_loan_pay_payment_type(true, true, true),
            LoanPayPaymentType::Late
        );
        assert_eq!(
            run_loan_pay_payment_type(false, true, true),
            LoanPayPaymentType::Full
        );
        assert_eq!(
            run_loan_pay_payment_type(false, false, true),
            LoanPayPaymentType::Overpayment
        );
        assert_eq!(
            run_loan_pay_payment_type(false, false, false),
            LoanPayPaymentType::Regular
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoan {
        broker_id: &'static str,
        scale: i32,
        impaired: bool,
        associated_asset: Option<&'static str>,
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

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
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
        associated_asset: Option<&'static str>,
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
            self.cover_available += amount;
        }

        fn adjust_debt_total(&mut self, delta: Self::Amount) {
            self.debt_total -= delta;
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVault {
        pseudo_account: &'static str,
        asset: &'static str,
        assets_available: i64,
        assets_total: i64,
        associated_asset: Option<&'static str>,
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
            self.assets_available += amount;
        }

        fn add_assets_total(&mut self, amount: Self::Amount) {
            self.assets_total += amount;
        }

        fn assets_available_exceeds_total(&self) -> bool {
            self.assets_available > self.assets_total
        }

        fn associate_asset(&mut self, asset: &Self::Asset) {
            self.associated_asset = Some(*asset);
        }
    }

    struct TestSink {
        loan: Option<TestLoan>,
        broker: Option<TestBroker>,
        vault: Option<TestVault>,
        required_cover: i64,
        owner_is_deep_frozen: bool,
        owner_requires_auth: bool,
        fallback_deep_frozen: Ter,
        unimpair_result: Ter,
        payment_result: Result<LoanPayPaymentParts<i64>, Ter>,
        require_auth_result: Ter,
        add_empty_holding_result: Ter,
        account_send_multi_result: Ter,
        balances: std::collections::HashMap<&'static str, i64>,
        asset_issuer: &'static str,
        expected_from: &'static str,
        expected_broker_payee: &'static str,
        expected_broker_amount: i64,
        steps: Rc<RefCell<Vec<&'static str>>>,
    }

    impl TestSink {
        fn new() -> Self {
            Self {
                loan: Some(TestLoan {
                    broker_id: "broker",
                    scale: 6,
                    impaired: false,
                    associated_asset: None,
                }),
                broker: Some(TestBroker {
                    owner: "owner",
                    pseudo_account: "broker-pseudo",
                    vault_id: "vault",
                    cover_available: 10,
                    debt_total: 0,
                    cover_rate_minimum: 0,
                    associated_asset: None,
                }),
                vault: Some(TestVault {
                    pseudo_account: "vault-pseudo",
                    asset: "USD",
                    assets_available: 10,
                    assets_total: 30,
                    associated_asset: None,
                }),
                required_cover: 10,
                owner_is_deep_frozen: false,
                owner_requires_auth: false,
                fallback_deep_frozen: Ter::TES_SUCCESS,
                unimpair_result: Ter::TES_SUCCESS,
                payment_result: Ok(LoanPayPaymentParts {
                    principal_paid: 10,
                    interest_paid: 3,
                    fee_paid: 1,
                    value_change: 0,
                }),
                require_auth_result: Ter::TES_SUCCESS,
                add_empty_holding_result: Ter::TES_SUCCESS,
                account_send_multi_result: Ter::TES_SUCCESS,
                balances: std::collections::HashMap::from([
                    ("owner", 100),
                    ("vault-pseudo", 10),
                    ("broker-pseudo", 2),
                ]),
                asset_issuer: "issuer",
                expected_from: "owner",
                expected_broker_payee: "owner",
                expected_broker_amount: 1,
                steps: Rc::new(RefCell::new(Vec::new())),
            }
        }
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
            self.steps.borrow_mut().push("read_loan");
            self.loan.clone()
        }

        fn read_broker(&mut self, broker_id: &Self::BrokerId) -> Option<Self::Broker> {
            self.steps.borrow_mut().push("read_broker");
            assert_eq!(*broker_id, "broker");
            self.broker.clone()
        }

        fn read_vault(&mut self, vault_id: &Self::VaultId) -> Option<Self::Vault> {
            self.steps.borrow_mut().push("read_vault");
            assert_eq!(*vault_id, "vault");
            self.vault.clone()
        }

        fn compute_required_cover_threshold(
            &mut self,
            _asset: &Self::Asset,
            _debt_total: &Self::Amount,
            _cover_rate_minimum: u32,
            _loan_scale: i32,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("required_cover");
            self.required_cover
        }

        fn broker_owner_is_deep_frozen(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.borrow_mut().push("owner_deep_frozen");
            self.owner_is_deep_frozen
        }

        fn broker_owner_requires_auth(
            &mut self,
            _owner: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.borrow_mut().push("owner_auth");
            self.owner_requires_auth
        }

        fn check_deep_frozen(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.borrow_mut().push("fallback_deep_frozen");
            assert_eq!(*account, "broker-pseudo");
            self.fallback_deep_frozen
        }

        fn unimpair_loan(
            &mut self,
            loan: &mut Self::Loan,
            _vault: &Self::Vault,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.borrow_mut().push("unimpair");
            loan.impaired = false;
            self.unimpair_result
        }

        fn make_payment(
            &mut self,
            _asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            amount: &Self::Amount,
            payment_type: LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            self.steps.borrow_mut().push("make_payment");
            assert_eq!(*amount, 25);
            assert_eq!(payment_type, LoanPayPaymentType::Full);
            self.payment_result.clone()
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            self.steps.borrow_mut().push("update_loan");
        }

        fn update_broker(&mut self, _broker: &Self::Broker) {
            self.steps.borrow_mut().push("update_broker");
        }

        fn adjust_broker_debt_total(
            &mut self,
            broker: &mut Self::Broker,
            debt_delta: &Self::Amount,
            _asset: &Self::Asset,
            _vault_scale: i32,
        ) {
            self.steps.borrow_mut().push("adjust_broker_debt_total");
            broker.adjust_debt_total(-*debt_delta);
        }

        fn update_vault(&mut self, _vault: &Self::Vault) {
            self.steps.borrow_mut().push("update_vault");
        }

        fn require_auth(&mut self, account: &Self::AccountId, _asset: &Self::Asset) -> Ter {
            self.steps.borrow_mut().push("require_auth");
            assert!(
                *account == "vault-pseudo" || *account == "owner" || *account == "broker-pseudo"
            );
            self.require_auth_result
        }

        fn broker_payee_balance_for_empty_holding(
            &mut self,
            account: &Self::AccountId,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("broker_payee_balance");
            assert_eq!(*account, "owner");
            12
        }

        fn add_empty_holding(
            &mut self,
            account: &Self::AccountId,
            balance: &Self::Amount,
            _asset: &Self::Asset,
        ) -> Ter {
            self.steps.borrow_mut().push("add_empty_holding");
            assert_eq!(*account, "owner");
            assert_eq!(*balance, 12);
            self.add_empty_holding_result
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
            assert_eq!(*from, self.expected_from);
            assert_eq!(*vault_pseudo, "vault-pseudo");
            assert_eq!(*vault_amount, 13);
            assert_eq!(*broker_payee, self.expected_broker_payee);
            assert_eq!(*broker_amount, self.expected_broker_amount);
            let from_balance = self.balances.get_mut(from).expect("from balance");
            *from_balance -= *vault_amount + *broker_amount;
            let vault_balance = self.balances.get_mut(vault_pseudo).expect("vault balance");
            *vault_balance += *vault_amount;
            let broker_balance = self.balances.entry(*broker_payee).or_insert(0);
            *broker_balance += *broker_amount;
            self.account_send_multi_result
        }

        fn sample_balance(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("sample_balance");
            *self.balances.get(account).unwrap_or(&0)
        }

        fn account_is_asset_issuer(
            &mut self,
            account: &Self::AccountId,
            _asset: &Self::Asset,
        ) -> bool {
            self.steps.borrow_mut().push("account_is_asset_issuer");
            *account == self.asset_issuer
        }
    }

    impl LoanPayDoApplyAmountsSink for TestSink {
        type Vault = TestVault;
        type Asset = &'static str;
        type Amount = i64;

        fn vault_scale(&mut self, _vault: &Self::Vault) -> i32 {
            self.steps.borrow_mut().push("vault_scale");
            2
        }

        fn round_to_asset_downward(
            &mut self,
            asset: &Self::Asset,
            value: &Self::Amount,
            scale: i32,
        ) -> Self::Amount {
            self.steps.borrow_mut().push("round_to_asset_downward");
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 13);
            assert_eq!(scale, 2);
            13
        }

        fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool {
            self.steps.borrow_mut().push("asset_is_integral");
            assert_eq!(*asset, "USD");
            false
        }

        fn is_rounded(&mut self, asset: &Self::Asset, value: &Self::Amount, scale: i32) -> bool {
            self.steps.borrow_mut().push("is_rounded");
            assert_eq!(*asset, "USD");
            assert_eq!(*value, 13);
            assert_eq!(scale, 6);
            true
        }
    }

    fn front_facts() -> LoanPayDoApplyFrontFacts<i64> {
        LoanPayDoApplyFrontFacts {
            amount: 25,
            zero_amount: 0,
            tx_requests_late_payment: false,
            tx_requests_full_payment: true,
            tx_requests_overpayment: false,
        }
    }

    fn apply_facts() -> LoanPayDoApplyFacts<&'static str, i64> {
        LoanPayDoApplyFacts {
            account: "owner",
            amount: 25,
            zero_amount: 0,
            tx_requests_late_payment: false,
            tx_requests_full_payment: true,
            tx_requests_overpayment: false,
        }
    }

    #[test]
    fn loan_pay_do_apply_front_maps_missing_objects_to_bad_ledger() {
        let mut missing_loan = TestSink::new();
        missing_loan.loan = None;
        assert_eq!(
            run_loan_pay_do_apply_front(&mut missing_loan, front_facts()),
            Err(Ter::TEF_BAD_LEDGER)
        );

        let mut missing_broker = TestSink::new();
        missing_broker.broker = None;
        assert_eq!(
            run_loan_pay_do_apply_front(&mut missing_broker, front_facts()),
            Err(Ter::TEF_BAD_LEDGER)
        );

        let mut missing_vault = TestSink::new();
        missing_vault.vault = None;
        assert_eq!(
            run_loan_pay_do_apply_front(&mut missing_vault, front_facts()),
            Err(Ter::TEF_BAD_LEDGER)
        );
    }

    #[test]
    fn loan_pay_do_apply_front_falls_back_to_pseudo_and_checks_deep_freeze() {
        let mut sink = TestSink::new();
        sink.required_cover = 11;
        sink.fallback_deep_frozen = Ter::TEC_FROZEN;

        let result = run_loan_pay_do_apply_front(&mut sink, front_facts());

        assert_eq!(result, Err(Ter::TEC_FROZEN));
        assert_eq!(
            sink.steps.borrow().as_slice(),
            &[
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "fallback_deep_frozen",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_front_unimpairs_before_payment_and_updates_after() {
        let mut sink = TestSink::new();
        sink.loan.as_mut().expect("loan").impaired = true;

        let state = run_loan_pay_do_apply_front(&mut sink, front_facts()).expect("success");

        assert_eq!(state.broker_payee, "owner");
        assert!(state.send_broker_fee_to_owner);
        assert_eq!(state.payment_type, LoanPayPaymentType::Full);
        assert_eq!(
            sink.steps.borrow().as_slice(),
            &[
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "unimpair",
                "make_payment",
                "update_loan",
                "update_broker",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_front_rejects_negative_parts_after_loan_update() {
        let mut sink = TestSink::new();
        sink.payment_result = Ok(LoanPayPaymentParts {
            principal_paid: -1,
            interest_paid: 1,
            fee_paid: 0,
            value_change: 0,
        });

        let result = run_loan_pay_do_apply_front(&mut sink, front_facts());

        assert_eq!(result, Err(Ter::TEC_LIMIT_EXCEEDED));
        assert_eq!(
            sink.steps.borrow().as_slice(),
            &[
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "make_payment",
                "update_loan",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_runs_tail_in_current() {
        let mut sink = TestSink::new();

        let state = run_loan_pay_do_apply(&mut sink, apply_facts()).expect("success");

        assert_eq!(state.loan.associated_asset, Some("USD"));
        assert_eq!(state.broker.associated_asset, Some("USD"));
        assert_eq!(state.vault.associated_asset, Some("USD"));
        assert_eq!(
            sink.steps.borrow().as_slice(),
            &[
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "make_payment",
                "update_loan",
                "update_broker",
                "sample_balance",
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
                "adjust_broker_debt_total",
                "update_vault",
                "sample_balance",
                "sample_balance",
                "sample_balance",
                "require_auth",
                "broker_payee_balance",
                "add_empty_holding",
                "require_auth",
                "account_send_multi",
                "sample_balance",
                "sample_balance",
                "sample_balance",
                "sample_balance",
                "account_is_asset_issuer",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_maps_vault_overflow_to_internal() {
        let mut sink = TestSink::new();
        sink.vault.as_mut().expect("vault").assets_available = 10;
        sink.vault.as_mut().expect("vault").assets_total = 10;

        let result = run_loan_pay_do_apply(&mut sink, apply_facts());

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(
            sink.steps.borrow().as_slice(),
            &[
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "make_payment",
                "update_loan",
                "update_broker",
                "sample_balance",
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
                "adjust_broker_debt_total",
                "update_vault",
                "sample_balance",
                "sample_balance",
                "sample_balance",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_passthroughs_non_duplicate_holding_failure() {
        let mut sink = TestSink::new();
        sink.add_empty_holding_result = Ter::TEC_PATH_DRY;

        let result = run_loan_pay_do_apply(&mut sink, apply_facts());

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(
            sink.steps.borrow().as_slice(),
            &[
                "read_loan",
                "read_broker",
                "read_vault",
                "required_cover",
                "owner_deep_frozen",
                "owner_auth",
                "make_payment",
                "update_loan",
                "update_broker",
                "sample_balance",
                "vault_scale",
                "round_to_asset_downward",
                "asset_is_integral",
                "is_rounded",
                "adjust_broker_debt_total",
                "update_vault",
                "sample_balance",
                "sample_balance",
                "sample_balance",
                "require_auth",
                "broker_payee_balance",
                "add_empty_holding",
            ]
        );
    }

    #[test]
    fn loan_pay_do_apply_skips_broker_payee_holding_balance_when_payee_is_not_borrower() {
        let mut sink = TestSink::new();
        sink.required_cover = 11;
        sink.expected_broker_payee = "broker-pseudo";

        let result = run_loan_pay_do_apply(&mut sink, apply_facts());

        assert!(result.is_ok());
        assert!(
            !sink
                .steps
                .borrow()
                .iter()
                .any(|step| *step == "broker_payee_balance")
        );
        assert!(
            !sink
                .steps
                .borrow()
                .iter()
                .any(|step| *step == "add_empty_holding")
        );
    }
}
