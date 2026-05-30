use std::{fmt, sync::Arc};

use basics::{
    base_uint::Uint256,
    number::{NumberParts as RuntimeNumber, RoundingMode},
};
use ledger::views::apply_view::ApplyView;
use protocol::StBase;
use protocol::{
    AccountID, Asset, LedgerEntryType, STAmount, STLedgerEntry, STTx, TenthBips16, TenthBips32,
    Ter, XRPAmount, account_keylet, feature_id, loan_key, owner_dir_keylet,
};

use super::common::*;
use super::helpers::*;

pub(super) struct LoanSetTxView<'a> {
    _sttx: &'a STTx,
    broker_id: Uint256,
    account: AccountID,
    counterparty: Option<AccountID>,
    principal_requested: RuntimeNumber,
    principal_requested_amount: LoanSetAmountValue,
    loan_origination_fee: Option<RuntimeNumber>,
    loan_origination_fee_amount: Option<LoanSetAmountValue>,
    loan_service_fee_amount: Option<LoanSetAmountValue>,
    late_payment_fee_amount: Option<LoanSetAmountValue>,
    close_payment_fee_amount: Option<LoanSetAmountValue>,
    interest_rate: Option<TenthBips32>,
    payment_interval: Option<u32>,
    payment_total: Option<u32>,
}

#[derive(Clone)]
pub(super) struct LoanSetBrokerView {
    entry: STLedgerEntry,
    owner: AccountID,
    vault_id: Uint256,
    pseudo_account: AccountID,
    management_fee_rate: TenthBips16,
    debt_total: RuntimeNumber,
    debt_maximum: RuntimeNumber,
    cover_available: RuntimeNumber,
    cover_rate_minimum: u32,
}

#[derive(Clone)]
pub(super) struct LoanSetVaultView {
    entry: STLedgerEntry,
    pseudo_account: AccountID,
    asset: Asset,
    assets_available: RuntimeNumber,
    assets_total: RuntimeNumber,
    assets_maximum: RuntimeNumber,
}

#[derive(Clone)]
pub(super) struct LoanSetAccountState {
    balance_drops: i64,
}

#[derive(Clone)]
pub(super) struct LoanSetProperties {
    inner: tx::LoanSetLoanProperties<RuntimeNumber>,
}

#[derive(Clone)]
pub(super) struct LoanSetState {
    inner: tx::LoanSetLoanState<RuntimeNumber>,
}

#[derive(Clone)]
pub(super) struct LoanSetAmountValue {
    amount: STAmount,
}

impl fmt::Display for LoanSetAmountValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.amount.text())
    }
}

impl tx::LoanSetDoApplyLedgerStateTx for LoanSetTxView<'_> {
    type BrokerId = Uint256;
    type AccountId = AccountID;

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

impl tx::LoanSetDoApplyRepresentabilityTx for LoanSetTxView<'_> {
    type Value = LoanSetAmountValue;

    fn value(&self, field: tx::LoanSetRepresentabilityField) -> Option<&Self::Value> {
        match field {
            tx::LoanSetRepresentabilityField::PrincipalRequested => {
                Some(&self.principal_requested_amount)
            }
            tx::LoanSetRepresentabilityField::LoanOriginationFee => {
                self.loan_origination_fee_amount.as_ref()
            }
            tx::LoanSetRepresentabilityField::LoanServiceFee => {
                self.loan_service_fee_amount.as_ref()
            }
            tx::LoanSetRepresentabilityField::LatePaymentFee => {
                self.late_payment_fee_amount.as_ref()
            }
            tx::LoanSetRepresentabilityField::ClosePaymentFee => {
                self.close_payment_fee_amount.as_ref()
            }
        }
    }
}

impl tx::LoanSetDoApplyPreGuardedTransferTx for LoanSetTxView<'_> {
    type Amount = RuntimeNumber;
    type InterestRate = TenthBips32;

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

impl tx::LoanSetDoApplyLoadedTransferAndPostTransferTx for LoanSetTxView<'_> {
    fn loan_origination_fee(&self) -> Option<&Self::Amount> {
        self.loan_origination_fee.as_ref()
    }
}

impl tx::LoanSetDoApplyLedgerStateBroker for LoanSetBrokerView {
    type AccountId = AccountID;
    type VaultId = Uint256;

    fn owner(&self) -> &Self::AccountId {
        &self.owner
    }

    fn vault_id(&self) -> &Self::VaultId {
        &self.vault_id
    }

