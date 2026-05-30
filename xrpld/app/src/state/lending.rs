use std::{fmt, sync::Arc};

use basics::{
    base_uint::{Uint160, Uint192, Uint256},
    number::{
        NumberParts as RuntimeNumber, NumberRoundModeGuard, RoundingMode, get_mantissa_scale,
    },
};
use ledger::{ReadView, RelativeDistanceAmount, has_expired, views::apply_view::ApplyView};
use protocol::StBase;
use protocol::{
    AccountID, Asset, LedgerEntryType, MPTIssue, STAmount, STLedgerEntry, STNumber, STTx,
    TenthBips16, TenthBips32, Ter, XRPAmount, account_keylet, feature_id, get_field_by_symbol,
    lending::LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION, loan_key, mpt_issuance_keylet_from_mptid,
    mptoken_keylet_from_mptid, owner_dir_keylet, tfLoanDefault, tfLoanImpair, tfLoanUnimpair,
    to_amount_from_number,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_160(account: &AccountID) -> Uint160 {
    Uint160::from_void(account.data())
}

fn account_send<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    account_send_with_mpt_transfer_waiver(view, from, to, amount, false)
}

fn account_send_with_mpt_transfer_waiver<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
    waive_mpt_can_transfer: bool,
) -> Ter {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            let from_keylet = account_keylet(to_160(from));
            let to_keylet = account_keylet(to_160(to));
            let Ok(Some(from_sle)) = view.peek(from_keylet) else {
                return Ter::TEF_BAD_LEDGER;
            };
            let Ok(Some(to_sle)) = view.peek(to_keylet) else {
                return Ter::TEF_BAD_LEDGER;
            };
            let from_balance = from_sle.get_field_amount(sf("sfBalance")).xrp().drops();
            let to_balance = to_sle.get_field_amount(sf("sfBalance")).xrp().drops();
            let drops = amount.xrp().drops();
            if from_balance < drops {
                return Ter::TEC_INSUFFICIENT_FUNDS;
            }
            let mut from_obj = from_sle.clone_as_object();
            from_obj.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(from_balance - drops)),
            );
            let mut to_obj = to_sle.clone_as_object();
            to_obj.set_field_amount(
                sf("sfBalance"),
                STAmount::from_xrp_amount(XRPAmount::from_drops(to_balance + drops)),
            );
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                from_obj,
                *from_sle.key(),
            )));
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                to_obj,
                *to_sle.key(),
            )));
            Ter::TES_SUCCESS
        }
        Asset::Issue(_) => ledger::ripple_state_helpers::account_send(view, from, to, amount),
        Asset::MPTIssue(issue) => transfer_mpt(
            view,
            issue,
            from,
            to,
            amount.mpt().value().unsigned_abs(),
            waive_mpt_can_transfer,
        ),
    }
}

fn asset_issuer(asset: Asset) -> AccountID {
    match asset {
        Asset::Issue(issue) => issue.account,
        Asset::MPTIssue(issue) => issue.issuer(),
    }
}

