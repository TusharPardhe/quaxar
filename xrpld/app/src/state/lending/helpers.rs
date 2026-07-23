use std::sync::Arc;

use basics::number::{NumberParts as RuntimeNumber, RoundingMode, get_mantissa_scale};
use ledger::{RelativeDistanceAmount, views::apply_view::ApplyView};
use protocol::{
    AccountID, Asset, STAmount, STLedgerEntry, STNumber, TenthBips16, TenthBips32, Ter,
    account_keylet, feature_id, to_amount_from_number,
};

use super::common::*;

pub(super) fn persist_entry<V: ApplyView>(view: &mut V, entry: STLedgerEntry) -> Ter {
    view.update(Arc::new(entry))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

pub(super) fn account_balance_drops(sle: &STLedgerEntry) -> i64 {
    sle.get_field_amount(sf("sfBalance")).xrp().drops()
}

pub(super) fn associate_asset_entry(entry: &mut STLedgerEntry, asset: Asset) {
    protocol::associate_asset(entry, asset);
}

pub(super) fn round_number_to_asset(asset: Asset, value: RuntimeNumber) -> RuntimeNumber {
    let mut number = STNumber::from(value);
    number.associate_asset(asset);
    number.value()
}

pub(super) fn vault_scale(vault_sle: &STLedgerEntry, asset: Asset) -> i32 {
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

pub(super) fn round_runtime_to_scale(
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

pub(super) fn round_number_to_asset_with_scale(
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

pub(super) fn adjust_imprecise_number(
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

pub(super) fn effective_loan_pay_amount(
    payment_type: tx::LoanPayPaymentType,
    fix_cleanup_3_1_3: bool,
    fix_cleanup_3_2_0: bool,
    asset: Asset,
    payment_amount: RuntimeNumber,
    loan_scale: i32,
) -> RuntimeNumber {
    if payment_type == tx::LoanPayPaymentType::Overpayment
        && (fix_cleanup_3_1_3 || fix_cleanup_3_2_0)
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

pub(super) fn amount_number(amount: &STAmount) -> RuntimeNumber {
    amount.as_number()
}

pub(super) fn asset_scale_from_value(asset: Asset, value: RuntimeNumber) -> i32 {
    if asset.integral() {
        return 0;
    }
    asset
        .amount(value)
        .map(|amount| amount.exponent())
        .unwrap_or(0)
}

pub(super) fn minimum_broker_cover(
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

pub(super) fn loan_pay_fee_route_minimum_cover(
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

pub(super) fn is_pseudo_account<V: ApplyView>(view: &mut V, account: &AccountID) -> bool {
    let Ok(Some(account_sle)) = view.peek(account_keylet(to_160(account))) else {
        return false;
    };
    account_sle.is_field_present(sf("sfVaultID"))
        || account_sle.is_field_present(sf("sfLoanBrokerID"))
}

pub(super) fn runtime_to_amount(
    asset: Asset,
    value: RuntimeNumber,
    rounding: RoundingMode,
) -> Option<STAmount> {
    to_amount_from_number(asset, value, rounding).ok()
}

pub(super) fn rounded_cover_deposit_amount(
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

pub(super) fn cover_amount_is_zero_at_cover_scale(
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

#[derive(Debug, Clone, Copy)]
pub(super) struct LoanPayComponentParts {
    pub(super) principal_paid: RuntimeNumber,
    pub(super) interest_paid: RuntimeNumber,
    pub(super) management_fee_paid: RuntimeNumber,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compute_loan_pay_periodic_components(
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
pub(super) fn compute_loan_pay_scheduled_payment_loop(
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

pub(super) fn zero_asset_number(asset: Asset) -> STNumber {
    with_asset_number(RuntimeNumber::zero(), asset)
}

pub(super) fn tenth_bips_of_runtime_number(value: RuntimeNumber, rate: u32) -> RuntimeNumber {
    value * RuntimeNumber::from_i64(i64::from(rate)) / RuntimeNumber::from_i64(100_000)
}

pub(super) fn compute_interest_and_fee_parts(
    asset: Asset,
    interest: RuntimeNumber,
    management_fee_rate: TenthBips16,
    loan_scale: i32,
) -> (RuntimeNumber, RuntimeNumber) {
    let fee = tx::compute_management_fee(asset, interest, management_fee_rate, loan_scale);
    (interest - fee, fee)
}

pub(super) fn loan_accrued_interest(
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

pub(super) fn loan_late_payment_interest(
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
pub(super) fn compute_full_payment_parts(
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

pub(super) struct OverpaymentReamortization {
    pub(super) principal_paid: RuntimeNumber,
    pub(super) interest_paid: RuntimeNumber,
    pub(super) management_fee_paid: RuntimeNumber,
    pub(super) fee_paid: RuntimeNumber,
    pub(super) value_change: RuntimeNumber,
    pub(super) periodic_payment: RuntimeNumber,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compute_overpayment_reamortization(
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

    let fix_cleanup_3_2_0 = rules.enabled(&feature_id("fixCleanup3_2_0"));
    let new_theoretical_state = if fix_cleanup_3_2_0 {
        let value = new_theoretical_state.value_outstanding;
        let principal = old_principal - tracked_principal_delta;
        let management_fee =
            tenth_bips_of_runtime_number(value - principal, u32::from(management_fee_rate.value()));
        tx::construct_loan_set_state(value, principal, management_fee)
    } else {
        new_theoretical_state
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
