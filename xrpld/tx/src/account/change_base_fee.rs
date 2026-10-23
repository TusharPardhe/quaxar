//! Current `Change::calculateBaseFee(...)` wrapper used by the
//! pseudo-transaction change family.

pub fn run_change_calculate_base_fee<Fee>(zero_fee: Fee) -> Fee
where
    Fee: Copy,
{
    zero_fee
}
