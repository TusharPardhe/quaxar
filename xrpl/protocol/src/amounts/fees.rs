//! Fee calculation logic from `xrpl/protocol/Fees.h`.

use crate::{XRPAmount, feature_xrp_fees};

pub fn calculate_base_fee(
    base_fee: u64,
    load_factor: u32,
    reference_fee_units: u32,
    rules: &crate::Rules,
) -> XRPAmount {
    if rules.enabled(&feature_xrp_fees()) {
        // New fee logic when featureXRPFees is enabled.
        // In reference, this often scales the base fee by the load factor.
        let scaled = (base_fee as u128 * load_factor as u128) / 1024;
        return XRPAmount::from_drops(scaled as i64);
    }

    // Legacy fee logic.
    let drops =
        (base_fee as u128 * load_factor as u128 * reference_fee_units as u128) / (1024 * 10);
    XRPAmount::from_drops(drops as i64)
}

pub fn calculate_reserve(base_reserve: u32, increment_reserve: u32, owner_count: u32) -> XRPAmount {
    let drops = base_reserve as i64 + (owner_count as i64 * increment_reserve as i64);
    XRPAmount::from_drops(drops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_calculation_matches_expected() {
        // base 10 XRP (10,000,000 drops), increment 2 XRP (2,000,000 drops)
        let reserve = calculate_reserve(10_000_000, 2_000_000, 1);
        assert_eq!(reserve.drops(), 12_000_000);
    }
}
