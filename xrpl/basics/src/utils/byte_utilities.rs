//! Rust equivalent of `xrpl/basics/ByteUtilities.h`.

use std::ops::Mul;

/// Convert a count into bytes at kilobyte granularity.
pub fn kilobytes<T>(value: T) -> T
where
    T: Copy + From<u16> + Mul<Output = T>,
{
    value * T::from(1024u16)
}

/// Convert a count into bytes at megabyte granularity.
pub fn megabytes<T>(value: T) -> T
where
    T: Copy + From<u16> + Mul<Output = T>,
{
    kilobytes(kilobytes(value))
}

#[cfg(test)]
mod tests {
    use super::{kilobytes, megabytes};

    #[test]
    fn matches_cpp_static_assert_examples() {
        assert_eq!(kilobytes(2u32), 2048u32);
        assert_eq!(megabytes(3u32), 3_145_728u32);
    }

    #[test]
    fn preserves_target_integer_type() {
        let as_usize: usize = kilobytes(32usize);
        let as_u64: u64 = megabytes(1u64);

        assert_eq!(as_usize, 32 * 1024);
        assert_eq!(as_u64, 1024 * 1024);
    }
}
