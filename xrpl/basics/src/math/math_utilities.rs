//! Rust equivalent of `xrpl/basics/MathUtilities.h`.

/// Calculate a percentage, rounded up, and capped to the range `[0, 100]`.
///
/// This mirrors the reference helper exactly:
/// - `total` must not be zero,
/// - `count` is capped at `total`,
/// - the percentage rounds up to the next integer.
pub fn calculate_percent(count: usize, total: usize) -> usize {
    assert!(total != 0, "total cannot be zero");
    (count.min(total) * 100).div_ceil(total)
}

#[cfg(test)]
mod tests {
    use super::calculate_percent;

    #[test]
    fn matches_cpp_static_assert_examples() {
        assert_eq!(calculate_percent(1, 2), 50);
        assert_eq!(calculate_percent(0, 100), 0);
        assert_eq!(calculate_percent(100, 100), 100);
        assert_eq!(calculate_percent(200, 100), 100);
        assert_eq!(calculate_percent(1, 100), 1);
        assert_eq!(calculate_percent(1, 99), 2);
        assert_eq!(calculate_percent(6, 14), 43);
        assert_eq!(calculate_percent(29, 33), 88);
        assert_eq!(calculate_percent(1, 64), 2);
        assert_eq!(calculate_percent(0, 100_000_000), 0);
        assert_eq!(calculate_percent(1, 100_000_000), 1);
        assert_eq!(calculate_percent(50_000_000, 100_000_000), 50);
        assert_eq!(calculate_percent(50_000_001, 100_000_000), 51);
        assert_eq!(calculate_percent(99_999_999, 100_000_000), 100);
    }

    #[test]
    #[should_panic(expected = "total cannot be zero")]
    fn panics_on_zero_total() {
        let _ = calculate_percent(1, 0);
    }
}