    fn account(&self) -> &Self::AccountId {
        &self.pseudo_account
    }
}

impl tx::LoanSetDoApplyLoadedPreGuardedTransferBroker for LoanSetBrokerView {
    type ManagementFeeRate = TenthBips16;

    fn management_fee_rate(&self) -> Self::ManagementFeeRate {
        self.management_fee_rate
    }
}

impl tx::LoanSetDoApplyLoadedGuardedTransferBroker for LoanSetBrokerView {
    type Amount = RuntimeNumber;
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

impl tx::LoanSetDoApplyLedgerStateVault for LoanSetVaultView {
    type AccountId = AccountID;
    type Asset = Asset;

    fn account(&self) -> &Self::AccountId {
        &self.pseudo_account
    }

    fn asset(&self) -> &Self::Asset {
        &self.asset
    }
}

impl tx::LoanSetDoApplyLoadedPreGuardedTransferVault for LoanSetVaultView {
    type Amount = RuntimeNumber;

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

impl tx::LoanSetDoApplyLoadedTransferAndPostTransferAccountState for LoanSetAccountState {
    type Balance = i64;

    fn balance(&self) -> &Self::Balance {
        &self.balance_drops
    }
}

impl tx::LoanSetDoApplyPreGuardedTransferProperties for LoanSetProperties {
    type Amount = RuntimeNumber;

    fn loan_scale(&self) -> i32 {
        self.inner.loan_scale
    }

    fn total_value_outstanding(&self) -> &Self::Amount {
        &self.inner.loan_state.value_outstanding
    }

    fn management_fee_due(&self) -> &Self::Amount {
        &self.inner.loan_state.management_fee_due
    }

    fn periodic_payment(&self) -> &Self::Amount {
        &self.inner.periodic_payment
    }
}

impl tx::LoanSetDoApplyPreGuardedTransferState for LoanSetState {
    type Amount = RuntimeNumber;

