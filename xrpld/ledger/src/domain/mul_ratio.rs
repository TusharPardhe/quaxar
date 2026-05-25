//!
//! mulRatio(amount, numerator, denominator, roundUp) computes:
//!   amount * numerator / denominator
//! with the specified rounding direction. Uses 128-bit intermediate arithmetic
//! to avoid overflow, matching reference boost::multiprecision::uint128_t behavior.

use protocol::IOUAmount;

pub const QUALITY_ONE: u32 = 1_000_000_000;

const MIN_MANTISSA: i64 = 1_000_000_000_000_000;
const MIN_EXPONENT: i32 = -96;
// Kept for compatibility with the canonical IOU amount bounds; the current
// narrowed mulRatio port does not consult the upper mantissa bound directly.
#[allow(dead_code)]
const MAX_MANTISSA: i64 = 9_999_999_999_999_999;

/// Power of 10 table for efficient log10 computation
fn power10(n: u32) -> u128 {
    let mut result: u128 = 1;
    for _ in 0..n {
        result *= 10;
    }
    result
}

/// Floor(log10(v)), returns -1 for v == 0
fn log10_floor(v: u128) -> i32 {
    if v == 0 {
        return -1;
    }
    let mut index = 0i32;
    let mut threshold: u128 = 10;
    while threshold <= v && index < 38 {
        index += 1;
        threshold *= 10;
    }
    index
}

/// Ceil(log10(v))
fn log10_ceil(v: u128) -> i32 {
    if v <= 1 {
        return 0;
    }
    let floor = log10_floor(v);
    if power10(floor as u32) == v {
        floor
    } else {
        floor + 1
    }
}

/// Maximum digits that fit in i64: floor(log10(i64::MAX)) = 18
const FL64: i32 = 18;

///
/// Computes: amt * num / den with controlled rounding.
/// roundUp=true: rounds away from zero for positive, toward zero for negative
/// roundUp=false: rounds toward zero for positive, away from zero for negative
pub fn mul_ratio(amt: IOUAmount, num: u32, den: u32, round_up: bool) -> IOUAmount {
    if den == 0 {
        return IOUAmount::new();
    }
    if amt.is_zero() || num == 0 {
        return IOUAmount::new();
    }

    let neg = amt.mantissa() < 0;
    let abs_mantissa = if neg { -amt.mantissa() } else { amt.mantissa() } as u128;
    let num128 = num as u128;
    let den128 = den as u128;

    // 32-bit * 64-bit stored in 128-bit — never overflows
    let mul = abs_mantissa * num128;

    let mut low = mul / den128;
    let mut rem = mul - low * den128;
    let mut exponent = amt.exponent();

    if rem != 0 {
        // Scale up to preserve precision
        let room_to_grow = FL64 - log10_ceil(low);
        if room_to_grow > 0 {
            exponent -= room_to_grow;
            let scale = power10(room_to_grow as u32);
            low *= scale;
            rem *= scale;
        }
        let add_rem = rem / den128;
        low += add_rem;
        rem -= add_rem * den128;
    }

    // Scale down if result overflows i64
    let mut has_rem = rem != 0;
    let must_shrink = log10_ceil(low) - FL64;
    if must_shrink > 0 {
        let sav = low;
        exponent += must_shrink;
        let scale = power10(must_shrink as u32);
        low /= scale;
        if !has_rem {
            has_rem = sav - low * scale != 0;
        }
    }

    let mut mantissa = low as i64;
    if neg {
        mantissa = -mantissa;
    }

    // Normalize
    let result = IOUAmount::from_parts(mantissa, exponent).unwrap_or_default();

    if has_rem {
        // Handle rounding
        if round_up && !neg {
            if result.is_zero() {
                return IOUAmount::from_parts(MIN_MANTISSA, MIN_EXPONENT).unwrap_or_default();
            }
            return IOUAmount::from_parts(result.mantissa() + 1, result.exponent())
                .unwrap_or(result);
        }
        if !round_up && neg {
            if result.is_zero() {
                return IOUAmount::from_parts(-MIN_MANTISSA, MIN_EXPONENT).unwrap_or_default();
            }
            return IOUAmount::from_parts(result.mantissa() - 1, result.exponent())
                .unwrap_or(result);
        }
    }

    result
}

pub fn mul_ratio_xrp(drops: i64, num: u32, den: u32, round_up: bool) -> i64 {
    if den == 0 {
        return 0;
    }
    let neg = drops < 0;
    let abs_drops = if neg { -drops } else { drops } as u128;
    let result = abs_drops * num as u128;
    let mut quotient = result / den as u128;
    let remainder = result - quotient * den as u128;

    if remainder != 0 && ((round_up && !neg) || (!round_up && neg)) {
        quotient += 1;
    }

    let mut out = quotient as i64;
    if neg {
        out = -out;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_ratio_basic() {
        let amt = IOUAmount::from_parts(1_000_000_000_000_000, -15).unwrap(); // 1.0
        let result = mul_ratio(amt, 15, 100, false); // * 0.15
        assert_eq!(result.mantissa(), 1_500_000_000_000_000);
        assert_eq!(result.exponent(), -16);
    }

    #[test]
    fn mul_ratio_quality_one() {
        let amt = IOUAmount::from_parts(5_000_000_000_000_000, -15).unwrap(); // 5.0
        let result = mul_ratio(amt, QUALITY_ONE, QUALITY_ONE, false);
        assert_eq!(result.mantissa(), amt.mantissa());
        assert_eq!(result.exponent(), amt.exponent());
    }

    #[test]
    fn mul_ratio_xrp_basic() {
        assert_eq!(mul_ratio_xrp(1000, 15, 100, false), 150);
        assert_eq!(mul_ratio_xrp(1000, 15, 100, true), 150);
        assert_eq!(mul_ratio_xrp(1001, 15, 100, false), 150); // rounds down
        assert_eq!(mul_ratio_xrp(1001, 15, 100, true), 151); // rounds up
    }
}
