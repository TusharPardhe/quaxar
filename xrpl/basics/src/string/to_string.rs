//! Rust equivalent of `xrpl/basics/ToString.h`.
//!
//! Rust already has the standard `ToString` trait for any type implementing
//! `Display`, so this migration boundary is intentionally thin.

use std::fmt::Display;

/// Generalized string conversion for displayable values.
pub fn to_string<T>(value: T) -> String
where
    T: Display,
{
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::to_string;

    #[test]
    fn matches_cpp_bool_char_and_string_behavior() {
        assert_eq!(to_string(true), "true");
        assert_eq!(to_string(false), "false");
        assert_eq!(to_string('x'), "x");
        assert_eq!(to_string("xrpl"), "xrpl");
        assert_eq!(to_string(String::from("rust")), "rust");
    }

    #[test]
    fn matches_cpp_arithmetic_behavior() {
        assert_eq!(to_string(42), "42");
        assert_eq!(to_string(-7), "-7");
        assert_eq!(to_string(3.5), "3.5");
    }
}