    fn interest_due(&self) -> &Self::Amount {
        &self.inner.interest_due
    }
}

pub(super) fn loan_set_debt_total_update(
    asset: Asset,
    current: RuntimeNumber,
    adjustment: RuntimeNumber,
    vault_sle: &STLedgerEntry,
    loan_scale: i32,
    fix_cleanup_3_2_0: bool,
) -> RuntimeNumber {
    let scale = if fix_cleanup_3_2_0 {
        vault_scale(vault_sle, asset)
    } else {
        loan_scale
    };
    adjust_imprecise_number(current, adjustment, asset, scale)
}

pub(super) fn load_loan_set_tx_view(sttx: &STTx) -> LoanSetTxView<'_> {
    LoanSetTxView {
        _sttx: sttx,
        broker_id: sttx.get_field_h256(sf("sfLoanBrokerID")),
        account: sttx.get_account_id(sf("sfAccount")),
        counterparty: sttx
            .is_field_present(sf("sfCounterparty"))
            .then(|| sttx.get_account_id(sf("sfCounterparty"))),
        principal_requested: amount_number(&sttx.get_field_amount(sf("sfPrincipalRequested"))),
        principal_requested_amount: LoanSetAmountValue {
            amount: sttx.get_field_amount(sf("sfPrincipalRequested")),
        },
        loan_origination_fee: sttx
            .is_field_present(sf("sfLoanOriginationFee"))
            .then(|| amount_number(&sttx.get_field_amount(sf("sfLoanOriginationFee")))),
        loan_origination_fee_amount: sttx.is_field_present(sf("sfLoanOriginationFee")).then(|| {
            LoanSetAmountValue {
                amount: sttx.get_field_amount(sf("sfLoanOriginationFee")),
            }
        }),
        loan_service_fee_amount: sttx.is_field_present(sf("sfLoanServiceFee")).then(|| {
            LoanSetAmountValue {
                amount: sttx.get_field_amount(sf("sfLoanServiceFee")),
            }
        }),
        late_payment_fee_amount: sttx.is_field_present(sf("sfLatePaymentFee")).then(|| {
            LoanSetAmountValue {
                amount: sttx.get_field_amount(sf("sfLatePaymentFee")),
            }
        }),
        close_payment_fee_amount: sttx.is_field_present(sf("sfClosePaymentFee")).then(|| {
            LoanSetAmountValue {
                amount: sttx.get_field_amount(sf("sfClosePaymentFee")),
            }
        }),
        interest_rate: sttx
            .is_field_present(sf("sfInterestRate"))
            .then(|| TenthBips32::new(sttx.get_field_u32(sf("sfInterestRate")))),
        payment_interval: sttx
            .is_field_present(sf("sfPaymentInterval"))
            .then(|| sttx.get_field_u32(sf("sfPaymentInterval"))),
        payment_total: sttx
            .is_field_present(sf("sfPaymentTotal"))
            .then(|| sttx.get_field_u32(sf("sfPaymentTotal"))),
    }
}

pub(super) fn load_loan_set_broker_view(entry: STLedgerEntry) -> LoanSetBrokerView {
    LoanSetBrokerView {
        owner: entry.get_account_id(sf("sfOwner")),
        vault_id: entry.get_field_h256(sf("sfVaultID")),
        pseudo_account: entry.get_account_id(sf("sfAccount")),
        management_fee_rate: TenthBips16::new(
            if entry.is_field_present(sf("sfManagementFeeRate")) {
                entry.get_field_u16(sf("sfManagementFeeRate"))
            } else {
                0
            },
        ),
        debt_total: entry.get_field_number(sf("sfDebtTotal")).value(),
        debt_maximum: if entry.is_field_present(sf("sfDebtMaximum")) {
            entry.get_field_number(sf("sfDebtMaximum")).value()
        } else {
            RuntimeNumber::zero()
        },
        cover_available: entry.get_field_number(sf("sfCoverAvailable")).value(),
        cover_rate_minimum: entry.get_field_u32(sf("sfCoverRateMinimum")),
        entry,
    }
}

pub(super) fn load_loan_set_vault_view(entry: STLedgerEntry) -> LoanSetVaultView {
    let asset = entry.get_field_issue(sf("sfAsset")).asset();
    LoanSetVaultView {
        pseudo_account: entry.get_account_id(sf("sfAccount")),
        assets_available: entry.get_field_number(sf("sfAssetsAvailable")).value(),
        assets_total: entry.get_field_number(sf("sfAssetsTotal")).value(),
        assets_maximum: if entry.is_field_present(sf("sfAssetsMaximum")) {
            entry.get_field_number(sf("sfAssetsMaximum")).value()
        } else {
            RuntimeNumber::zero()
        },
        entry,
        asset,
    }
}

pub(super) fn load_loan_set_account_state(entry: STLedgerEntry) -> LoanSetAccountState {
    LoanSetAccountState {
        balance_drops: entry.get_field_amount(sf("sfBalance")).xrp().drops(),
    }
}

pub(super) fn set_optional_loan_tx_field(
    loan: &mut STLedgerEntry,
    sttx: &STTx,
    field: tx::LoanSetDoApplyLoanTransactionField,
    default_value: Option<u32>,
) {
    match field {
        tx::LoanSetDoApplyLoanTransactionField::LoanOriginationFee => {
            if sttx.is_field_present(sf("sfLoanOriginationFee")) {
                loan.set_field_amount(
                    sf("sfLoanOriginationFee"),
                    sttx.get_field_amount(sf("sfLoanOriginationFee")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::LoanServiceFee => {
            if sttx.is_field_present(sf("sfLoanServiceFee")) {
                loan.set_field_amount(
                    sf("sfLoanServiceFee"),
                    sttx.get_field_amount(sf("sfLoanServiceFee")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::LatePaymentFee => {
            if sttx.is_field_present(sf("sfLatePaymentFee")) {
                loan.set_field_amount(
                    sf("sfLatePaymentFee"),
                    sttx.get_field_amount(sf("sfLatePaymentFee")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::ClosePaymentFee => {
            if sttx.is_field_present(sf("sfClosePaymentFee")) {
                loan.set_field_amount(
                    sf("sfClosePaymentFee"),
                    sttx.get_field_amount(sf("sfClosePaymentFee")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::OverpaymentFee => {
            if sttx.is_field_present(sf("sfOverpaymentFee")) {
                loan.set_field_amount(
                    sf("sfOverpaymentFee"),
                    sttx.get_field_amount(sf("sfOverpaymentFee")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::InterestRate => {
            if sttx.is_field_present(sf("sfInterestRate")) {
                loan.set_field_u32(
                    sf("sfInterestRate"),
                    sttx.get_field_u32(sf("sfInterestRate")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::LateInterestRate => {
            if sttx.is_field_present(sf("sfLateInterestRate")) {
                loan.set_field_u32(
                    sf("sfLateInterestRate"),
                    sttx.get_field_u32(sf("sfLateInterestRate")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::CloseInterestRate => {
            if sttx.is_field_present(sf("sfCloseInterestRate")) {
                loan.set_field_u32(
                    sf("sfCloseInterestRate"),
                    sttx.get_field_u32(sf("sfCloseInterestRate")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::OverpaymentInterestRate => {
            if sttx.is_field_present(sf("sfOverpaymentInterestRate")) {
                loan.set_field_u32(
                    sf("sfOverpaymentInterestRate"),
                    sttx.get_field_u32(sf("sfOverpaymentInterestRate")),
                );
            }
        }
        tx::LoanSetDoApplyLoanTransactionField::GracePeriod => {
            let value = if sttx.is_field_present(sf("sfGracePeriod")) {
                sttx.get_field_u32(sf("sfGracePeriod"))
            } else {
                default_value.unwrap_or_default()
            };
            loan.set_field_u32(sf("sfGracePeriod"), value);
        }
    }
}

pub fn apply_loan_set<V: ApplyView>(view: &mut V, sttx: &STTx, pre_fee_balance_drops: i64) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    // Guard against missing required fields that would cause zero-key panics
    if !sttx.is_field_present(sf("sfLoanBrokerID"))
        || !sttx.is_field_present(sf("sfPrincipalRequested"))
    {
        return Ter::TEM_MALFORMED;
    }

    let tx_view = load_loan_set_tx_view(sttx);
    let principal_requested_amount = sttx.get_field_amount(sf("sfPrincipalRequested"));
    let view_ptr: *mut V = view;
    let representability_asset = {
        let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
        let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
        else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
            broker_sle.get_field_h256(sf("sfVaultID")),
        )) else {
            return Ter::TEF_BAD_LEDGER;
        };
        vault_sle.get_field_issue(sf("sfAsset")).asset()
    };
    let rules = view.rules();

    tx::run_loan_set_family_do_apply(
        &tx_view,
        &pre_fee_balance_drops,
        |broker_id| {
            unsafe { &mut *view_ptr }
                .peek(protocol::loan_broker_keylet_from_key(*broker_id))
                .ok()
                .flatten()
                .map(|sle| load_loan_set_broker_view((*sle).clone()))
        },
        |vault_id| {
            unsafe { &mut *view_ptr }
                .peek(protocol::vault_keylet_from_key(*vault_id))
                .ok()
                .flatten()
                .map(|sle| load_loan_set_vault_view((*sle).clone()))
        },
        |account| {
            unsafe { &mut *view_ptr }
                .peek(account_keylet(to_160(account)))
                .ok()
                .flatten()
                .map(|sle| load_loan_set_account_state((*sle).clone()))
        },
        TenthBips32::new(0),
        2_592_000,
        12,
        &RuntimeNumber::zero(),
        |vault| vault_scale(&vault.entry, vault.asset),
        |asset,
         principal_requested,
         interest_rate,
         payment_interval,
         payment_total,
         management_fee_rate,
         minimum_scale| {
            LoanSetProperties {
                inner: tx::compute_loan_set_properties(
                    &rules,
                    *asset,
                    *principal_requested,
                    interest_rate,
                    payment_interval,
                    payment_total,
                    management_fee_rate,
                    minimum_scale,
                ),
            }
        },
        |total_value_outstanding, principal_outstanding, management_fee_outstanding| LoanSetState {
            inner: tx::construct_loan_set_state(
                *total_value_outstanding,
                *principal_outstanding,
                *management_fee_outstanding,
            ),
        },
        |_, value| {
            runtime_to_amount(
                representability_asset,
                amount_number(&value.amount),
                RoundingMode::ToNearest,
            )
            .is_some()
        },
        |asset, principal_requested, expect_interest, payment_total, properties| {
            let guards = tx::LoanSetLoanGuardProperties {
                periodic_payment: properties.inner.periodic_payment,
                total_value_outstanding: properties.inner.loan_state.value_outstanding,
                loan_scale: properties.inner.loan_scale,
                first_payment_principal: properties.inner.first_payment_principal,
            };
            match tx::check_loan_set_loan_guards(
                asset,
                principal_requested,
                expect_interest,
                payment_total,
                &guards,
                &RuntimeNumber::zero(),
                |asset, value, scale| {
                    round_number_to_asset_with_scale(*asset, *value, scale, RoundingMode::ToNearest)
                },
                |total, rounded| {
                    if *rounded <= RuntimeNumber::zero() {
                        0
                    } else {
                        let mut payments = 0_i64;
                        let mut remaining = *total;
                        while remaining > RuntimeNumber::zero() {
                            remaining -= *rounded;
                            payments += 1;
                            if payments > i64::from(u32::MAX) {
                                break;
                            }
                        }
                        payments
                    }
                },
            ) {
                Ok(()) => Ter::TES_SUCCESS,
                Err(err) => err.ter(),
            }
        },
        |debt_total, cover_rate| {
            let view = unsafe { &mut *view_ptr };
            let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
            let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
            else {
                return tenth_bips_of_runtime_number(*debt_total, cover_rate);
            };
            let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
                broker_sle.get_field_h256(sf("sfVaultID")),
            )) else {
                return tenth_bips_of_runtime_number(*debt_total, cover_rate);
            };
            let asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
            minimum_broker_cover(
                asset,
                *debt_total,
                cover_rate,
                &vault_sle,
                view.rules().enabled(&feature_id("fixCleanup3_2_0")),
            )
        },
        || {
            let view = unsafe { &mut *view_ptr };
            let borrower = if sttx.is_field_present(sf("sfCounterparty")) {
                let counterparty = sttx.get_account_id(sf("sfCounterparty"));
                let broker_owner = view
                    .peek(protocol::loan_broker_keylet_from_key(
                        sttx.get_field_h256(sf("sfLoanBrokerID")),
                    ))
                    .ok()
                    .flatten()
                    .map(|sle| sle.get_account_id(sf("sfOwner")))
                    .unwrap_or(sttx.get_account_id(sf("sfAccount")));
                if counterparty == broker_owner {
                    sttx.get_account_id(sf("sfAccount"))
                } else {
                    counterparty
                }
            } else {
                sttx.get_account_id(sf("sfAccount"))
            };
            let keylet = account_keylet(to_160(&borrower));
            let Ok(Some(borrower_sle)) = view.peek(keylet) else {
                return 0;
            };
            let _ = ledger::adjust_owner_count(view, &borrower_sle, 1);
            view.peek(keylet)
                .ok()
                .flatten()
                .map(|sle| sle.get_field_u32(sf("sfOwnerCount")))
                .unwrap_or(0)
        },
        |owner_count| {
            unsafe { &*view_ptr }
                .fees()
                .account_reserve(owner_count as usize) as i64
        },
        || {
            let view = unsafe { &mut *view_ptr };
            let borrower = sttx.get_account_id(sf("sfAccount"));
            let prior = XRPAmount::from_drops(pre_fee_balance_drops);
            let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
            let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
            else {
                return Ter::TEF_BAD_LEDGER;
            };
            let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
                broker_sle.get_field_h256(sf("sfVaultID")),
            )) else {
                return Ter::TEF_BAD_LEDGER;
            };
            ledger::add_empty_holding(
                view,
                &borrower,
                prior,
                &vault_sle.get_field_issue(sf("sfAsset")).asset(),
            )
        },
        || Ter::TES_SUCCESS,
        || {
            let view = unsafe { &mut *view_ptr };
            let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
            let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
            else {
                return Ter::TEF_BAD_LEDGER;
            };
            let owner = broker_sle.get_account_id(sf("sfOwner"));
            let prior = view
                .peek(account_keylet(to_160(&owner)))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")).xrp())
                .unwrap_or_else(XRPAmount::new);
            let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
                broker_sle.get_field_h256(sf("sfVaultID")),
            )) else {
                return Ter::TEF_BAD_LEDGER;
            };
            ledger::add_empty_holding(
                view,
                &owner,
                prior,
                &vault_sle.get_field_issue(sf("sfAsset")).asset(),
            )
        },
        || Ter::TES_SUCCESS,
        || {
            let view = unsafe { &mut *view_ptr };
            let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
            let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
            else {
                return Ter::TEF_BAD_LEDGER;
            };
            let broker_owner = broker_sle.get_account_id(sf("sfOwner"));
            let counterparty = sttx
                .is_field_present(sf("sfCounterparty"))
                .then(|| sttx.get_account_id(sf("sfCounterparty")));
            let borrower = match counterparty {
                Some(cp) if cp != broker_owner => cp,
                _ => sttx.get_account_id(sf("sfAccount")),
            };
            let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
                broker_sle.get_field_h256(sf("sfVaultID")),
            )) else {
                return Ter::TEF_BAD_LEDGER;
            };
            let asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
            let Some(origination_fee) = sttx
                .is_field_present(sf("sfLoanOriginationFee"))
                .then(|| amount_number(&sttx.get_field_amount(sf("sfLoanOriginationFee"))))
            else {
                let amount = principal_requested_amount.clone();
                return account_send(
                    view,
                    &vault_sle.get_account_id(sf("sfAccount")),
                    &borrower,
                    &amount,
                );
            };
            let borrower_amount_number =
                amount_number(&principal_requested_amount) - origination_fee;
            let Some(borrower_amount) =
                runtime_to_amount(asset, borrower_amount_number, RoundingMode::ToNearest)
            else {
                return Ter::TEC_INTERNAL;
            };
            let borrower_result = account_send(
                view,
                &vault_sle.get_account_id(sf("sfAccount")),
                &borrower,
                &borrower_amount,
            );
            if borrower_result != Ter::TES_SUCCESS {
                return borrower_result;
            }
            account_send(
                view,
                &vault_sle.get_account_id(sf("sfAccount")),
                &broker_owner,
                &sttx.get_field_amount(sf("sfLoanOriginationFee")),
            )
        },
        || {
            let view = unsafe { &mut *view_ptr };
            let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
            let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
            else {
                return Ter::TEF_BAD_LEDGER;
            };
            let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
                broker_sle.get_field_h256(sf("sfVaultID")),
            )) else {
                return Ter::TEF_BAD_LEDGER;
            };

            let broker = load_loan_set_broker_view((*broker_sle).clone());
            let vault = load_loan_set_vault_view((*vault_sle).clone());
            let payment_interval = if sttx.is_field_present(sf("sfPaymentInterval")) {
                sttx.get_field_u32(sf("sfPaymentInterval"))
            } else {
                2_592_000
            };
            let payment_total = if sttx.is_field_present(sf("sfPaymentTotal")) {
                sttx.get_field_u32(sf("sfPaymentTotal"))
            } else {
                12
            };
            let interest_rate = sttx
                .is_field_present(sf("sfInterestRate"))
                .then(|| TenthBips32::new(sttx.get_field_u32(sf("sfInterestRate"))))
                .unwrap_or(TenthBips32::new(0));
            let minimum_scale = vault_scale(&vault.entry, vault.asset);
            let properties = tx::compute_loan_set_properties(
                &rules,
                vault.asset,
                amount_number(&principal_requested_amount),
                interest_rate,
                payment_interval,
                payment_total,
                broker.management_fee_rate,
                minimum_scale,
            );
            let state = tx::construct_loan_set_state(
                properties.loan_state.value_outstanding,
                amount_number(&principal_requested_amount),
                properties.loan_state.management_fee_due,
            );

            let counterparty = sttx
                .is_field_present(sf("sfCounterparty"))
                .then(|| sttx.get_account_id(sf("sfCounterparty")));
            let borrower = match counterparty {
                Some(cp) if cp != broker.owner => cp,
                _ => sttx.get_account_id(sf("sfAccount")),
            };
            let start_date = view.parent_close_time().as_seconds();
            let loan_sequence = broker.entry.get_field_u32(sf("sfLoanSequence"));
            let loan_id = loan_key(broker_id, loan_sequence);
            let mut loan = STLedgerEntry::from_type_and_key(LedgerEntryType::Loan, loan_id);
            loan.set_field_h256(sf("sfPreviousTxnID"), sttx.get_transaction_id());
            loan.set_field_u32(sf("sfPreviousTxnLgrSeq"), view.seq());
            loan.set_field_i32(sf("sfLoanScale"), properties.loan_scale);
            loan.set_field_u32(sf("sfStartDate"), start_date);
            loan.set_field_u32(sf("sfPaymentInterval"), payment_interval);
            loan.set_field_u32(sf("sfLoanSequence"), loan_sequence);
            loan.set_field_h256(sf("sfLoanBrokerID"), broker_id);
            loan.set_account_id(sf("sfBorrower"), borrower);
            if sttx.is_flag(protocol::tfLoanOverpayment) {
                loan.set_flag(protocol::lsfLoanOverpayment);
            }
            tx::run_loan_set_do_apply_loan_transaction_fields(0, |field, default_value| {
                set_optional_loan_tx_field(&mut loan, sttx, field, default_value)
            });
            loan.set_field_number(
                sf("sfPrincipalOutstanding"),
                with_asset_number(amount_number(&principal_requested_amount), vault.asset),
            );
            loan.set_field_number(
                sf("sfPeriodicPayment"),
                with_asset_number(properties.periodic_payment, vault.asset),
            );
            loan.set_field_number(
                sf("sfTotalValueOutstanding"),
                with_asset_number(properties.loan_state.value_outstanding, vault.asset),
            );
            loan.set_field_number(
                sf("sfManagementFeeOutstanding"),
                with_asset_number(properties.loan_state.management_fee_due, vault.asset),
            );
            loan.set_field_u32(sf("sfPreviousPaymentDueDate"), 0);
            loan.set_field_u32(
                sf("sfNextPaymentDueDate"),
                start_date.saturating_add(payment_interval),
            );
            loan.set_field_u32(sf("sfPaymentRemaining"), payment_total);
            let broker_pseudo_page = match ledger::dir_insert(
                view,
                &owner_dir_keylet(to_160(&broker.pseudo_account)),
                loan_id,
                &|_| {},
            ) {
                Ok(Some(page)) => page,
                _ => return Ter::TEF_BAD_LEDGER,
            };
            let borrower_page = match ledger::dir_insert(
                view,
                &owner_dir_keylet(to_160(&borrower)),
                loan_id,
                &|_| {},
            ) {
                Ok(Some(page)) => page,
                _ => return Ter::TEF_BAD_LEDGER,
            };
            loan.set_field_u64(sf("sfLoanBrokerNode"), broker_pseudo_page);
            loan.set_field_u64(sf("sfOwnerNode"), borrower_page);
            associate_asset_entry(&mut loan, vault.asset);
            match view.insert(Arc::new(loan)) {
                Ok(_) => {}
                Err(_) => return Ter::TEF_BAD_LEDGER,
            }

            let mut vault_obj = vault.entry.clone_as_object();
            vault_obj.set_field_number(
                sf("sfAssetsAvailable"),
                with_asset_number(
                    vault.assets_available - amount_number(&principal_requested_amount),
                    vault.asset,
                ),
            );
            vault_obj.set_field_number(
                sf("sfAssetsTotal"),
                with_asset_number(vault.assets_total + state.interest_due, vault.asset),
            );
            let mut vault_entry = STLedgerEntry::from_stobject(vault_obj, *vault.entry.key());
            associate_asset_entry(&mut vault_entry, vault.asset);
            let vault_update = persist_entry(view, vault_entry);
            if vault_update != Ter::TES_SUCCESS {
                return vault_update;
            }

            let mut broker_obj = broker.entry.clone_as_object();
            broker_obj.set_field_number(
                sf("sfDebtTotal"),
                with_asset_number(
                    loan_set_debt_total_update(
                        vault.asset,
                        broker.debt_total,
                        amount_number(&principal_requested_amount) + state.interest_due,
                        &vault.entry,
                        properties.loan_scale,
                        rules.enabled(&feature_id("fixCleanup3_2_0")),
                    ),
                    vault.asset,
                ),
            );
            broker_obj.set_field_u32(
                sf("sfOwnerCount"),
                broker
                    .entry
                    .get_field_u32(sf("sfOwnerCount"))
                    .saturating_add(1),
            );
            let next_sequence = broker
                .entry
                .get_field_u32(sf("sfLoanSequence"))
                .wrapping_add(1);
            if next_sequence == 0 {
                return Ter::TEC_MAX_SEQUENCE_REACHED;
            }
            broker_obj.set_field_u32(sf("sfLoanSequence"), next_sequence);
            let mut broker_entry = STLedgerEntry::from_stobject(broker_obj, *broker.entry.key());
            associate_asset_entry(&mut broker_entry, vault.asset);
            let broker_update = persist_entry(view, broker_entry);
            if broker_update != Ter::TES_SUCCESS {
                return broker_update;
            }
            Ter::TES_SUCCESS
        },
    )
}

pub fn apply_loan_broker_set<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    pre_fee_balance_drops: i64,
) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let vault_id = sttx.get_field_h256(sf("sfVaultID"));

    if sttx.is_field_present(sf("sfLoanBrokerID")) {
        let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
        let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
        else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
            broker_sle.get_field_h256(sf("sfVaultID")),
        )) else {
            return Ter::TEC_INTERNAL;
        };
        let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();

        let mut obj = broker_sle.clone_as_object();
        if sttx.is_field_present(sf("sfData")) {
            let data = sttx.get_field_vl(sf("sfData"));
            obj.set_field_vl(sf("sfData"), &data);
        }
        if sttx.is_field_present(sf("sfDebtMaximum")) {
            obj.set_field_number(
                sf("sfDebtMaximum"),
                sttx.get_field_number(sf("sfDebtMaximum")),
            );
        }
        let mut broker = STLedgerEntry::from_stobject(obj, *broker_sle.key());
        associate_asset_entry(&mut broker, vault_asset);
        return persist_entry(view, broker);
    }

    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(vault_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_pseudo_id = vault_sle.get_account_id(sf("sfAccount"));
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    let sequence = sttx.get_seq_value();

    let Ok(Some(owner_sle)) = view.peek(account_keylet(to_160(&account))) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let broker_keylet = protocol::loan_broker_keylet(to_160(&account), sequence);
    let owner_page = match ledger::dir_insert(
        view,
        &owner_dir_keylet(to_160(&account)),
        broker_keylet.key,
        &|_| {},
    ) {
        Ok(Some(page)) => page,
        _ => return Ter::TEF_BAD_LEDGER,
    };
    let vault_page = match ledger::dir_insert(
        view,
        &owner_dir_keylet(to_160(&vault_pseudo_id)),
        broker_keylet.key,
        &|_| {},
    ) {
        Ok(Some(page)) => page,
        _ => return Ter::TEF_BAD_LEDGER,
    };

    let _ = ledger::adjust_owner_count(view, &owner_sle, 2);
    let Ok(Some(updated_owner_sle)) = view.peek(account_keylet(to_160(&account))) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let reserve = view
        .fees()
        .account_reserve(updated_owner_sle.get_field_u32(sf("sfOwnerCount")) as usize);
    if pre_fee_balance_drops < reserve as i64 {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let pseudo = match ledger::create_pseudo_account(view, broker_keylet.key, sf("sfLoanBrokerID"))
    {
        Ok(pseudo) => pseudo,
        Err(ter) => return ter,
    };
    let pseudo_id = pseudo.get_account_id(sf("sfAccount"));

    let holding = ledger::add_empty_holding(
        view,
        &pseudo_id,
        XRPAmount::from_drops(pre_fee_balance_drops),
        &vault_asset,
    );
    if holding != Ter::TES_SUCCESS {
        return holding;
    }

    let mut broker =
        STLedgerEntry::from_type_and_key(LedgerEntryType::LoanBroker, broker_keylet.key);
    broker.set_field_h256(sf("sfPreviousTxnID"), sttx.get_transaction_id());
    broker.set_field_u32(sf("sfPreviousTxnLgrSeq"), view.seq());
    broker.set_field_u32(sf("sfSequence"), sequence);
    broker.set_field_u64(sf("sfOwnerNode"), owner_page);
    broker.set_field_u64(sf("sfVaultNode"), vault_page);
    broker.set_field_h256(sf("sfVaultID"), vault_id);
    broker.set_account_id(sf("sfOwner"), account);
    broker.set_account_id(sf("sfAccount"), pseudo_id);
    broker.set_field_u32(sf("sfLoanSequence"), 1);
    broker.set_field_u32(sf("sfOwnerCount"), 0);
    broker.set_field_number(
        sf("sfDebtTotal"),
        with_asset_number(RuntimeNumber::zero(), vault_asset),
    );
    broker.set_field_number(
        sf("sfCoverAvailable"),
        with_asset_number(RuntimeNumber::zero(), vault_asset),
    );
    if sttx.is_field_present(sf("sfData")) {
        let data = sttx.get_field_vl(sf("sfData"));
        broker.set_field_vl(sf("sfData"), &data);
    }
    if sttx.is_field_present(sf("sfManagementFeeRate")) {
        broker.set_field_u16(
            sf("sfManagementFeeRate"),
            sttx.get_field_u16(sf("sfManagementFeeRate")),
        );
    }
    if sttx.is_field_present(sf("sfDebtMaximum")) {
        broker.set_field_number(
            sf("sfDebtMaximum"),
            sttx.get_field_number(sf("sfDebtMaximum")),
        );
    }
    if sttx.is_field_present(sf("sfCoverRateMinimum")) {
        broker.set_field_u32(
            sf("sfCoverRateMinimum"),
            sttx.get_field_u32(sf("sfCoverRateMinimum")),
        );
    }
    if sttx.is_field_present(sf("sfCoverRateLiquidation")) {
        broker.set_field_u32(
            sf("sfCoverRateLiquidation"),
            sttx.get_field_u32(sf("sfCoverRateLiquidation")),
        );
    }
    associate_asset_entry(&mut broker, vault_asset);
    view.insert(Arc::new(broker))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}
