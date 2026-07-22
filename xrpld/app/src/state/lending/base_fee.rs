use basics::number::{NumberParts as RuntimeNumber, RoundingMode};
use ledger::{ReadView, has_expired};
use protocol::{STTx, lending::LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION};

use super::{common::*, helpers::*};

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
    let fix_cleanup_3_1_3 = view.rules().enabled(&protocol::fix_cleanup_3_1_3());
    if fix_cleanup_3_1_3
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
    let increments = if fix_cleanup_3_1_3 {
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
