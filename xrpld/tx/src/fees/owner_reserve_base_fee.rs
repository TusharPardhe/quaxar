//! Current `Transactor::calculateOwnerReserveFee(...)` helper.
//!
//! This ports the current returned fee value and the adjacent developer-only
//! invariant shape:
//!
//! - the returned owner-reserve fee is `view.fees().increment`, and
//! - the current reasonability check is `increment > base * 100`.

use std::ops::Mul;

pub const OWNER_RESERVE_FEE_REASONABLE_ASSERT_MESSAGE: &str =
    "xrpl::Transactor::calculateOwnerReserveFee : Owner reserve is reasonable";

pub fn owner_reserve_fee_is_reasonable<Fee>(base_fee: Fee, owner_reserve_fee: Fee) -> bool
where
    Fee: Copy + PartialOrd + Mul<u64, Output = Fee>,
{
    owner_reserve_fee > (base_fee * 100_u64)
}

pub fn run_owner_reserve_base_fee<Fee>(base_fee: Fee, owner_reserve_fee: Fee) -> Fee
where
    Fee: Copy + PartialOrd + Mul<u64, Output = Fee>,
{
    debug_assert!(
        owner_reserve_fee_is_reasonable(base_fee, owner_reserve_fee),
        "{OWNER_RESERVE_FEE_REASONABLE_ASSERT_MESSAGE}"
    );

    owner_reserve_fee
}

#[cfg(test)]
mod tests {
    use super::{owner_reserve_fee_is_reasonable, run_owner_reserve_base_fee};

    #[test]
    fn owner_reserve_fee_reasonability_requires_strictly_more_than_hundred_base_fees() {
        assert!(!owner_reserve_fee_is_reasonable(10_u64, 1_000_u64));
        assert!(owner_reserve_fee_is_reasonable(10_u64, 1_001_u64));
    }

    #[test]
    fn owner_reserve_base_fee_returns_increment() {
        let fee = run_owner_reserve_base_fee(10_u64, 2_000_u64);

        assert_eq!(fee, 2_000_u64);
    }
}
