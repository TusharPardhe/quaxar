use basics::number::{NumberParts as RuntimeNumber, RoundingMode};
use ledger::{ReadView, has_expired};
use protocol::{STTx, Ter, lending::LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION};

use super::{common::*, helpers::*};

fn base_fee_lookup<T>(lookup: Result<Option<T>, ledger::ViewError>) -> Result<Option<T>, Ter> {
    lookup.map_err(|_| Ter::TEF_BAD_LEDGER)
}

pub fn calculate_loan_pay_base_fee<V: ReadView>(
    view: &V,
    sttx: &STTx,
    normal_cost: u64,
) -> Result<u64, Ter> {
    if sttx.is_flag(protocol::tfLoanFullPayment) || sttx.is_flag(protocol::tfLoanLatePayment) {
        return Ok(normal_cost);
    }

    let loan_id = sttx.get_field_h256(sf("sfLoanID"));
    let loan_sle = match base_fee_lookup(view.read(protocol::loan_keylet_from_key(loan_id))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ok(normal_cost),
        Err(_) => return Err(Ter::TEF_BAD_LEDGER),
    };

    let payments_remaining = loan_sle.get_field_u32(sf("sfPaymentRemaining"));
    if payments_remaining <= tx::LOAN_PAYMENTS_PER_FEE_INCREMENT {
        return Ok(normal_cost);
    }
    if has_expired(
        view,
        loan_sle
            .is_field_present(sf("sfNextPaymentDueDate"))
            .then(|| loan_sle.get_field_u32(sf("sfNextPaymentDueDate"))),
    ) {
        return Ok(normal_cost);
    }

    let broker_sle = match base_fee_lookup(view.read(protocol::loan_broker_keylet_from_key(
        loan_sle.get_field_h256(sf("sfLoanBrokerID")),
    ))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ok(normal_cost),
        Err(_) => return Err(Ter::TEF_BAD_LEDGER),
    };
    let vault_sle = match base_fee_lookup(view.read(protocol::vault_keylet_from_key(
        broker_sle.get_field_h256(sf("sfVaultID")),
    ))) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ok(normal_cost),
        Err(_) => return Err(Ter::TEF_BAD_LEDGER),
    };

    let amount = sttx.get_field_amount(sf("sfAmount"));
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    if amount.asset() != vault_asset {
        return Ok(normal_cost);
    }

    let periodic_payment = loan_sle.get_field_number(sf("sfPeriodicPayment")).value();
    let service_fee = if loan_sle.is_field_present(sf("sfLoanServiceFee")) {
        loan_sle.get_field_number(sf("sfLoanServiceFee")).value()
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
        return Ok(normal_cost);
    }

    let payment_amount = amount_number(&amount);
    let fix_cleanup_3_1_3 = view.rules().enabled(&protocol::fix_cleanup_3_1_3());
    if fix_cleanup_3_1_3
        && payment_amount
            >= regular_payment
                * RuntimeNumber::from_i64(i64::from(LOAN_MAXIMUM_PAYMENTS_PER_TRANSACTION))
    {
        return Ok(normal_cost.saturating_mul(tx::LOAN_MAXIMUM_FEE_INCREMENTS));
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
    Ok(normal_cost.saturating_mul(increments))
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

    #[test]
    fn loan_pay_base_fee_lookup_distinguishes_absence_from_storage_failure() {
        assert_eq!(base_fee_lookup::<u8>(Ok(None)), Ok(None));
        assert_eq!(base_fee_lookup(Ok(Some(7_u8))), Ok(Some(7)));
        assert_eq!(
            base_fee_lookup::<u8>(Err(ledger::ViewError::Conversion(
                "injected loan base-fee SHAMap read failure".to_owned(),
            ))),
            Err(Ter::TEF_BAD_LEDGER)
        );
    }
}