fn token_balance<V: ApplyView>(view: &mut V, mpt_id: Uint192, account: &AccountID) -> Option<u64> {
    view.peek(mptoken_keylet_from_mptid(mpt_id, to_160(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_u64(sf("sfMPTAmount")))
}

fn set_token_balance<V: ApplyView>(
    view: &mut V,
    mpt_id: Uint192,
    account: &AccountID,
    balance: u64,
) -> Ter {
    let Ok(Some(sle)) = view.peek(mptoken_keylet_from_mptid(mpt_id, to_160(account))) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut obj = sle.clone_as_object();
    obj.set_field_u64(sf("sfMPTAmount"), balance);
    view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn transfer_mpt<V: ApplyView>(
    view: &mut V,
    issue: MPTIssue,
    from: &AccountID,
    to: &AccountID,
    amount: u64,
    waive_can_transfer: bool,
) -> Ter {
    if amount == 0 || from == to {
        return Ter::TES_SUCCESS;
    }
    if ledger::mptoken_helpers::is_frozen_mpt(view, from, &issue).unwrap_or(false)
        || ledger::mptoken_helpers::is_frozen_mpt(view, to, &issue).unwrap_or(false)
    {
        return Ter::TEC_LOCKED;
    }
    if !waive_can_transfer {
        match ledger::mptoken_helpers::can_transfer_mpt(view, &issue, from, to) {
            Ok(Ter::TES_SUCCESS) => {}
            Ok(ter) => return ter,
            Err(_) => return Ter::TEF_BAD_LEDGER,
        }
    }

    let mpt_id = issue.mpt_id();
    let issuer = issue.issuer();
    if *from != issuer {
        let Some(balance) = token_balance(view, mpt_id, from) else {
            return Ter::TEF_BAD_LEDGER;
        };
        if balance < amount {
            return Ter::TEC_INSUFFICIENT_FUNDS;
        }
        let ter = set_token_balance(view, mpt_id, from, balance - amount);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    if *to != issuer {
        let prior_balance = view
            .peek(account_keylet(to_160(to)))
            .ok()
            .flatten()
            .map(|sle| sle.get_field_amount(sf("sfBalance")).xrp())
            .unwrap_or_default();
        let ter = ledger::add_empty_holding(view, to, prior_balance, &Asset::from(issue));
        if ter != Ter::TES_SUCCESS && ter != Ter::TEC_DUPLICATE {
            return ter;
        }
        let Some(balance) = token_balance(view, mpt_id, to) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Some(next) = balance.checked_add(amount) else {
            return Ter::TEF_INTERNAL;
        };
        let ter = set_token_balance(view, mpt_id, to, next);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    let Ok(Some(issuance)) = view.peek(mpt_issuance_keylet_from_mptid(mpt_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut obj = issuance.clone_as_object();
    let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
    match (*from == issuer, *to == issuer) {
        (true, false) => obj.set_field_u64(sf("sfOutstandingAmount"), outstanding + amount),
        (false, true) => obj.set_field_u64(
            sf("sfOutstandingAmount"),
            outstanding.saturating_sub(amount),
        ),
        _ => {}
    }
    view.update(Arc::new(STLedgerEntry::from_stobject(obj, *issuance.key())))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn check_cover_sendable<V: ApplyView>(view: &mut V, account: &AccountID, asset: Asset) -> Ter {
    match asset {
        Asset::Issue(issue) if issue.native() => Ter::TES_SUCCESS,
        Asset::Issue(issue) => {
            if ledger::ripple_state_helpers::is_frozen(view, account, &issue) {
                Ter::TEC_FROZEN
            } else {
                Ter::TES_SUCCESS
            }
        }
        Asset::MPTIssue(issue) => {
            if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(false) {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            }
        }
    }
}

fn asset_deep_frozen<V: ApplyView>(view: &mut V, account: &AccountID, asset: Asset) -> bool {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == *account => false,
        Asset::Issue(issue) => {
            let Some(line) = view
                .peek(protocol::line(*account, issue.account, issue.currency))
                .ok()
                .flatten()
                .or_else(|| {
                    view.read(protocol::line(*account, issue.account, issue.currency))
                        .ok()
                        .flatten()
                })
            else {
                return false;
            };
            line.is_flag(protocol::lsfLowDeepFreeze) || line.is_flag(protocol::lsfHighDeepFreeze)
        }
        Asset::MPTIssue(issue) => {
            ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(true)
        }
    }
}

fn check_asset_deep_frozen<V: ApplyView>(view: &mut V, account: &AccountID, asset: Asset) -> Ter {
    if !asset_deep_frozen(view, account, asset) {
        return Ter::TES_SUCCESS;
    }
    match asset {
        Asset::MPTIssue(_) => Ter::TEC_LOCKED,
        Asset::Issue(_) => Ter::TEC_FROZEN,
    }
}

fn asset_requires_strong_auth<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> bool {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == *account => false,
        Asset::Issue(issue) => {
            let line_keylet = protocol::line(*account, issue.account, issue.currency);
            let trust_line = view
                .peek(line_keylet)
                .ok()
                .flatten()
                .or_else(|| view.read(line_keylet).ok().flatten());
            let Some(trust_line) = trust_line else {
                return true;
            };

            let issuer_keylet = protocol::account_keylet(to_160(&issue.account));
            if let Some(issuer) = view
                .peek(issuer_keylet)
                .ok()
                .flatten()
                .or_else(|| view.read(issuer_keylet).ok().flatten())
                && issuer.is_flag(protocol::lsfRequireAuth)
            {
                let auth_flag = if *account > issue.account {
                    protocol::lsfLowAuth
                } else {
                    protocol::lsfHighAuth
                };
                return !trust_line.is_flag(auth_flag);
            }

            false
        }
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
            .map(|ter| ter != Ter::TES_SUCCESS)
            .unwrap_or(true),
    }
}

fn check_mpt_cover_destination_auth<V: ApplyView>(
    view: &mut V,
    destination: &AccountID,
    issue: &MPTIssue,
    require_holding: bool,
) -> Ter {
    if require_holding
        && view
            .read(mptoken_keylet_from_mptid(
                issue.mpt_id(),
                to_160(destination),
            ))
            .ok()
            .flatten()
            .is_none()
    {
        return Ter::TEC_NO_AUTH;
    }

    ledger::mptoken_helpers::require_auth_mpt(view, issue, destination)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn check_mpt_cover_transfer<V: ApplyView>(
    view: &mut V,
    source: &AccountID,
    destination: &AccountID,
    owner: &AccountID,
    asset: Asset,
    waive_can_transfer: bool,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();

    if source != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, source, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }
    if destination != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, destination, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }

    if !waive_can_transfer {
        let transfer = ledger::mptoken_helpers::can_transfer_mpt(view, &issue, source, destination)
            .unwrap_or(Ter::TEF_BAD_LEDGER);
        if transfer != Ter::TES_SUCCESS {
            return transfer;
        }
    }

    let auth = check_mpt_cover_destination_auth(view, destination, &issue, destination != owner);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }

    Ter::TES_SUCCESS
}

fn with_asset_number(value: RuntimeNumber, asset: Asset) -> STNumber {
    let mut number = STNumber::from(value);
    number.associate_asset(asset);
    number
}

#[derive(Clone)]
struct BrokerCoverState {
    key: Uint256,
    owner: AccountID,
    vault_id: Uint256,
    pseudo_account: AccountID,
    cover_available: RuntimeNumber,
    debt_total: RuntimeNumber,
    cover_rate_minimum: u32,
    cover_asset: Asset,
}

#[derive(Clone)]
struct VaultCoverState {
    entry: STLedgerEntry,
    asset: Asset,
}

fn load_broker<V: ApplyView>(view: &mut V, broker_id: Uint256) -> Option<BrokerCoverState> {
    let broker_sle = view
        .peek(protocol::loan_broker_keylet_from_key(broker_id))
        .ok()
        .flatten()?;
    Some(BrokerCoverState {
        key: *broker_sle.key(),
        owner: broker_sle.get_account_id(sf("sfOwner")),
        vault_id: broker_sle.get_field_h256(sf("sfVaultID")),
        pseudo_account: broker_sle.get_account_id(sf("sfAccount")),
        cover_available: broker_sle.get_field_number(sf("sfCoverAvailable")).value(),
        debt_total: if broker_sle.is_field_present(sf("sfDebtTotal")) {
            broker_sle.get_field_number(sf("sfDebtTotal")).value()
        } else {
            RuntimeNumber::zero()
        },
        cover_rate_minimum: if broker_sle.is_field_present(sf("sfCoverRateMinimum")) {
            broker_sle.get_field_u32(sf("sfCoverRateMinimum"))
        } else {
            0
        },
        cover_asset: broker_sle.get_field_issue(sf("sfAsset")).asset(),
    })
}

fn load_vault<V: ApplyView>(view: &mut V, vault_id: Uint256) -> Option<VaultCoverState> {
    let vault_sle = view
        .peek(protocol::vault_keylet_from_key(vault_id))
        .ok()
        .flatten()?;
    Some(VaultCoverState {
        entry: (*vault_sle).clone(),
        asset: vault_sle.get_field_issue(sf("sfAsset")).asset(),
    })
}

fn persist_broker_cover<V: ApplyView>(
    view: &mut V,
    broker_id: Uint256,
    broker: &BrokerCoverState,
) -> Ter {
    let Ok(Some(sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut obj = sle.clone_as_object();
    obj.set_field_number(
        sf("sfCoverAvailable"),
        with_asset_number(broker.cover_available, broker.cover_asset),
    );
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, broker.key)));
    Ter::TES_SUCCESS
}

fn runtime_number_floor_to_u32(value: RuntimeNumber) -> u32 {
    if value <= RuntimeNumber::zero() {
        return 0;
    }

    let Ok((mantissa, exponent)) = value.external_parts() else {
        return u32::MAX;
    };
    if mantissa <= 0 {
        return 0;
    }

    let mut magnitude = mantissa as u128;
    if exponent >= 0 {
        for _ in 0..exponent {
            magnitude = magnitude.saturating_mul(10);
            if magnitude > u128::from(u32::MAX) {
                return u32::MAX;
            }
        }
    } else {
        for _ in 0..(-exponent) {
            magnitude /= 10;
            if magnitude == 0 {
                return 0;
            }
        }
    }

    u32::try_from(magnitude).unwrap_or(u32::MAX)
}

fn runtime_number_ceil_to_u64(value: RuntimeNumber) -> u64 {
    if value <= RuntimeNumber::zero() {
        return 0;
    }

    let Ok((mantissa, exponent)) = value.external_parts() else {
        return u64::MAX;
    };
    if mantissa <= 0 {
        return 0;
    }

    let mut magnitude = mantissa as u128;
    let mut remainder = false;
    if exponent >= 0 {
        for _ in 0..exponent {
            magnitude = magnitude.saturating_mul(10);
            if magnitude > u128::from(u64::MAX) {
                return u64::MAX;
            }
        }
    } else {
        for _ in 0..(-exponent) {
            remainder |= magnitude % 10 != 0;
            magnitude /= 10;
        }
    }

    if remainder {
        magnitude = magnitude.saturating_add(1);
    }
    u64::try_from(magnitude).unwrap_or(u64::MAX)
}

pub fn calculate_loan_pay_base_fee<V: ReadView>(view: &V, sttx: &STTx, normal_cost: u64) -> u64 {
    if sttx.is_flag(protocol::tfLoanFullPayment) || sttx.is_flag(protocol::tfLoanLatePayment) {
        return normal_cost;
    }

    let loan_id = sttx.get_field_h256(sf("sfLoanID"));
    let Ok(Some(loan_sle)) = view.read(protocol::loan_keylet_from_key(loan_id)) else {
        return normal_cost;
    };

    let payments_remaining = loan_sle.get_field_u32(sf("sfPaymentRemaining"));
    if payments_remaining <= tx::LOAN_PAYMENTS_PER_FEE_INCREMENT {
        return normal_cost;
    }
    if has_expired(
        view,
        loan_sle
            .is_field_present(sf("sfNextPaymentDueDate"))
            .then(|| loan_sle.get_field_u32(sf("sfNextPaymentDueDate"))),
    ) {
        return normal_cost;
    }

    let Ok(Some(broker_sle)) = view.read(protocol::loan_broker_keylet_from_key(
        loan_sle.get_field_h256(sf("sfLoanBrokerID")),
    )) else {
        return normal_cost;
    };
    let Ok(Some(vault_sle)) = view.read(protocol::vault_keylet_from_key(
        broker_sle.get_field_h256(sf("sfVaultID")),
    )) else {
        return normal_cost;
    };

    let amount = sttx.get_field_amount(sf("sfAmount"));
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    if amount.asset() != vault_asset {
        return normal_cost;
    }

    let periodic_payment = loan_sle.get_field_number(sf("sfPeriodicPayment")).value();
    let service_fee = if loan_sle.is_field_present(sf("sfLoanServiceFee")) {
        amount_number(&loan_sle.get_field_amount(sf("sfLoanServiceFee")))
    } else {
        RuntimeNumber::zero()
    };
    let loan_scale = loan_sle.get_field_i32(sf("sfLoanScale"));
    let regular_payment = round_number_to_asset_with_scale(
        vault_asset,
        periodic_payment,
        loan_scale,
        RoundingMode::Upward,
    ) + service_fee;
    if regular_payment <= RuntimeNumber::zero() {
        return normal_cost;
    }

    let payment_amount = amount_number(&amount);
    let fix_security_3_1_3 = view.rules().enabled(&feature_id("fixSecurity3_1_3"));
    if fix_security_3_1_3
        && payment_amount
            >= regular_payment
                * RuntimeNumber::from_i64(i64::from(LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION))
    {
        return normal_cost.saturating_mul(tx::LOAN_MAXIMUM_FEE_INCREMENTS);
    }

    let estimate = payment_amount / regular_payment;
    let payment_estimate = if sttx.is_flag(protocol::tfLoanOverpayment) {
        runtime_number_ceil_to_u64(estimate)
    } else {
        u64::from(runtime_number_floor_to_u32(estimate))
    };
    let increments =
        tx::compute_loan_pay_fee_increments(i64::try_from(payment_estimate).unwrap_or(i64::MAX));
    let increments = if fix_security_3_1_3 {
        increments.min(tx::LOAN_MAXIMUM_FEE_INCREMENTS)
    } else {
        increments
    };
    normal_cost.saturating_mul(increments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_number_floor_to_u32_handles_integral_fractional_and_large_values() {
        assert_eq!(runtime_number_floor_to_u32(RuntimeNumber::from_i64(3)), 3);
        assert_eq!(
            runtime_number_floor_to_u32(RuntimeNumber::from_i64_and_exponent(39, -1)),
            3
        );
        assert_eq!(runtime_number_floor_to_u32(RuntimeNumber::zero()), 0);
        assert_eq!(
            runtime_number_floor_to_u32(RuntimeNumber::from_i64_and_exponent(5, 12)),
            u32::MAX
        );
    }
}

fn persist_entry<V: ApplyView>(view: &mut V, entry: STLedgerEntry) -> Ter {
    view.update(Arc::new(entry))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn account_balance_drops(sle: &STLedgerEntry) -> i64 {
    sle.get_field_amount(sf("sfBalance")).xrp().drops()
}

fn associate_asset_entry(entry: &mut STLedgerEntry, asset: Asset) {
    protocol::associate_asset(entry, asset);
}

fn round_number_to_asset(asset: Asset, value: RuntimeNumber) -> RuntimeNumber {
    let mut number = STNumber::from(value);
    number.associate_asset(asset);
    number.value()
}

fn vault_scale(vault_sle: &STLedgerEntry, asset: Asset) -> i32 {
    if asset.integral() {
        return 0;
    }
    if vault_sle.is_field_present(sf("sfScale")) {
        return -(vault_sle.get_field_u8(sf("sfScale")) as i32);
    }
    asset
        .amount(vault_sle.get_field_number(sf("sfAssetsTotal")).value())
        .map(|amount| amount.exponent())
        .unwrap_or(0)
}

fn round_runtime_to_scale(
    value: RuntimeNumber,
    target_scale: i32,
    rounding: RoundingMode,
) -> RuntimeNumber {
    let Ok((mantissa, mut exponent)) = value.external_parts() else {
        return value;
    };
    if mantissa == 0 || exponent >= target_scale {
        return value;
    }

    let negative = mantissa < 0;
    let mut abs = mantissa.unsigned_abs() as u128;
    let mut removed = Vec::new();
    while exponent < target_scale {
        removed.push((abs % 10) as u8);
        abs /= 10;
        exponent += 1;
    }

    let first = removed.first().copied().unwrap_or(0);
    let has_more = removed.iter().skip(1).any(|digit| *digit != 0);
    let round_up = match rounding {
        RoundingMode::TowardsZero => false,
        RoundingMode::Downward => negative && (first != 0 || has_more),
        RoundingMode::Upward => !negative && (first != 0 || has_more),
        RoundingMode::ToNearest => {
            first > 5 || (first == 5 && (has_more || ((abs as u64) & 1) == 1))
        }
    };
    if round_up {
        abs += 1;
    }

    let signed = if negative { -(abs as i64) } else { abs as i64 };
    RuntimeNumber::try_from_external_parts(signed, exponent, get_mantissa_scale()).unwrap_or(value)
}

fn round_number_to_asset_with_scale(
    asset: Asset,
    value: RuntimeNumber,
    scale: i32,
    rounding: RoundingMode,
) -> RuntimeNumber {
    let rounded_to_asset = round_number_to_asset(asset, value);
    if asset.integral() {
        return round_runtime_to_scale(value, 0, rounding);
    }
    round_runtime_to_scale(rounded_to_asset, scale, rounding)
}

fn adjust_imprecise_number(
    value: RuntimeNumber,
    adjustment: RuntimeNumber,
    asset: Asset,
    scale: i32,
) -> RuntimeNumber {
    let adjusted =
        round_number_to_asset_with_scale(asset, value + adjustment, scale, RoundingMode::ToNearest);
    if adjusted < RuntimeNumber::zero() {
        RuntimeNumber::zero()
    } else {
        adjusted
    }
}

fn effective_loan_pay_amount(
    payment_type: tx::LoanPayPaymentType,
    fix_security_3_1_3: bool,
    fix_cleanup_3_2_0: bool,
    asset: Asset,
    payment_amount: RuntimeNumber,
    loan_scale: i32,
) -> RuntimeNumber {
    if payment_type == tx::LoanPayPaymentType::Overpayment
        && (fix_security_3_1_3 || fix_cleanup_3_2_0)
    {
        round_number_to_asset_with_scale(
            asset,
            payment_amount,
            loan_scale,
            RoundingMode::TowardsZero,
        )
    } else {
        payment_amount
    }
}

#[derive(Debug, Clone, Copy)]
struct LoanPayComponentParts {
    principal_paid: RuntimeNumber,
    interest_paid: RuntimeNumber,
    management_fee_paid: RuntimeNumber,
}

#[allow(clippy::too_many_arguments)]
fn compute_loan_pay_periodic_components(
    rules: &protocol::Rules,
    asset: Asset,
    loan_scale: i32,
    total_value_outstanding: RuntimeNumber,
    principal_outstanding: RuntimeNumber,
    management_fee_outstanding: RuntimeNumber,
    periodic_payment: RuntimeNumber,
    interest_rate: TenthBips32,
    payment_interval: u32,
    payments_remaining: u32,
    management_fee_rate: TenthBips16,
) -> LoanPayComponentParts {
    let rounded_periodic_payment =
        round_number_to_asset_with_scale(asset, periodic_payment, loan_scale, RoundingMode::Upward);
    if payments_remaining <= 1 || total_value_outstanding <= rounded_periodic_payment {
        let interest_paid =
            total_value_outstanding - principal_outstanding - management_fee_outstanding;
        return LoanPayComponentParts {
            principal_paid: principal_outstanding,
            interest_paid: interest_paid.max(RuntimeNumber::zero()),
            management_fee_paid: management_fee_outstanding,
        };
    }

    let periodic_rate = tx::loan_set_periodic_rate(interest_rate, payment_interval);
    let target = tx::compute_theoretical_loan_state(
        rules,
        asset,
        periodic_payment,
        periodic_rate,
        payments_remaining.saturating_sub(1),
        management_fee_rate,
    );
    let fix_cleanup_3_2_0 = rules.enabled(&feature_id("fixCleanup3_2_0"));
    let principal_rounding = if fix_cleanup_3_2_0 {
        RoundingMode::Upward
    } else {
        RoundingMode::ToNearest
    };
    let interest_rounding = if fix_cleanup_3_2_0 {
        RoundingMode::Downward
    } else {
        RoundingMode::ToNearest
    };
    let rounded_target_value = round_number_to_asset_with_scale(
        asset,
        target.value_outstanding,
        loan_scale,
        RoundingMode::ToNearest,
    );
    let rounded_target_principal = round_number_to_asset_with_scale(
        asset,
        target.principal_outstanding,
        loan_scale,
        principal_rounding,
    );
    let rounded_target_interest =
        round_number_to_asset_with_scale(asset, target.interest_due, loan_scale, interest_rounding);
    let rounded_target_management_fee = round_number_to_asset_with_scale(
        asset,
        target.management_fee_due,
        loan_scale,
        RoundingMode::ToNearest,
    );

    let current_interest =
        (total_value_outstanding - principal_outstanding - management_fee_outstanding)
            .max(RuntimeNumber::zero());
    let mut principal_paid =
        (principal_outstanding - rounded_target_principal).max(RuntimeNumber::zero());
    let mut interest_paid = (current_interest - rounded_target_interest).max(RuntimeNumber::zero());
    let mut management_fee_paid =
        (management_fee_outstanding - rounded_target_management_fee).max(RuntimeNumber::zero());

    principal_paid = principal_paid.min(principal_outstanding);
    interest_paid = interest_paid
        .min(current_interest)
        .min((rounded_periodic_payment - principal_paid).max(RuntimeNumber::zero()));
    management_fee_paid = management_fee_paid.min(management_fee_outstanding).min(
        (rounded_periodic_payment - principal_paid - interest_paid).max(RuntimeNumber::zero()),
    );

    let mut total_value_paid = principal_paid + interest_paid + management_fee_paid;
    if total_value_paid > total_value_outstanding {
        total_value_paid = total_value_outstanding;
    }
    if total_value_paid > rounded_periodic_payment {
        let excess = total_value_paid - rounded_periodic_payment;
        let interest_reduction = interest_paid.min(excess);
        interest_paid -= interest_reduction;
        let remaining_excess = excess - interest_reduction;
        let fee_reduction = management_fee_paid.min(remaining_excess);
        management_fee_paid -= fee_reduction;
        let remaining_excess = remaining_excess - fee_reduction;
        principal_paid -= principal_paid.min(remaining_excess);
        total_value_paid = principal_paid + interest_paid + management_fee_paid;
    }
    if total_value_paid == RuntimeNumber::zero() && rounded_target_value < total_value_outstanding {
        interest_paid = current_interest.min(rounded_periodic_payment);
    }

    LoanPayComponentParts {
        principal_paid,
        interest_paid,
        management_fee_paid,
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_loan_pay_scheduled_payment_loop(
    rules: &protocol::Rules,
    asset: Asset,
    loan_scale: i32,
    payment_amount: RuntimeNumber,
    total_value_outstanding: RuntimeNumber,
    principal_outstanding: RuntimeNumber,
    management_fee_outstanding: RuntimeNumber,
    periodic_payment: RuntimeNumber,
    interest_rate: TenthBips32,
    payment_interval: u32,
    payments_remaining: u32,
    management_fee_rate: TenthBips16,
    service_fee: RuntimeNumber,
) -> (
    RuntimeNumber,
    RuntimeNumber,
    RuntimeNumber,
    RuntimeNumber,
    RuntimeNumber,
    u32,
) {
    let mut current_total = total_value_outstanding;
    let mut current_principal = principal_outstanding;
    let mut current_management_fee = management_fee_outstanding;
    let mut current_remaining = payments_remaining;

    let mut principal_paid = RuntimeNumber::zero();
    let mut interest_paid = RuntimeNumber::zero();
    let mut management_fee_paid = RuntimeNumber::zero();
    let mut fee_paid = RuntimeNumber::zero();
    let mut total_paid = RuntimeNumber::zero();
    let mut periods_paid = 0_u32;

    while current_remaining > 0
        && periods_paid < tx::LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION
        && current_total > RuntimeNumber::zero()
    {
        let components = compute_loan_pay_periodic_components(
            rules,
            asset,
            loan_scale,
            current_total,
            current_principal,
            current_management_fee,
            periodic_payment,
            interest_rate,
            payment_interval,
            current_remaining,
            management_fee_rate,
        );
        let tracked_due =
            components.principal_paid + components.interest_paid + components.management_fee_paid;
        let period_due = tracked_due + service_fee;
        if period_due <= RuntimeNumber::zero() || total_paid + period_due > payment_amount {
            break;
        }

        principal_paid += components.principal_paid;
        interest_paid += components.interest_paid;
        management_fee_paid += components.management_fee_paid;
        fee_paid += components.management_fee_paid + service_fee;
        total_paid += period_due;

        current_principal =
            (current_principal - components.principal_paid).max(RuntimeNumber::zero());
        current_management_fee =
            (current_management_fee - components.management_fee_paid).max(RuntimeNumber::zero());
        current_total = (current_total - tracked_due).max(RuntimeNumber::zero());
        current_remaining = current_remaining.saturating_sub(1);
        periods_paid += 1;
    }

    (
        principal_paid,
        interest_paid,
        management_fee_paid,
        fee_paid,
        RuntimeNumber::zero(),
        periods_paid,
    )
}

fn zero_asset_number(asset: Asset) -> STNumber {
    with_asset_number(RuntimeNumber::zero(), asset)
}

fn tenth_bips_of_runtime_number(value: RuntimeNumber, rate: u32) -> RuntimeNumber {
    value * RuntimeNumber::from_i64(i64::from(rate)) / RuntimeNumber::from_i64(100_000)
}

fn compute_interest_and_fee_parts(
    asset: Asset,
    interest: RuntimeNumber,
    management_fee_rate: TenthBips16,
    loan_scale: i32,
) -> (RuntimeNumber, RuntimeNumber) {
    let fee = tx::compute_management_fee(asset, interest, management_fee_rate, loan_scale);
    (interest - fee, fee)
}

fn loan_accrued_interest(
    principal_outstanding: RuntimeNumber,
    periodic_rate: RuntimeNumber,
    parent_close_time: u32,
    start_date: u32,
    previous_payment_date: u32,
    payment_interval: u32,
) -> RuntimeNumber {
    if periodic_rate == RuntimeNumber::zero() || payment_interval == 0 {
        return RuntimeNumber::zero();
    }

    let last_payment_date = previous_payment_date.max(start_date);
    if parent_close_time <= last_payment_date {
        return RuntimeNumber::zero();
    }

    principal_outstanding
        * periodic_rate
        * RuntimeNumber::from_i64(i64::from(parent_close_time - last_payment_date))
        / RuntimeNumber::from_i64(i64::from(payment_interval))
}

fn loan_late_payment_interest(
    principal_outstanding: RuntimeNumber,
    late_interest_rate: TenthBips32,
    parent_close_time: u32,
    next_payment_due_date: u32,
) -> RuntimeNumber {
    if principal_outstanding == RuntimeNumber::zero() || late_interest_rate.value() == 0 {
        return RuntimeNumber::zero();
    }

    if parent_close_time <= next_payment_due_date {
        return RuntimeNumber::zero();
    }

    let seconds_overdue = parent_close_time - next_payment_due_date;
    principal_outstanding * tx::loan_set_periodic_rate(late_interest_rate, seconds_overdue)
}

#[allow(clippy::too_many_arguments)]
fn compute_full_payment_parts(
    rules: &protocol::Rules,
    asset: Asset,
    loan_scale: i32,
    parent_close_time: u32,
    total_interest_outstanding: RuntimeNumber,
    periodic_payment: RuntimeNumber,
    periodic_rate: RuntimeNumber,
    payments_remaining: u32,
    previous_payment_date: u32,
    start_date: u32,
    payment_interval: u32,
    close_interest_rate: TenthBips32,
    close_payment_fee: RuntimeNumber,
    management_fee_rate: TenthBips16,
) -> (RuntimeNumber, RuntimeNumber, RuntimeNumber) {
    let theoretical_principal = tx::loan_principal_from_periodic_payment(
        rules,
        periodic_payment,
        periodic_rate,
        payments_remaining,
    );
    let accrued_interest = loan_accrued_interest(
        theoretical_principal,
        periodic_rate,
        parent_close_time,
        start_date,
        previous_payment_date,
        payment_interval,
    );
    let prepayment_penalty =
        tenth_bips_of_runtime_number(theoretical_principal, close_interest_rate.value());
    let full_interest = round_number_to_asset_with_scale(
        asset,
        accrued_interest + prepayment_penalty,
        loan_scale,
        RoundingMode::Downward,
    );
    let (net_interest, management_fee) =
        compute_interest_and_fee_parts(asset, full_interest, management_fee_rate, loan_scale);
    let value_change = net_interest - total_interest_outstanding;
    let fee_paid = close_payment_fee + management_fee;
    (net_interest, fee_paid, value_change)
}

struct OverpaymentReamortization {
    principal_paid: RuntimeNumber,
    interest_paid: RuntimeNumber,
    management_fee_paid: RuntimeNumber,
    fee_paid: RuntimeNumber,
    value_change: RuntimeNumber,
    periodic_payment: RuntimeNumber,
}

#[allow(clippy::too_many_arguments)]
fn compute_overpayment_reamortization(
    rules: &protocol::Rules,
    asset: Asset,
    loan_scale: i32,
    overpayment: RuntimeNumber,
    old_total_value: RuntimeNumber,
    old_principal: RuntimeNumber,
    old_management_fee: RuntimeNumber,
    periodic_payment: RuntimeNumber,
    interest_rate: TenthBips32,
    payment_interval: u32,
    periodic_rate: RuntimeNumber,
    payment_remaining: u32,
    management_fee_rate: TenthBips16,
    overpayment_interest_rate: TenthBips32,
    overpayment_fee_rate: TenthBips32,
) -> Option<OverpaymentReamortization> {
    if overpayment <= RuntimeNumber::zero() || payment_remaining == 0 {
        return None;
    }

    let overpayment_fee = round_number_to_asset_with_scale(
        asset,
        tenth_bips_of_runtime_number(overpayment, overpayment_fee_rate.value()),
        loan_scale,
        RoundingMode::ToNearest,
    );
    let overpayment_interest = round_number_to_asset_with_scale(
        asset,
        tenth_bips_of_runtime_number(overpayment, overpayment_interest_rate.value()),
        loan_scale,
        RoundingMode::ToNearest,
    );
    let (net_overpayment_interest, overpayment_management_fee) = compute_interest_and_fee_parts(
        asset,
        overpayment_interest,
        management_fee_rate,
        loan_scale,
    );
    let tracked_principal_delta =
        overpayment - net_overpayment_interest - overpayment_management_fee - overpayment_fee;
    if tracked_principal_delta <= RuntimeNumber::zero() {
        return None;
    }

    let old_interest =
        (old_total_value - old_principal - old_management_fee).max(RuntimeNumber::zero());
    let old_state =
        tx::construct_loan_set_state(old_total_value, old_principal, old_management_fee);
    let theoretical_state = tx::compute_theoretical_loan_state(
        rules,
        asset,
        periodic_payment,
        periodic_rate,
        payment_remaining,
        management_fee_rate,
    );
    let value_error = old_state.value_outstanding - theoretical_state.value_outstanding;
    let principal_error = old_state.principal_outstanding - theoretical_state.principal_outstanding;
    let interest_error = old_state.interest_due - theoretical_state.interest_due;
    let management_error = old_state.management_fee_due - theoretical_state.management_fee_due;

    let new_theoretical_principal = (theoretical_state.principal_outstanding
        - tracked_principal_delta)
        .max(RuntimeNumber::zero());

    let mut new_loan_properties = tx::compute_loan_set_properties(
        rules,
        asset,
        new_theoretical_principal,
        interest_rate,
        payment_interval,
        payment_remaining,
        management_fee_rate,
        loan_scale,
    );
    let new_theoretical_state = tx::compute_theoretical_loan_state(
        rules,
        asset,
        new_loan_properties.periodic_payment,
        periodic_rate,
        payment_remaining,
        management_fee_rate,
    );
    let new_theoretical_state = tx::LoanSetLoanState {
        value_outstanding: new_theoretical_state.value_outstanding + value_error,
        principal_outstanding: new_theoretical_state.principal_outstanding + principal_error,
        interest_due: new_theoretical_state.interest_due + interest_error,
        management_fee_due: new_theoretical_state.management_fee_due + management_error,
    };

    let new_principal = round_number_to_asset_with_scale(
        asset,
        new_theoretical_state.principal_outstanding,
        loan_scale,
        RoundingMode::Upward,
    )
    .max(RuntimeNumber::zero())
    .min(old_principal);
    let total_value_outstanding = round_number_to_asset_with_scale(
        asset,
        new_principal + new_theoretical_state.interest_due,
        loan_scale,
        RoundingMode::Upward,
    )
    .max(RuntimeNumber::zero())
    .min(old_total_value);
    let new_management_fee = round_number_to_asset_with_scale(
        asset,
        new_theoretical_state.management_fee_due,
        loan_scale,
        RoundingMode::ToNearest,
    )
    .max(RuntimeNumber::zero())
    .min(old_management_fee);
    let rounded_new_state =
        tx::construct_loan_set_state(total_value_outstanding, new_principal, new_management_fee);
    new_loan_properties.loan_state = rounded_new_state.clone();

    let guards = tx::LoanSetLoanGuardProperties {
        periodic_payment: new_loan_properties.periodic_payment,
        total_value_outstanding: rounded_new_state.value_outstanding,
        loan_scale: new_loan_properties.loan_scale,
        first_payment_principal: new_loan_properties.first_payment_principal,
    };
    if tx::check_loan_set_loan_guards(
        &asset,
        &rounded_new_state.principal_outstanding,
        rounded_new_state.interest_due != RuntimeNumber::zero(),
        payment_remaining,
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
    )
    .is_err()
    {
        return None;
    }

    if new_loan_properties.periodic_payment <= RuntimeNumber::zero()
        || rounded_new_state.value_outstanding <= RuntimeNumber::zero()
        || rounded_new_state.management_fee_due < RuntimeNumber::zero()
    {
        return None;
    }

    if rounded_new_state.principal_outstanding >= old_principal {
        return None;
    }

    let principal_paid = old_principal - rounded_new_state.principal_outstanding;
    let management_fee_paid = old_management_fee - rounded_new_state.management_fee_due;
    let new_interest = rounded_new_state.interest_due.max(RuntimeNumber::zero());
    let value_change = new_interest - old_interest + net_overpayment_interest;
    if value_change > net_overpayment_interest {
        return None;
    }

    Some(OverpaymentReamortization {
        principal_paid,
        interest_paid: net_overpayment_interest,
        management_fee_paid,
        fee_paid: overpayment_fee + overpayment_management_fee,
        value_change,
        periodic_payment: new_loan_properties.periodic_payment,
    })
}

#[derive(Clone)]
struct LoanSetTxView<'a> {
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
struct LoanSetBrokerView {
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
struct LoanSetVaultView {
    entry: STLedgerEntry,
    pseudo_account: AccountID,
    asset: Asset,
    assets_available: RuntimeNumber,
    assets_total: RuntimeNumber,
    assets_maximum: RuntimeNumber,
}

#[derive(Clone)]
struct LoanSetAccountState {
    balance_drops: i64,
}

#[derive(Clone)]
struct LoanSetProperties {
    inner: tx::LoanSetLoanProperties<RuntimeNumber>,
}

#[derive(Clone)]
struct LoanSetState {
    inner: tx::LoanSetLoanState<RuntimeNumber>,
}

#[derive(Clone)]
struct LoanSetAmountValue {
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

fn amount_number(amount: &STAmount) -> RuntimeNumber {
    amount.as_number()
}

fn asset_scale_from_value(asset: Asset, value: RuntimeNumber) -> i32 {
    if asset.integral() {
        return 0;
    }
    asset
        .amount(value)
        .map(|amount| amount.exponent())
        .unwrap_or(0)
}

fn minimum_broker_cover(
    asset: Asset,
    debt_total: RuntimeNumber,
    cover_rate_minimum: u32,
    vault_sle: &STLedgerEntry,
    fix_cleanup_3_2_0: bool,
) -> RuntimeNumber {
    let scale = if fix_cleanup_3_2_0 {
        vault_scale(vault_sle, asset)
    } else {
        asset_scale_from_value(asset, debt_total)
    };
    round_number_to_asset_with_scale(
        asset,
        tenth_bips_of_runtime_number(debt_total, cover_rate_minimum),
        scale,
        RoundingMode::Upward,
    )
}

fn loan_pay_fee_route_minimum_cover(
    asset: Asset,
    debt_total: RuntimeNumber,
    cover_rate_minimum: u32,
    loan_scale: i32,
    vault_scale: i32,
    fix_cleanup_3_2_0: bool,
) -> RuntimeNumber {
    let scale = if fix_cleanup_3_2_0 {
        vault_scale
    } else {
        loan_scale
    };
    round_number_to_asset_with_scale(
        asset,
        tenth_bips_of_runtime_number(debt_total, cover_rate_minimum),
        scale,
        RoundingMode::Upward,
    )
}

fn loan_set_debt_total_update(
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

fn is_pseudo_account<V: ApplyView>(view: &mut V, account: &AccountID) -> bool {
    let Ok(Some(account_sle)) = view.peek(account_keylet(to_160(account))) else {
        return false;
    };
    account_sle.is_field_present(sf("sfVaultID"))
        || account_sle.is_field_present(sf("sfLoanBrokerID"))
}

fn runtime_to_amount(
    asset: Asset,
    value: RuntimeNumber,
    rounding: RoundingMode,
) -> Option<STAmount> {
    to_amount_from_number(asset, value, rounding).ok()
}

fn rounded_cover_deposit_amount(
    asset: Asset,
    cover_available: RuntimeNumber,
    amount: &STAmount,
    fix_cleanup_3_2_0: bool,
) -> Option<(STAmount, RuntimeNumber)> {
    if !fix_cleanup_3_2_0 {
        return Some((amount.clone(), amount_number(amount)));
    }

    let scale = asset_scale_from_value(asset, cover_available);
    let rounded = round_number_to_asset_with_scale(
        asset,
        amount_number(amount),
        scale,
        RoundingMode::Downward,
    );
    runtime_to_amount(asset, rounded, RoundingMode::Downward).map(|amount| (amount, rounded))
}

fn cover_amount_is_zero_at_cover_scale(
    asset: Asset,
    cover_available: RuntimeNumber,
    amount: &STAmount,
    fix_cleanup_3_2_0: bool,
) -> bool {
    if !fix_cleanup_3_2_0 {
        return false;
    }

    let scale = asset_scale_from_value(asset, cover_available);
    round_number_to_asset_with_scale(asset, amount_number(amount), scale, RoundingMode::ToNearest)
        == RuntimeNumber::zero()
}

fn load_loan_set_tx_view(sttx: &STTx) -> LoanSetTxView<'_> {
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

fn load_loan_set_broker_view(entry: STLedgerEntry) -> LoanSetBrokerView {
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

fn load_loan_set_vault_view(entry: STLedgerEntry) -> LoanSetVaultView {
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

fn load_loan_set_account_state(entry: STLedgerEntry) -> LoanSetAccountState {
    LoanSetAccountState {
        balance_drops: entry.get_field_amount(sf("sfBalance")).xrp().drops(),
    }
}

fn set_optional_loan_tx_field(
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

pub fn apply_loan_broker_delete<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));

    let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
        broker_sle.get_field_h256(sf("sfVaultID")),
    )) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_pseudo_id = vault_sle.get_account_id(sf("sfAccount"));
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    let broker_pseudo_id = broker_sle.get_account_id(sf("sfAccount"));

    if account != broker_sle.get_account_id(sf("sfOwner")) {
        return Ter::TEC_NO_PERMISSION;
    }
    if broker_sle.get_field_u32(sf("sfOwnerCount")) != 0 {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    let debt_total = broker_sle.get_field_number(sf("sfDebtTotal")).value();
    if debt_total != RuntimeNumber::zero() {
        let rounded = round_number_to_asset_with_scale(
            vault_asset,
            debt_total,
            vault_scale(&vault_sle, vault_asset),
            RoundingMode::TowardsZero,
        );
        if rounded != RuntimeNumber::zero() {
            return Ter::TEC_HAS_OBLIGATIONS;
        }
    }
    let cover_available = broker_sle.get_field_number(sf("sfCoverAvailable")).value();
    if view.rules().enabled(&feature_id("fixCleanup3_2_0"))
        && cover_available > RuntimeNumber::zero()
    {
        let ter = check_cover_sendable(view, &broker_pseudo_id, vault_asset);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
        let ter = check_mpt_cover_transfer(
            view,
            &broker_pseudo_id,
            &account,
            &account,
            vault_asset,
            true,
        );
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&account)),
            broker_sle.get_field_u64(sf("sfOwnerNode")),
            *broker_sle.key(),
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }
    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&vault_pseudo_id)),
            broker_sle.get_field_u64(sf("sfVaultNode")),
            *broker_sle.key(),
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }

    let Ok(cover_amount) = vault_asset.amount(cover_available) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let waive_mpt_can_transfer = view.rules().enabled(&feature_id("fixCleanup3_2_0"));
    let payout = account_send_with_mpt_transfer_waiver(
        view,
        &broker_pseudo_id,
        &account,
        &cover_amount,
        waive_mpt_can_transfer,
    );
    if payout != Ter::TES_SUCCESS {
        return payout;
    }

    let cleanup = ledger::remove_empty_holding(view, &broker_pseudo_id, &vault_asset);
    if cleanup != Ter::TES_SUCCESS {
        return cleanup;
    }

    let Ok(Some(pseudo_sle)) = view.peek(account_keylet(to_160(&broker_pseudo_id))) else {
        return Ter::TEF_BAD_LEDGER;
    };
    if account_balance_drops(&pseudo_sle) != 0 {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    if pseudo_sle.get_field_u32(sf("sfOwnerCount")) != 0 {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    if view
        .read(owner_dir_keylet(to_160(&broker_pseudo_id)))
        .ok()
        .flatten()
        .is_some()
    {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if view.erase(pseudo_sle).is_err() || view.erase(broker_sle.clone()).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }
    let Ok(Some(owner_sle)) = view.peek(account_keylet(to_160(&account))) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let _ = ledger::adjust_owner_count(view, &owner_sle, -2);
    Ter::TES_SUCCESS
}

pub fn apply_loan_delete<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let loan_id = sttx.get_field_h256(sf("sfLoanID"));

    let Ok(Some(loan_sle)) = view.peek(protocol::loan_keylet_from_key(loan_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let borrower_id = loan_sle.get_account_id(sf("sfBorrower"));
    let Ok(Some(borrower_sle)) = view.peek(account_keylet(to_160(&borrower_id))) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let broker_id = loan_sle.get_field_h256(sf("sfLoanBrokerID"));
    let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let broker_pseudo_id = broker_sle.get_account_id(sf("sfAccount"));

    let vault_id = broker_sle.get_field_h256(sf("sfVaultID"));
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(vault_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();

    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&broker_pseudo_id)),
            loan_sle.get_field_u64(sf("sfLoanBrokerNode")),
            loan_id,
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }
    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&borrower_id)),
            loan_sle.get_field_u64(sf("sfOwnerNode")),
            loan_id,
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }

    if view.erase(loan_sle.clone()).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }

    let Some(broker_owner_count) = broker_sle.get_field_u32(sf("sfOwnerCount")).checked_sub(1)
    else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut broker_obj = broker_sle.clone_as_object();
    broker_obj.set_field_u32(sf("sfOwnerCount"), broker_owner_count);
    if broker_owner_count == 0 {
        let debt_total = broker_sle.get_field_number(sf("sfDebtTotal"));
        if debt_total.value() != RuntimeNumber::zero() {
            let rounded = round_number_to_asset(vault_asset, debt_total.value());
            if rounded != RuntimeNumber::zero() {
                return Ter::TEF_BAD_LEDGER;
            }
            broker_obj.set_field_number(
                sf("sfDebtTotal"),
                with_asset_number(RuntimeNumber::zero(), vault_asset),
            );
        }
    }
    let mut broker = STLedgerEntry::from_stobject(broker_obj, *broker_sle.key());
    associate_asset_entry(&mut broker, vault_asset);
    let broker_result = persist_entry(view, broker);
    if broker_result != Ter::TES_SUCCESS {
        return broker_result;
    }

    let _ = ledger::adjust_owner_count(view, &borrower_sle, -1);

    let mut loan = STLedgerEntry::from_stobject(loan_sle.clone_as_object(), *loan_sle.key());
    associate_asset_entry(&mut loan, vault_asset);
    let mut vault = STLedgerEntry::from_stobject(vault_sle.clone_as_object(), *vault_sle.key());
    associate_asset_entry(&mut vault, vault_asset);

    Ter::TES_SUCCESS
}

pub fn apply_loan_broker_cover_deposit<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
    let amount = sttx.get_field_amount(sf("sfAmount"));

    let Some(mut broker) = load_broker(view, broker_id) else {
        return Ter::TEF_INTERNAL;
    };
    let Some(vault) = load_vault(view, broker.vault_id) else {
        return Ter::TEF_INTERNAL;
    };
    if account != broker.owner {
        return Ter::TEC_NO_PERMISSION;
    }
    if amount.asset() != vault.asset {
        return Ter::TEC_WRONG_ASSET;
    }

    let Some((deposit_amount, added)) = rounded_cover_deposit_amount(
        vault.asset,
        broker.cover_available,
        &amount,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    ) else {
        return Ter::TEF_INTERNAL;
    };
    if added == RuntimeNumber::zero() {
        return Ter::TEC_PRECISION_LOSS;
    }

    let transfer_result = account_send(view, &account, &broker.pseudo_account, &deposit_amount);
    if transfer_result != Ter::TES_SUCCESS {
        return transfer_result;
    }

    broker.cover_available += added;
    broker.cover_asset = vault.asset;

    persist_broker_cover(view, broker_id, &broker)
}

