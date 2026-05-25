//! Rust port of `xrpl/basics/safe_cast.h`.
//!
//! Rust already distinguishes infallible conversions (`From`/`Into`) from
//! potentially lossy ones (`as`, `TryFrom`). This module uses that split
//! directly:
//! - `safe_cast` only compiles for infallible conversions.
//! - `unsafe_cast` performs an explicit Rust `as` cast for primitive integers.
//!
//! ```rust
//! use basics::safe_cast::safe_cast;
//!
//! let widened: u16 = safe_cast(7u8);
//! assert_eq!(widened, 7u16);
//! ```
//!
//! ```compile_fail
//! use basics::safe_cast::safe_cast;
//!
//! let _narrowed: u8 = safe_cast(300u16);
//! ```

use std::convert::Infallible;

/// Compile-time checked infallible cast.
pub fn safe_cast<Dest, Src>(value: Src) -> Dest
where
    Dest: TryFrom<Src, Error = Infallible>,
{
    match Dest::try_from(value) {
        Ok(result) => result,
        Err(never) => match never {},
    }
}

/// Trait backing explicit potentially lossy integer casts.
pub trait UnsafeCastFrom<Src> {
    fn unsafe_cast_from(value: Src) -> Self;
}

/// Explicit cast for primitive integers.
pub fn unsafe_cast<Dest, Src>(value: Src) -> Dest
where
    Dest: UnsafeCastFrom<Src>,
{
    Dest::unsafe_cast_from(value)
}

macro_rules! impl_unsafe_cast_to {
    ($dest:ty => $($src:ty),* $(,)?) => {
        $(
            impl UnsafeCastFrom<$src> for $dest {
                fn unsafe_cast_from(value: $src) -> Self {
                    value as $dest
                }
            }
        )*
    };
}

impl_unsafe_cast_to!(i8 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(i16 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(i32 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(i64 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(i128 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(isize => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(u8 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(u16 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(u32 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(u64 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(u128 => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
impl_unsafe_cast_to!(usize => i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

#[cfg(test)]
mod tests {
    use super::{safe_cast, unsafe_cast};

    #[test]
    fn safe_cast_matches_infallible_integer_conversions() {
        let widened_unsigned: u64 = safe_cast(255u8);
        let widened_signed: i64 = safe_cast(255u8);
        let signed_widen: i32 = safe_cast(-7i8);

        assert_eq!(widened_unsigned, 255u64);
        assert_eq!(widened_signed, 255i64);
        assert_eq!(signed_widen, -7i32);
    }

    #[test]
    fn unsafe_cast_matches_rust_as_semantics() {
        let narrowed: u8 = unsafe_cast(300u16);
        let signed_to_unsigned: u8 = unsafe_cast(-1i8);
        let unsigned_to_signed: i8 = unsafe_cast(255u8);

        assert_eq!(narrowed, 44u8);
        assert_eq!(signed_to_unsigned, 255u8);
        assert_eq!(unsigned_to_signed, -1i8);
    }

    #[test]
    fn unsafe_cast_allows_explicit_narrowing() {
        let value: u16 = 1025;
        let narrowed: u8 = unsafe_cast(value);
        assert_eq!(narrowed, 1u8);
    }
}
