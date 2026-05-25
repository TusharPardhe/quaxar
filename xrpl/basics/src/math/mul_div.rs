//! Rust port of the current `xrpl::mulDiv` behavior.
//!
//! The reference implementation lives in:
//! - `include/xrpl/basics/mulDiv.h`
//! - `src/libxrpl/basics/the reference source`
//!
//! Behavior we preserve:
//! - compute `value * mul / div` in a widened integer type,
//! - return `None` if the final result does not fit in `u64`.
//!
//! Notes for a JS/TS engineer:
//! - `u64` is an unsigned 64-bit integer.
//! - `Option<u64>` means "either a number or no value".
//! - `Some(x)` is like a present value.
//! - `None` is like an explicit `undefined`/missing result.

/// Maximum allowed result, matching `std::numeric_limits<std::uint64_t>::max()`
/// in the reference implementation.
pub const MULDIV_MAX: u64 = u64::MAX;

/// Compute `value * mul / div` with widened intermediate arithmetic.
///
/// This mirrors the the reference implementation implementation by:
/// - promoting the multiplication to `u128`,
/// - dividing in widened precision,
/// - returning `None` if the final answer exceeds `u64::MAX`.
///
/// Like the the reference implementation function, this implementation assumes `div != 0`.
pub fn mul_div(value: u64, mul: u64, div: u64) -> Option<u64> {
    let result = (u128::from(value) * u128::from(mul)) / u128::from(div);

    if result > u128::from(MULDIV_MAX) {
        return None;
    }

    Some(result as u64)
}

#[cfg(test)]
mod tests {
    use super::mul_div;

    #[test]
    fn matches_cpp_mul_div_examples() {
        let max = u64::MAX;
        let max32 = u32::MAX as u64;

        let result = mul_div(85, 20, 5);
        assert_eq!(result, Some(340));

        let result = mul_div(20, 85, 5);
        assert_eq!(result, Some(340));

        let result = mul_div(0, max - 1, max - 3);
        assert_eq!(result, Some(0));

        let result = mul_div(max - 1, 0, max - 3);
        assert_eq!(result, Some(0));

        let result = mul_div(max, 2, max / 2);
        assert_eq!(result, Some(4));

        let result = mul_div(max, 1000, max / 1000);
        assert_eq!(result, Some(1_000_000));

        let result = mul_div(max, 1000, max / 1001);
        assert_eq!(result, Some(1_001_000));

        let result = mul_div(max32 + 1, max32 + 1, 5);
        assert_eq!(result, Some(3_689_348_814_741_910_323));

        // Overflow case from the existing reference gtest.
        let result = mul_div(max - 1, max - 2, 5);
        assert_eq!(result, None);
    }
}