pub fn apply_loan_broker_cover_withdraw<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    _pre_fee_balance_drops: i64,
) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let destination = if sttx.is_field_present(sf("sfDestination")) {
        sttx.get_account_id(sf("sfDestination"))
    } else {
        sttx.get_account_id(sf("sfAccount"))
    };
    let account = sttx.get_account_id(sf("sfAccount"));

    let Some(mut broker) = load_broker(view, broker_id) else {
        return Ter::TEC_INTERNAL;
    };
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(broker.vault_id)) else {
        return Ter::TEC_INTERNAL;
    };
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();

    if is_pseudo_account(view, &destination) {
        return Ter::TEC_PSEUDO_ACCOUNT;
    }
    if account != broker.owner {
        return Ter::TEC_NO_PERMISSION;
    }
    if amount.asset() != vault_asset {
        return Ter::TEC_WRONG_ASSET;
    }
    if cover_amount_is_zero_at_cover_scale(
        vault_asset,
        broker.cover_available,
        &amount,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    ) {
        return Ter::TEC_PRECISION_LOSS;
    }

    let deducted = amount.as_number();
    if broker.cover_available < deducted {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    let minimum_cover = minimum_broker_cover(
        vault_asset,
        broker.debt_total,
        broker.cover_rate_minimum,
        &vault_sle,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    );
    if broker.cover_available - deducted < minimum_cover {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    let waive_mpt_can_transfer = view.rules().enabled(&feature_id("fixCleanup3_2_0"));
    let transfer_check = check_mpt_cover_transfer(
        view,
        &broker.pseudo_account,
        &destination,
        &account,
        vault_asset,
        waive_mpt_can_transfer,
    );
    if transfer_check != Ter::TES_SUCCESS {
        return transfer_check;
    }

    broker.cover_available -= deducted;
    broker.cover_asset = vault_asset;

    let update_result = persist_broker_cover(view, broker_id, &broker);
    if update_result != Ter::TES_SUCCESS {
        return update_result;
    }

    account_send_with_mpt_transfer_waiver(
        view,
        &broker.pseudo_account,
        &destination,
        &amount,
        waive_mpt_can_transfer,
    )
}

pub fn apply_loan_broker_cover_clawback<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));

    let Some(mut broker) = load_broker(view, broker_id) else {
        return Ter::TEC_INTERNAL;
    };
    let Some(vault) = load_vault(view, broker.vault_id) else {
        return Ter::TEC_INTERNAL;
    };

    if vault.asset.native() || asset_issuer(vault.asset) != account {
        return Ter::TEC_NO_PERMISSION;
    }

    let requested = sttx
        .is_field_present(sf("sfAmount"))
        .then(|| sttx.get_field_amount(sf("sfAmount")));
    if let Some(amount) = &requested
        && amount.asset() != vault.asset
    {
        return Ter::TEC_WRONG_ASSET;
    }

    let minimum_cover = minimum_broker_cover(
        vault.asset,
        broker.debt_total,
        broker.cover_rate_minimum,
        &vault.entry,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    );
    let max_claw_amount = broker.cover_available - minimum_cover;
    if max_claw_amount <= RuntimeNumber::zero() {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    let deducted = requested
        .as_ref()
        .filter(|amount| amount.signum() > 0)
        .map(amount_number)
        .unwrap_or(max_claw_amount)
        .min(max_claw_amount);
    let Some(claw_amount) = runtime_to_amount(vault.asset, deducted, RoundingMode::Downward) else {
        return Ter::TEC_INTERNAL;
    };
    if claw_amount.signum() == 0 {
        return Ter::TEC_PRECISION_LOSS;
    }
    if cover_amount_is_zero_at_cover_scale(
        vault.asset,
        broker.cover_available,
        &claw_amount,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    ) {
        return Ter::TEC_PRECISION_LOSS;
    }

    broker.cover_available -= deducted;
    broker.cover_asset = vault.asset;

    let update_result = persist_broker_cover(view, broker_id, &broker);
    if update_result != Ter::TES_SUCCESS {
        return update_result;
    }

    account_send(view, &broker.pseudo_account, &account, &claw_amount)
}

