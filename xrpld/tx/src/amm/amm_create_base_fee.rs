//! Current the reference implementation wrapper.

use std::ops::Mul;

use crate::run_owner_reserve_base_fee;

pub fn run_amm_create_calculate_base_fee<Fee>(base_fee: Fee, owner_reserve_fee: Fee) -> Fee
where
    Fee: Copy + PartialOrd + Mul<u64, Output = Fee>,
{
    run_owner_reserve_base_fee(base_fee, owner_reserve_fee)
}
