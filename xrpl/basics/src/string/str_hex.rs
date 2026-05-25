//! Rust equivalent of `xrpl/basics/strHex.h`.
//!
//! The reference version has an iterator overload and a collection overload.
//! In Rust, `AsRef<[u8]>` is the natural equivalent for collection-like byte
//! inputs, while `str_hex_iter` covers generic iterators.

use std::borrow::Borrow;

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

/// Convert any byte slice-like value into an uppercase hexadecimal string.
pub fn str_hex<T>(bytes: T) -> String
where
    T: AsRef<[u8]>,
{
    str_hex_iter(bytes.as_ref())
}

/// Convert a generic iterator of bytes into an uppercase hexadecimal string.
pub fn str_hex_iter<I, B>(bytes: I) -> String
where
    I: IntoIterator<Item = B>,
    B: Borrow<u8>,
{
    let iter = bytes.into_iter();
    let (lower, _) = iter.size_hint();
    let mut result = String::with_capacity(lower.saturating_mul(2));

    for byte in iter {
        let value = *byte.borrow();
        result.push(HEX_DIGITS[(value >> 4) as usize] as char);
        result.push(HEX_DIGITS[(value & 0x0f) as usize] as char);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::{str_hex, str_hex_iter};

    #[test]
    fn matches_expected_uppercase_hex() {
        assert_eq!(str_hex([]), "");
        assert_eq!(str_hex([0x00]), "00");
        assert_eq!(str_hex([0x0a, 0xbc, 0xff]), "0ABCFF");
    }

    #[test]
    fn supports_collections_and_iterators() {
        let data = vec![0xde, 0xad, 0xbe, 0xef];

        assert_eq!(str_hex(&data), "DEADBEEF");
        assert_eq!(str_hex_iter(data.iter()), "DEADBEEF");
    }

    #[test]
    fn supports_non_slice_iterators() {
        let values = [0x12, 0x34, 0x56];

        assert_eq!(str_hex_iter(values), "123456");
    }
}