pub fn apply_loan_manage<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let loan_id = sttx.get_field_h256(sf("sfLoanID"));
    let tx_requests_default = sttx.is_flag(tfLoanDefault);
    let tx_requests_impair = sttx.is_flag(tfLoanImpair);
    let tx_requests_unimpair = sttx.is_flag(tfLoanUnimpair);

    let Ok(Some(loan_sle)) = view.peek(protocol::loan_keylet_from_key(loan_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let broker_id = loan_sle.get_field_h256(sf("sfLoanBrokerID"));
    let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_id = broker_sle.get_field_h256(sf("sfVaultID"));
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(vault_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();

    let next_due = loan_sle
        .is_field_present(sf("sfNextPaymentDueDate"))
        .then(|| loan_sle.get_field_u32(sf("sfNextPaymentDueDate")));
    let grace_period = if loan_sle.is_field_present(sf("sfGracePeriod")) {
        loan_sle.get_field_u32(sf("sfGracePeriod"))
    } else {
        0
    };
    let default_expiration = next_due.map(|due| due.saturating_add(grace_period));
    let preclaim = tx::run_loan_manage_preclaim(tx::LoanManagePreclaimFacts {
        loan_exists: true,
        loan_is_defaulted: loan_sle.is_flag(protocol::lsfLoanDefault),
        loan_is_impaired: loan_sle.is_flag(protocol::lsfLoanImpaired),
        tx_requests_impair,
        tx_requests_unimpair,
        tx_requests_default,
        payment_remaining_is_zero: loan_sle.get_field_u32(sf("sfPaymentRemaining")) == 0,
        default_is_too_soon: tx_requests_default && !has_expired(view, default_expiration),
        broker_exists: true,
        submitter_is_broker_owner: broker_sle.get_account_id(sf("sfOwner")) == account,
    });
    if preclaim != Ter::TES_SUCCESS {
        return preclaim;
    }

    let result = if tx_requests_default {
        let loan_scale = loan_sle.get_field_i32(sf("sfLoanScale"));
        let scale = vault_scale(&vault_sle, vault_asset);

        let total_value_outstanding = loan_sle
            .get_field_number(sf("sfTotalValueOutstanding"))
            .value();
        let management_fee_outstanding = loan_sle
            .get_field_number(sf("sfManagementFeeOutstanding"))
            .value();
        let total_default_amount = total_value_outstanding - management_fee_outstanding;

        let broker_debt_total = broker_sle.get_field_number(sf("sfDebtTotal")).value();
        let minimum_cover = tenth_bips_of_runtime_number(
            broker_debt_total,
            broker_sle.get_field_u32(sf("sfCoverRateMinimum")),
        );
        let liquidation_cover = tenth_bips_of_runtime_number(
            minimum_cover,
            broker_sle.get_field_u32(sf("sfCoverRateLiquidation")),
        );
        let liquidation_cover_capped = liquidation_cover.min(total_default_amount);
        let _upward = NumberRoundModeGuard::new(RoundingMode::Upward);
        let covered_before_cover_available = round_number_to_asset_with_scale(
            vault_asset,
            liquidation_cover_capped,
            loan_scale,
            RoundingMode::Upward,
        );
        drop(_upward);
        let cover_available = broker_sle.get_field_number(sf("sfCoverAvailable")).value();
        let default_covered = covered_before_cover_available.min(cover_available);
        let vault_default_amount = total_default_amount - default_covered;

        let vault_total_assets = vault_sle.get_field_number(sf("sfAssetsTotal")).value();
        if vault_total_assets < vault_default_amount {
            Ter::TEF_BAD_LEDGER
        } else {
            let vault_default_rounded = round_number_to_asset_with_scale(
                vault_asset,
                vault_default_amount,
                scale,
                RoundingMode::Downward,
            );
            let mut vault_total_after = vault_total_assets - vault_default_rounded;
            let vault_available_after =
                vault_sle.get_field_number(sf("sfAssetsAvailable")).value() + default_covered;
            if vault_available_after > vault_total_after && !vault_asset.integral() {
                let difference = vault_available_after - vault_total_after;
                let available_exponent = vault_available_after
                    .external_parts()
                    .map(|(_, exponent)| exponent)
                    .unwrap_or(0);
                let difference_exponent = difference
                    .external_parts()
                    .map(|(_, exponent)| exponent)
                    .unwrap_or(0);
                if available_exponent - difference_exponent > 13 {
                    vault_total_after = vault_available_after;
                }
            }
            if vault_available_after > vault_total_after {
                Ter::TEC_INTERNAL
            } else {
                let mut vault_obj = vault_sle.clone_as_object();
                vault_obj.set_field_number(
                    sf("sfAssetsTotal"),
                    with_asset_number(vault_total_after, vault_asset),
                );
                vault_obj.set_field_number(
                    sf("sfAssetsAvailable"),
                    with_asset_number(vault_available_after, vault_asset),
                );
                if loan_sle.is_flag(protocol::lsfLoanImpaired) {
                    let loss_unrealized =
                        vault_sle.get_field_number(sf("sfLossUnrealized")).value();
                    if loss_unrealized < total_default_amount {
                        return Ter::TEF_BAD_LEDGER;
                    }
                    let loss_after = adjust_imprecise_number(
                        loss_unrealized,
                        -total_default_amount,
                        vault_asset,
                        scale,
                    );
                    vault_obj.set_field_number(
                        sf("sfLossUnrealized"),
                        with_asset_number(loss_after, vault_asset),
                    );
                }
                let vault_update = persist_entry(
                    view,
                    STLedgerEntry::from_stobject(vault_obj, *vault_sle.key()),
                );
                if vault_update != Ter::TES_SUCCESS {
                    vault_update
                } else {
                    if cover_available < default_covered {
                        return Ter::TEF_BAD_LEDGER;
                    }
                    let mut broker_obj = broker_sle.clone_as_object();
                    let broker_debt_after = adjust_imprecise_number(
                        broker_debt_total,
                        -total_default_amount,
                        vault_asset,
                        scale,
                    );
                    broker_obj.set_field_number(
                        sf("sfDebtTotal"),
                        with_asset_number(broker_debt_after, vault_asset),
                    );
                    broker_obj.set_field_number(
                        sf("sfCoverAvailable"),
                        with_asset_number(cover_available - default_covered, vault_asset),
                    );
                    let broker_update = persist_entry(
                        view,
                        STLedgerEntry::from_stobject(broker_obj, *broker_sle.key()),
                    );
                    if broker_update != Ter::TES_SUCCESS {
                        broker_update
                    } else {
                        let mut loan_obj = loan_sle.clone_as_object();
                        loan_obj.set_flag(protocol::lsfLoanDefault);
                        loan_obj.set_field_number(
                            sf("sfTotalValueOutstanding"),
                            zero_asset_number(vault_asset),
                        );
                        loan_obj.set_field_u32(sf("sfPaymentRemaining"), 0);
                        loan_obj.set_field_number(
                            sf("sfPrincipalOutstanding"),
                            zero_asset_number(vault_asset),
                        );
                        loan_obj.set_field_number(
                            sf("sfManagementFeeOutstanding"),
                            zero_asset_number(vault_asset),
                        );
                        loan_obj.set_field_u32(sf("sfNextPaymentDueDate"), 0);
                        let loan_update = persist_entry(
                            view,
                            STLedgerEntry::from_stobject(loan_obj, *loan_sle.key()),
                        );
                        if loan_update != Ter::TES_SUCCESS {
                            loan_update
                        } else {
                            let Ok(default_covered_amount) = vault_asset.amount(default_covered)
                            else {
                                return Ter::TEF_BAD_LEDGER;
                            };
                            account_send(
                                view,
                                &broker_sle.get_account_id(sf("sfAccount")),
                                &vault_sle.get_account_id(sf("sfAccount")),
                                &default_covered_amount,
                            )
                        }
                    }
                }
            }
        }
    } else if tx_requests_impair {
        let scale = vault_scale(&vault_sle, vault_asset);
        let loss_unrealized = loan_sle
            .get_field_number(sf("sfTotalValueOutstanding"))
            .value()
            - loan_sle
                .get_field_number(sf("sfManagementFeeOutstanding"))
                .value();
        let current_loss = vault_sle.get_field_number(sf("sfLossUnrealized")).value();
        let updated_loss =
            adjust_imprecise_number(current_loss, loss_unrealized, vault_asset, scale);
        let unavailable_assets = vault_sle.get_field_number(sf("sfAssetsTotal")).value()
            - vault_sle.get_field_number(sf("sfAssetsAvailable")).value();
        if updated_loss > unavailable_assets {
            Ter::TEC_LIMIT_EXCEEDED
        } else {
            let mut vault_obj = vault_sle.clone_as_object();
            vault_obj.set_field_number(
                sf("sfLossUnrealized"),
                with_asset_number(updated_loss, vault_asset),
            );
            let vault_update = persist_entry(
                view,
                STLedgerEntry::from_stobject(vault_obj, *vault_sle.key()),
            );
            if vault_update != Ter::TES_SUCCESS {
                vault_update
            } else {
                let mut loan_obj = loan_sle.clone_as_object();
                loan_obj.set_flag(protocol::lsfLoanImpaired);
                let current_due = next_due.unwrap_or(0);
                let next_payment_due = if has_expired(view, Some(current_due)) {
                    current_due
                } else {
                    view.parent_close_time().as_seconds()
                };
                loan_obj.set_field_u32(sf("sfNextPaymentDueDate"), next_payment_due);
                persist_entry(
                    view,
                    STLedgerEntry::from_stobject(loan_obj, *loan_sle.key()),
                )
            }
        }
    } else if tx_requests_unimpair {
        let scale = vault_scale(&vault_sle, vault_asset);
        let loss_reversed = loan_sle
            .get_field_number(sf("sfTotalValueOutstanding"))
            .value()
            - loan_sle
                .get_field_number(sf("sfManagementFeeOutstanding"))
                .value();
        let current_loss = vault_sle.get_field_number(sf("sfLossUnrealized")).value();
        if current_loss < loss_reversed {
            Ter::TEF_BAD_LEDGER
        } else {
            let updated_loss =
                adjust_imprecise_number(current_loss, -loss_reversed, vault_asset, scale);
            let mut vault_obj = vault_sle.clone_as_object();
            vault_obj.set_field_number(
                sf("sfLossUnrealized"),
                with_asset_number(updated_loss, vault_asset),
            );
            let vault_update = persist_entry(
                view,
                STLedgerEntry::from_stobject(vault_obj, *vault_sle.key()),
            );
            if vault_update != Ter::TES_SUCCESS {
                vault_update
            } else {
                let previous_due = if loan_sle.is_field_present(sf("sfPreviousPaymentDueDate")) {
                    loan_sle.get_field_u32(sf("sfPreviousPaymentDueDate"))
                } else {
                    0
                };
                let start_date = loan_sle.get_field_u32(sf("sfStartDate"));
                let payment_interval = loan_sle.get_field_u32(sf("sfPaymentInterval"));
                let normal_payment_due_date = previous_due
                    .max(start_date)
                    .saturating_add(payment_interval);
                let next_payment_due = if has_expired(view, Some(normal_payment_due_date)) {
                    view.parent_close_time()
                        .as_seconds()
                        .saturating_add(payment_interval)
                } else {
                    normal_payment_due_date
                };
                let mut loan_obj = loan_sle.clone_as_object();
                loan_obj.clear_flag(protocol::lsfLoanImpaired);
                loan_obj.set_field_u32(sf("sfNextPaymentDueDate"), next_payment_due);
                persist_entry(
                    view,
                    STLedgerEntry::from_stobject(loan_obj, *loan_sle.key()),
                )
            }
        }
    } else {
        Ter::TES_SUCCESS
    };

    if result == Ter::TES_SUCCESS && view.rules().enabled(&feature_id("fixCleanup3_1_3")) {
        let Ok(Some(loan_entry)) = view.peek(protocol::loan_keylet_from_key(loan_id)) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Ok(Some(broker_entry)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id))
        else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Ok(Some(vault_entry)) = view.peek(protocol::vault_keylet_from_key(vault_id)) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let loan_obj = loan_entry.clone_as_object();
        let mut loan_update = STLedgerEntry::from_stobject(loan_obj, *loan_entry.key());
        associate_asset_entry(&mut loan_update, vault_asset);
        let _ = view.update(Arc::new(loan_update));
        let broker_obj = broker_entry.clone_as_object();
        let mut broker_update = STLedgerEntry::from_stobject(broker_obj, *broker_entry.key());
        associate_asset_entry(&mut broker_update, vault_asset);
        let _ = view.update(Arc::new(broker_update));
        let vault_obj = vault_entry.clone_as_object();
        let mut vault_update = STLedgerEntry::from_stobject(vault_obj, *vault_entry.key());
        associate_asset_entry(&mut vault_update, vault_asset);
        let _ = view.update(Arc::new(vault_update));
    }

    result
}

// ============================================================================
// LOAN_PAY: Pre-cached trait bridge connecting tx crate logic to ledger view
// ============================================================================

/// Pre-cached loan view satisfying LoanPayDoApplyLoan's reference contracts.
struct LpLoanView {
    broker_id: Uint256,
    scale: i32,
    impaired: bool,
    _asset: Asset,
}

impl LpLoanView {
    fn from_sle(sle: &STLedgerEntry, asset: Asset) -> Self {
        Self {
            broker_id: sle.get_field_h256(sf("sfLoanBrokerID")),
            scale: if sle.is_field_present(sf("sfLoanScale")) {
                sle.get_field_i32(sf("sfLoanScale"))
            } else {
                0
            },
            impaired: (sle.get_field_u32(sf("sfFlags")) & protocol::lsfLoanImpaired) != 0,
            _asset: asset,
        }
    }
}

impl tx::LoanPayDoApplyLoan for LpLoanView {
    type BrokerId = Uint256;
    type Asset = Asset;
    fn broker_id(&self) -> &Uint256 {
        &self.broker_id
    }
    fn scale(&self) -> i32 {
        self.scale
    }
    fn is_impaired(&self) -> bool {
        self.impaired
    }
    fn associate_asset(&mut self, _asset: &Asset) { /* tracked via self.asset */
    }
}

/// Pre-cached broker view satisfying LoanPayDoApplyBroker's reference contracts.
struct LpBrokerView {
    owner: AccountID,
    pseudo: AccountID,
    vault_id: Uint256,
    cover_available: RuntimeNumber,
    debt_total: RuntimeNumber,
    cover_rate_minimum: u32,
    _asset: Asset,
}

impl LpBrokerView {
    fn from_sle(sle: &STLedgerEntry, asset: Asset) -> Self {
        Self {
            owner: sle.get_account_id(sf("sfOwner")),
            pseudo: sle.get_account_id(sf("sfAccount")),
            vault_id: sle.get_field_h256(sf("sfVaultID")),
            cover_available: if sle.is_field_present(sf("sfCoverAvailable")) {
                sle.get_field_number(sf("sfCoverAvailable")).value()
            } else {
                RuntimeNumber::zero()
            },
            debt_total: if sle.is_field_present(sf("sfDebtTotal")) {
                sle.get_field_number(sf("sfDebtTotal")).value()
            } else {
                RuntimeNumber::zero()
            },
            cover_rate_minimum: if sle.is_field_present(sf("sfCoverRateMinimum")) {
                sle.get_field_u32(sf("sfCoverRateMinimum"))
            } else {
                0
            },
            _asset: asset,
        }
    }
}

impl tx::LoanPayDoApplyBroker for LpBrokerView {
    type AccountId = AccountID;
    type VaultId = Uint256;
    type Amount = RuntimeNumber;
    type Asset = Asset;
    fn owner(&self) -> &AccountID {
        &self.owner
    }
    fn pseudo_account(&self) -> &AccountID {
        &self.pseudo
    }
    fn vault_id(&self) -> &Uint256 {
        &self.vault_id
    }
    fn cover_available(&self) -> &RuntimeNumber {
        &self.cover_available
    }
    fn debt_total(&self) -> &RuntimeNumber {
        &self.debt_total
    }
    fn cover_rate_minimum(&self) -> u32 {
        self.cover_rate_minimum
    }
    fn add_cover_available(&mut self, amount: RuntimeNumber) {
        self.cover_available += amount;
    }
    fn adjust_debt_total(&mut self, delta: RuntimeNumber) {
        let new_val = self.debt_total + delta;
        self.debt_total = if new_val < RuntimeNumber::zero() {
            RuntimeNumber::zero()
        } else {
            new_val
        };
    }
    fn associate_asset(&mut self, _asset: &Asset) {}
}

/// Pre-cached vault view satisfying LoanPayDoApplyVault's reference contracts.
struct LpVaultView {
    pseudo: AccountID,
    asset: Asset,
    assets_available: RuntimeNumber,
    assets_total: RuntimeNumber,
}

impl LpVaultView {
    fn from_sle(sle: &STLedgerEntry) -> Self {
        let asset = sle.get_field_issue(sf("sfAsset")).asset();
        Self {
            pseudo: sle.get_account_id(sf("sfAccount")),
            asset,
            assets_available: if sle.is_field_present(sf("sfAssetsAvailable")) {
                sle.get_field_number(sf("sfAssetsAvailable")).value()
            } else {
                RuntimeNumber::zero()
            },
            assets_total: if sle.is_field_present(sf("sfAssetsTotal")) {
                sle.get_field_number(sf("sfAssetsTotal")).value()
            } else {
                RuntimeNumber::zero()
            },
        }
    }
}

impl tx::LoanPayDoApplyVault for LpVaultView {
    type AccountId = AccountID;
    type Asset = Asset;
    type Amount = RuntimeNumber;
    fn pseudo_account(&self) -> &AccountID {
        &self.pseudo
    }
    fn asset(&self) -> &Asset {
        &self.asset
    }
    fn assets_available(&self) -> &RuntimeNumber {
        &self.assets_available
    }
    fn assets_total(&self) -> &RuntimeNumber {
        &self.assets_total
    }
    fn add_assets_available(&mut self, amount: RuntimeNumber) {
        self.assets_available += amount;
    }
    fn add_assets_total(&mut self, amount: RuntimeNumber) {
        self.assets_total += amount;
    }
    fn assets_available_exceeds_total(&self) -> bool {
        self.assets_available > self.assets_total
    }
    fn associate_asset(&mut self, _asset: &Asset) {}
}

pub fn apply_loan_pay<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view
        .rules()
        .enabled(&protocol::feature_id("LendingProtocol"))
    {
        return Ter::TEM_DISABLED;
    }

    let loan_id = sttx.get_field_h256(sf("sfLoanID"));
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let borrower = sttx.get_account_id(sf("sfAccount"));
    let flags = sttx.get_field_u32(sf("sfFlags"));

    let payment_type = tx::run_loan_pay_payment_type(
        flags & protocol::LOAN_LATE_PAYMENT_FLAG != 0,
        flags & protocol::LOAN_FULL_PAYMENT_FLAG != 0,
        flags & protocol::LOAN_OVERPAYMENT_FLAG != 0,
    );

    // Load and cache loan
    let loan_keylet = protocol::loan_keylet_from_key(loan_id);
    let loan_sle = match view.peek(loan_keylet) {
        Ok(Some(sle)) => sle,
        _ => return Ter::TEF_BAD_LEDGER,
    };

    // Load and cache broker
    let broker_id = loan_sle.get_field_h256(sf("sfLoanBrokerID"));
    let broker_keylet = protocol::loan_broker_keylet_from_key(broker_id);
    let broker_sle = match view.peek(broker_keylet) {
        Ok(Some(sle)) => sle,
        _ => return Ter::TEF_BAD_LEDGER,
    };

    // Load and cache vault
    let vault_id = broker_sle.get_field_h256(sf("sfVaultID"));
    let vault_keylet = protocol::vault_keylet_from_key(vault_id);
    let vault_sle = match view.peek(vault_keylet) {
        Ok(Some(sle)) => sle,
        _ => return Ter::TEF_BAD_LEDGER,
    };

    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    let loan_view = LpLoanView::from_sle(&loan_sle, vault_asset);
    let mut broker_view = LpBrokerView::from_sle(&broker_sle, vault_asset);
    let mut vault_view = LpVaultView::from_sle(&vault_sle);
    let loan_scale = loan_view.scale;
    let v_scale = vault_scale(&vault_sle, vault_asset);

    let payment_amount = amount_number(&amount);
    if payment_amount <= RuntimeNumber::zero() {
        return Ter::TEM_BAD_AMOUNT;
    }

    // Unimpair if needed (reference LoanManage::unimpairLoan before payment)
    if loan_view.impaired {
        let loan_obj = loan_sle.clone_as_object();
        let mut lu = STLedgerEntry::from_stobject(loan_obj, *loan_sle.key());
        let cur_flags = lu.get_field_u32(sf("sfFlags"));
        lu.set_field_u32(sf("sfFlags"), cur_flags & !protocol::lsfLoanImpaired);
        let _ = view.update(Arc::new(lu));
    }

    // Read loan payment state
    let periodic_payment = if loan_sle.is_field_present(sf("sfPeriodicPayment")) {
        loan_sle.get_field_number(sf("sfPeriodicPayment")).value()
    } else {
        RuntimeNumber::zero()
    };
    let service_fee = if loan_sle.is_field_present(sf("sfLoanServiceFee")) {
        amount_number(&loan_sle.get_field_amount(sf("sfLoanServiceFee")))
    } else {
        RuntimeNumber::zero()
    };
    let close_payment_fee = if loan_sle.is_field_present(sf("sfClosePaymentFee")) {
        amount_number(&loan_sle.get_field_amount(sf("sfClosePaymentFee")))
    } else {
        RuntimeNumber::zero()
    };
    let late_payment_fee = if loan_sle.is_field_present(sf("sfLatePaymentFee")) {
        amount_number(&loan_sle.get_field_amount(sf("sfLatePaymentFee")))
    } else {
        RuntimeNumber::zero()
    };
    let management_fee_outstanding = if loan_sle.is_field_present(sf("sfManagementFeeOutstanding"))
    {
        loan_sle
            .get_field_number(sf("sfManagementFeeOutstanding"))
            .value()
    } else {
        RuntimeNumber::zero()
    };
    let principal_outstanding = if loan_sle.is_field_present(sf("sfPrincipalOutstanding")) {
        loan_sle
            .get_field_number(sf("sfPrincipalOutstanding"))
            .value()
    } else {
        RuntimeNumber::zero()
    };
    let total_value_outstanding = if loan_sle.is_field_present(sf("sfTotalValueOutstanding")) {
        loan_sle
            .get_field_number(sf("sfTotalValueOutstanding"))
            .value()
    } else {
        RuntimeNumber::zero()
    };
    let payments_remaining = if loan_sle.is_field_present(sf("sfPaymentRemaining")) {
        loan_sle.get_field_u32(sf("sfPaymentRemaining"))
    } else {
        0
    };
    let interest_rate = if loan_sle.is_field_present(sf("sfInterestRate")) {
        TenthBips32::new(loan_sle.get_field_u32(sf("sfInterestRate")))
    } else {
        TenthBips32::new(0)
    };
    let late_interest_rate = if loan_sle.is_field_present(sf("sfLateInterestRate")) {
        TenthBips32::new(loan_sle.get_field_u32(sf("sfLateInterestRate")))
    } else {
        TenthBips32::new(0)
    };
    let close_interest_rate = if loan_sle.is_field_present(sf("sfCloseInterestRate")) {
        TenthBips32::new(loan_sle.get_field_u32(sf("sfCloseInterestRate")))
    } else {
        TenthBips32::new(0)
    };
    let overpayment_interest_rate = if loan_sle.is_field_present(sf("sfOverpaymentInterestRate")) {
        TenthBips32::new(loan_sle.get_field_u32(sf("sfOverpaymentInterestRate")))
    } else {
        TenthBips32::new(0)
    };
    let overpayment_fee_rate = if loan_sle.is_field_present(sf("sfOverpaymentFee")) {
        TenthBips32::new(loan_sle.get_field_u32(sf("sfOverpaymentFee")))
    } else {
        TenthBips32::new(0)
    };
    let payment_interval = if loan_sle.is_field_present(sf("sfPaymentInterval")) {
        loan_sle.get_field_u32(sf("sfPaymentInterval"))
    } else {
        0
    };
    let previous_payment_date = if loan_sle.is_field_present(sf("sfPreviousPaymentDueDate")) {
        loan_sle.get_field_u32(sf("sfPreviousPaymentDueDate"))
    } else {
        0
    };
    let start_date = if loan_sle.is_field_present(sf("sfStartDate")) {
        loan_sle.get_field_u32(sf("sfStartDate"))
    } else {
        0
    };
    let management_fee_rate = if broker_sle.is_field_present(sf("sfManagementFeeRate")) {
        TenthBips16::new(broker_sle.get_field_u16(sf("sfManagementFeeRate")))
    } else {
        TenthBips16::new(0)
    };

    // Compute payment parts (reference loanMakePayment logic)
    let rounded_periodic_payment = round_number_to_asset_with_scale(
        vault_asset,
        periodic_payment,
        loan_scale,
        RoundingMode::Upward,
    );
    let regular_payment = rounded_periodic_payment + service_fee;
    let effective_overpayment_amount = effective_loan_pay_amount(
        payment_type,
        view.rules().enabled(&feature_id("fixSecurity3_1_3")),
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
        vault_asset,
        payment_amount,
        loan_scale,
    );
    let (
        principal_paid,
        interest_paid,
        management_fee_paid,
        fee_paid,
        value_change,
        payment_remaining_decrement,
        periodic_payment_override,
    ) = match payment_type {
        tx::LoanPayPaymentType::Regular => {
            if regular_payment <= RuntimeNumber::zero() {
                let p = payment_amount.min(principal_outstanding);
                (
                    p,
                    RuntimeNumber::zero(),
                    RuntimeNumber::zero(),
                    RuntimeNumber::zero(),
                    RuntimeNumber::zero(),
                    1,
                    None,
                )
            } else if payment_amount < regular_payment {
                return Ter::TEC_INSUFFICIENT_PAYMENT;
            } else {
                let parts = compute_loan_pay_scheduled_payment_loop(
                    &view.rules(),
                    vault_asset,
                    loan_scale,
                    payment_amount,
                    total_value_outstanding,
                    principal_outstanding,
                    management_fee_outstanding,
                    periodic_payment,
                    interest_rate,
                    payment_interval,
                    payments_remaining,
                    management_fee_rate,
                    service_fee,
                );
                (parts.0, parts.1, parts.2, parts.3, parts.4, parts.5, None)
            }
        }
        tx::LoanPayPaymentType::Late => {
            let next_payment_due_date = loan_sle
                .is_field_present(sf("sfNextPaymentDueDate"))
                .then(|| loan_sle.get_field_u32(sf("sfNextPaymentDueDate")));
            if !has_expired(view, next_payment_due_date) {
                return Ter::TEC_TOO_SOON;
            }
            let components = compute_loan_pay_periodic_components(
                &view.rules(),
                vault_asset,
                loan_scale,
                total_value_outstanding,
                principal_outstanding,
                management_fee_outstanding,
                periodic_payment,
                interest_rate,
                payment_interval,
                payments_remaining,
                management_fee_rate,
            );
            let late_interest = round_number_to_asset_with_scale(
                vault_asset,
                loan_late_payment_interest(
                    principal_outstanding,
                    late_interest_rate,
                    view.parent_close_time().as_seconds(),
                    next_payment_due_date.unwrap_or(0),
                ),
                loan_scale,
                RoundingMode::ToNearest,
            );
            let (late_net_interest, late_management_fee) = compute_interest_and_fee_parts(
                vault_asset,
                late_interest,
                management_fee_rate,
                loan_scale,
            );
            let tracked_due = components.principal_paid
                + components.interest_paid
                + components.management_fee_paid;
            let fee = components.management_fee_paid
                + service_fee
                + late_payment_fee
                + late_management_fee;
            let total_due = tracked_due + service_fee + late_payment_fee + late_interest;
            if payment_amount < total_due {
                return Ter::TEC_INSUFFICIENT_PAYMENT;
            }
            (
                components.principal_paid,
                components.interest_paid + late_net_interest,
                components.management_fee_paid,
                fee,
                late_net_interest,
                1,
                None,
            )
        }
        tx::LoanPayPaymentType::Full => {
            if payments_remaining <= 1 {
                return Ter::TEC_KILLED;
            }
            let outstanding_interest =
                (total_value_outstanding - principal_outstanding - management_fee_outstanding)
                    .max(RuntimeNumber::zero());
            let periodic_rate = tx::loan_set_periodic_rate(interest_rate, payment_interval);
            let (full_interest_paid, full_fee_paid, value_change) = compute_full_payment_parts(
                &view.rules(),
                vault_asset,
                loan_scale,
                view.parent_close_time().as_seconds(),
                outstanding_interest,
                periodic_payment,
                periodic_rate,
                payments_remaining,
                previous_payment_date,
                start_date,
                payment_interval,
                close_interest_rate,
                close_payment_fee,
                management_fee_rate,
            );
            let needed =
                total_value_outstanding + value_change + full_fee_paid - management_fee_outstanding;
            if payment_amount < needed {
                return Ter::TEC_INSUFFICIENT_PAYMENT;
            }
            (
                principal_outstanding,
                full_interest_paid,
                management_fee_outstanding,
                full_fee_paid,
                value_change,
                payments_remaining,
                None,
            )
        }
        tx::LoanPayPaymentType::Overpayment => {
            if regular_payment <= RuntimeNumber::zero() {
                let p = effective_overpayment_amount.min(principal_outstanding);
                (
                    p,
                    RuntimeNumber::zero(),
                    RuntimeNumber::zero(),
                    RuntimeNumber::zero(),
                    RuntimeNumber::zero(),
                    1,
                    None,
                )
            } else {
                let (
                    mut principal_paid,
                    mut interest_paid,
                    mut management_fee_paid,
                    mut fee_paid,
                    mut value_change,
                    periods_paid,
                ) = compute_loan_pay_scheduled_payment_loop(
                    &view.rules(),
                    vault_asset,
                    loan_scale,
                    effective_overpayment_amount,
                    total_value_outstanding,
                    principal_outstanding,
                    management_fee_outstanding,
                    periodic_payment,
                    interest_rate,
                    payment_interval,
                    payments_remaining,
                    management_fee_rate,
                    service_fee,
                );
                let total_scheduled_paid = principal_paid + interest_paid + fee_paid;
                let mut periodic_payment_override = None;
                if loan_sle.is_flag(protocol::lsfLoanOverpayment)
                    && periods_paid < payments_remaining
                    && total_scheduled_paid < effective_overpayment_amount
                    && periods_paid < tx::LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION
                {
                    let current_total = (total_value_outstanding
                        - principal_paid
                        - interest_paid
                        - management_fee_paid)
                        .max(RuntimeNumber::zero());
                    let current_principal =
                        (principal_outstanding - principal_paid).max(RuntimeNumber::zero());
                    let current_management_fee = (management_fee_outstanding - management_fee_paid)
                        .max(RuntimeNumber::zero());
                    let remaining_payments = payments_remaining.saturating_sub(periods_paid);
                    let overpayment_raw =
                        (effective_overpayment_amount - total_scheduled_paid).min(current_total);
                    let overpayment = if view.rules().enabled(&feature_id("fixCleanup3_2_0")) {
                        round_number_to_asset_with_scale(
                            vault_asset,
                            overpayment_raw,
                            loan_scale,
                            RoundingMode::Downward,
                        )
                    } else {
                        overpayment_raw
                    };
                    let periodic_rate = tx::loan_set_periodic_rate(interest_rate, payment_interval);
                    if let Some(extra) = compute_overpayment_reamortization(
                        &view.rules(),
                        vault_asset,
                        loan_scale,
                        overpayment,
                        current_total,
                        current_principal,
                        current_management_fee,
                        periodic_payment,
                        interest_rate,
                        payment_interval,
                        periodic_rate,
                        remaining_payments,
                        management_fee_rate,
                        overpayment_interest_rate,
                        overpayment_fee_rate,
                    ) {
                        principal_paid += extra.principal_paid;
                        interest_paid += extra.interest_paid;
                        management_fee_paid += extra.management_fee_paid;
                        fee_paid += extra.fee_paid;
                        value_change += extra.value_change;
                        periodic_payment_override = Some(extra.periodic_payment);
                    }
                }
                (
                    principal_paid,
                    interest_paid,
                    management_fee_paid,
                    fee_paid,
                    value_change,
                    periods_paid.max(1),
                    periodic_payment_override,
                )
            }
        }
    };

    let total_to_vault = principal_paid + interest_paid;
    let total_to_broker = fee_paid;
    let total_paid = total_to_vault + total_to_broker;
    if total_paid <= RuntimeNumber::zero() {
        return Ter::TEC_INSUFFICIENT_PAYMENT;
    }

    // Broker fee routing (reference sendBrokerFeeToOwner)
    let required_cover = loan_pay_fee_route_minimum_cover(
        vault_asset,
        broker_view.debt_total,
        broker_view.cover_rate_minimum,
        loan_scale,
        v_scale,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    );
    let send_fee_to_owner = broker_view.cover_available >= required_cover
        && !asset_deep_frozen(view, &broker_view.owner, vault_asset)
        && !asset_requires_strong_auth(view, &broker_view.owner, vault_asset);
    let broker_payee = if send_fee_to_owner {
        broker_view.owner
    } else {
        broker_view.pseudo
    };
    if !send_fee_to_owner {
        let ter = check_asset_deep_frozen(view, &broker_payee, vault_asset);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    // Update cached views and validate precision before moving funds. C++ applies these
    // guards before accountSendMulti so failed rounding does not transfer balances.
    let assets_available_before = vault_view.assets_available;
    let assets_total_before = vault_view.assets_total;
    let rounded_to_vault = round_number_to_asset_with_scale(
        vault_asset,
        total_to_vault,
        v_scale,
        RoundingMode::Downward,
    );
    vault_view.assets_available += rounded_to_vault;
    if value_change != RuntimeNumber::zero() {
        vault_view.assets_total += value_change;
    }
    let assets_available_after = vault_view.assets_available;
    let assets_total_after = vault_view.assets_total;
    if assets_available_after > assets_total_after {
        return Ter::TEC_INTERNAL;
    }
    if assets_available_after == assets_available_before {
        return Ter::TEC_PRECISION_LOSS;
    }
    if value_change != RuntimeNumber::zero() && assets_total_after == assets_total_before {
        return Ter::TEC_PRECISION_LOSS;
    }
    if value_change == RuntimeNumber::zero() && assets_total_after != assets_total_before {
        return Ter::TEC_INTERNAL;
    }
    if rounded_to_vault + total_to_broker > payment_amount {
        return Ter::TEC_INSUFFICIENT_PAYMENT;
    }

    let debt_reduction = total_to_vault - value_change;
    let new_debt = broker_view.debt_total + (-debt_reduction);
    broker_view.debt_total = if new_debt < RuntimeNumber::zero() {
        RuntimeNumber::zero()
    } else {
        new_debt
    };
    if !send_fee_to_owner && total_to_broker > RuntimeNumber::zero() {
        broker_view.cover_available += total_to_broker;
    }

    // Fund transfers
    if rounded_to_vault > RuntimeNumber::zero() {
        if let Some(xfer) = runtime_to_amount(vault_asset, rounded_to_vault, RoundingMode::Downward)
        {
            let ter = account_send(view, &borrower, &vault_view.pseudo, &xfer);
            if !protocol::is_tes_success(ter) {
                return ter;
            }
        }
    }
    if total_to_broker > RuntimeNumber::zero() {
        if let Some(xfer) = runtime_to_amount(vault_asset, total_to_broker, RoundingMode::Downward)
        {
            let ter = account_send(view, &borrower, &broker_payee, &xfer);
            if !protocol::is_tes_success(ter) {
                return ter;
            }
        }
    }

    // Persist loan update
    let loan_sle_now = match view.peek(loan_keylet) {
        Ok(Some(s)) => s,
        _ => return Ter::TEF_BAD_LEDGER,
    };
    let lo = loan_sle_now.clone_as_object();
    let mut lu = STLedgerEntry::from_stobject(lo, *loan_sle_now.key());
    lu.set_field_number(
        sf("sfPrincipalOutstanding"),
        with_asset_number(principal_outstanding - principal_paid, vault_asset),
    );
    lu.set_field_number(
        sf("sfManagementFeeOutstanding"),
        with_asset_number(
            (management_fee_outstanding - management_fee_paid).max(RuntimeNumber::zero()),
            vault_asset,
        ),
    );
    let tracked_value_paid = principal_paid + interest_paid + management_fee_paid - value_change;
    let new_total = total_value_outstanding - tracked_value_paid;
    lu.set_field_number(
        sf("sfTotalValueOutstanding"),
        with_asset_number(
            if new_total < RuntimeNumber::zero() {
                RuntimeNumber::zero()
            } else {
                new_total
            },
            vault_asset,
        ),
    );
    if let Some(periodic_payment) = periodic_payment_override {
        lu.set_field_number(
            sf("sfPeriodicPayment"),
            with_asset_number(periodic_payment, vault_asset),
        );
    }
    if lu.is_field_present(sf("sfPaymentRemaining")) {
        let r = lu.get_field_u32(sf("sfPaymentRemaining"));
        let dec = match payment_type {
            tx::LoanPayPaymentType::Full => r,
            _ => payment_remaining_decrement
                .max(1)
                .min(r)
                .min(tx::LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION),
        };
        lu.set_field_u32(sf("sfPaymentRemaining"), r.saturating_sub(dec));
    }
    associate_asset_entry(&mut lu, vault_asset);
    let _ = view.update(Arc::new(lu));

    // Persist vault update
    let vs = match view.peek(vault_keylet) {
        Ok(Some(s)) => s,
        _ => return Ter::TEF_BAD_LEDGER,
    };
    let vo = vs.clone_as_object();
    let mut vu = STLedgerEntry::from_stobject(vo, *vs.key());
    vu.set_field_number(
        sf("sfAssetsAvailable"),
        with_asset_number(vault_view.assets_available, vault_asset),
    );
    vu.set_field_number(
        sf("sfAssetsTotal"),
        with_asset_number(vault_view.assets_total, vault_asset),
    );
    associate_asset_entry(&mut vu, vault_asset);
    let _ = view.update(Arc::new(vu));

    // Persist broker update
    let bs = match view.peek(broker_keylet) {
        Ok(Some(s)) => s,
        _ => return Ter::TEF_BAD_LEDGER,
    };
    let bo = bs.clone_as_object();
    let mut bu = STLedgerEntry::from_stobject(bo, *bs.key());
    bu.set_field_number(
        sf("sfDebtTotal"),
        with_asset_number(broker_view.debt_total, vault_asset),
    );
    bu.set_field_number(
        sf("sfCoverAvailable"),
        with_asset_number(broker_view.cover_available, vault_asset),
    );
    associate_asset_entry(&mut bu, vault_asset);
    let _ = view.update(Arc::new(bu));

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod loan_pay_effective_amount_tests {
    use super::*;
    use protocol::{Issue, currency_from_string};

    fn account(byte: u8) -> AccountID {
        AccountID::from_array([byte; 20])
    }

    fn usd_asset() -> Asset {
        Asset::Issue(Issue::new(currency_from_string("USD"), account(0x55)))
    }

    fn vault_with_assets_total(asset: Asset, assets_total: RuntimeNumber) -> STLedgerEntry {
        let keylet = protocol::vault_keylet(Uint160::from_void(account(0x54).data()), 1);
        let mut vault = STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, keylet.key);
        vault.set_field_number(sf("sfAssetsTotal"), with_asset_number(assets_total, asset));
        vault.set_field_u8(sf("sfScale"), 2);
        vault
    }

    #[test]
    fn loan_pay_overpayment_amount_truncates_at_loan_scale_after_fix_security_3_1_3() {
        let amount = RuntimeNumber::try_from_external_parts(123_456_789, -8, get_mantissa_scale())
            .expect("valid runtime number");

        let rounded = effective_loan_pay_amount(
            tx::LoanPayPaymentType::Overpayment,
            true,
            false,
            usd_asset(),
            amount,
            -6,
        );

        assert_eq!(
            rounded,
            RuntimeNumber::try_from_external_parts(1_234_567, -6, get_mantissa_scale())
                .expect("expected rounded number")
        );
    }

    #[test]
    fn loan_pay_overpayment_amount_keeps_legacy_precision_without_fix_security_3_1_3() {
        let amount = RuntimeNumber::try_from_external_parts(123_456_789, -8, get_mantissa_scale())
            .expect("valid runtime number");

        let legacy = effective_loan_pay_amount(
            tx::LoanPayPaymentType::Overpayment,
            false,
            false,
            usd_asset(),
            amount,
            -6,
        );

        assert_eq!(legacy, amount);
    }

    #[test]
    fn loan_pay_regular_amount_keeps_precision_after_fix_security_3_1_3() {
        let amount = RuntimeNumber::try_from_external_parts(123_456_789, -8, get_mantissa_scale())
            .expect("valid runtime number");

        let regular = effective_loan_pay_amount(
            tx::LoanPayPaymentType::Regular,
            true,
            true,
            usd_asset(),
            amount,
            -6,
        );

        assert_eq!(regular, amount);
    }

    #[test]
    fn loan_pay_overpayment_amount_truncates_at_loan_scale_after_fix_cleanup_3_2_0() {
        let amount = RuntimeNumber::try_from_external_parts(123_456_789, -8, get_mantissa_scale())
            .expect("valid runtime number");

        let rounded = effective_loan_pay_amount(
            tx::LoanPayPaymentType::Overpayment,
            false,
            true,
            usd_asset(),
            amount,
            -6,
        );

        assert_eq!(
            rounded,
            RuntimeNumber::try_from_external_parts(1_234_567, -6, get_mantissa_scale())
                .expect("expected rounded number")
        );
    }

    #[test]
    fn loan_pay_fee_route_cover_uses_loan_scale_before_cleanup_3_2_0() {
        let debt_total = RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
            .expect("valid debt total");

        let required =
            loan_pay_fee_route_minimum_cover(usd_asset(), debt_total, 100_000, -4, -2, false);

        assert_eq!(
            required,
            RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
                .expect("legacy loan-scale threshold")
        );
    }

    #[test]
    fn loan_pay_fee_route_cover_uses_vault_scale_after_cleanup_3_2_0() {
        let debt_total = RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
            .expect("valid debt total");

        let required =
            loan_pay_fee_route_minimum_cover(usd_asset(), debt_total, 100_000, -4, -2, true);

        assert_eq!(
            required,
            RuntimeNumber::try_from_external_parts(124, -2, get_mantissa_scale())
                .expect("post-fix vault-scale threshold")
        );
    }

    #[test]
    fn loan_set_debt_total_update_uses_loan_scale_before_cleanup_3_2_0() {
        let asset = usd_asset();
        let vault_total = RuntimeNumber::try_from_external_parts(100, -2, get_mantissa_scale())
            .expect("valid vault total");
        let vault = vault_with_assets_total(asset, vault_total);
        let adjustment = RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
            .expect("valid adjustment");

        let debt_total =
            loan_set_debt_total_update(asset, RuntimeNumber::zero(), adjustment, &vault, -4, false);

        assert_eq!(
            debt_total,
            RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
                .expect("legacy loan-scale debt total")
        );
    }

    #[test]
    fn loan_set_debt_total_update_uses_vault_scale_after_cleanup_3_2_0() {
        let asset = usd_asset();
        let vault_total = RuntimeNumber::try_from_external_parts(100, -2, get_mantissa_scale())
            .expect("valid vault total");
        let vault = vault_with_assets_total(asset, vault_total);
        let adjustment = RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
            .expect("valid adjustment");

        let debt_total =
            loan_set_debt_total_update(asset, RuntimeNumber::zero(), adjustment, &vault, -4, true);

        assert_eq!(
            debt_total,
            RuntimeNumber::try_from_external_parts(123, -2, get_mantissa_scale())
                .expect("post-fix vault-scale debt total")
        );
    }
}
