//! Current the reference implementation wrapper.
//!
//! This ports the deterministic outer behavior around:
//!
//! - starting from the lower `Transactor::calculateBaseFee(...)` result, and
//! - adding `base * (32 + fulfillment_size / 16)` only when `sfFulfillment`
//!   is present.

use std::ops::{Add, Mul};

pub fn run_escrow_finish_calculate_base_fee<Fee>(
    transactor_base_fee: Fee,
    ledger_base_fee: Fee,
    fulfillment_size: Option<usize>,
) -> Fee
where
    Fee: Copy + Add<Output = Fee> + Mul<u64, Output = Fee>,
{
    let extra_fee = fulfillment_size
        .map(|size| {
            32_u64 + u64::try_from(size / 16).expect("fulfillment size should fit into u64")
        })
        .map(|multiplier| ledger_base_fee * multiplier);

    match extra_fee {
        Some(extra_fee) => transactor_base_fee + extra_fee,
        None => transactor_base_fee,
    }
}

#[cfg(test)]
mod tests {
    use super::run_escrow_finish_calculate_base_fee;

    #[test]
    fn escrow_finish_calculate_base_fee_keeps_transactor_fee_without_fulfillment() {
        let fee = run_escrow_finish_calculate_base_fee(10_u64, 10_u64, None);

        assert_eq!(fee, 10);
    }

    #[test]
    fn escrow_finish_calculate_base_fee_adds_base_times_thirty_two_for_empty_fulfillment() {
        let fee = run_escrow_finish_calculate_base_fee(10_u64, 10_u64, Some(0));

        assert_eq!(fee, 330);
    }

    #[test]
    fn escrow_finish_calculate_base_fee_uses_integer_chunks_of_sixteen_bytes() {
        let fee = run_escrow_finish_calculate_base_fee(10_u64, 10_u64, Some(31));

        assert_eq!(fee, 340);
    }
}
