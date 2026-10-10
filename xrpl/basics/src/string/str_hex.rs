//! Rust equivalent of `xrpl/basics/strHex.h`.
//!
//! The reference version has an iterator overload and a collection overload.
//! In Rust, `AsRef<[u8]>` is the natural equivalent for collection-like byte
//! inputs, while `str_hex_iter` covers generic iterators.

use std::borrow::Borrow;

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

/// Two uppercase hex digits for every byte value, built at compile time.
const HEX_PAIRS: [[u8; 2]; 256] = {
    let mut table = [[0_u8; 2]; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = [HEX_DIGITS[i >> 4], HEX_DIGITS[i & 0x0f]];
        i += 1;
    }
    table
};

/// Write the uppercase hex encoding of `bytes` into `out`, which must be
/// exactly `2 * bytes.len()` long. One table lookup per byte; no allocation
/// and no `core::fmt`.
#[inline]
pub fn encode_upper_to_slice(bytes: &[u8], out: &mut [u8]) {
    assert_eq!(out.len(), bytes.len() * 2, "hex output length mismatch");
    for (pair, byte) in out.chunks_exact_mut(2).zip(bytes) {
        pair.copy_from_slice(&HEX_PAIRS[usize::from(*byte)]);
    }
}

/// Append the uppercase hex encoding of `bytes` to `out`.
#[inline]
pub fn push_hex_upper(out: &mut String, bytes: &[u8]) {
    let start = out.len();
    // SAFETY: every byte written is an ASCII hex digit, so the string stays
    // valid UTF-8; the zero fill is overwritten before the borrow ends.
    let vec = unsafe { out.as_mut_vec() };
    vec.resize(start + bytes.len() * 2, 0);
    encode_upper_to_slice(bytes, &mut vec[start..]);
}

/// Convert any byte slice-like value into an uppercase hexadecimal string.
pub fn str_hex<T>(bytes: T) -> String
where
    T: AsRef<[u8]>,
{
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    push_hex_upper(&mut out, bytes);
    out
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
        let pair = &HEX_PAIRS[usize::from(*byte.borrow())];
        result.push(char::from(pair[0]));
        result.push(char::from(pair[1]));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::{push_hex_upper, str_hex, str_hex_iter};

    fn reference(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02X}")).collect()
    }

    #[test]
    fn every_byte_value_matches_format_reference() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(str_hex(&all), reference(&all));
        assert_eq!(str_hex_iter(all.iter()), reference(&all));
    }

    #[test]
    fn pseudo_random_lengths_match_format_reference() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        for len in 0..300 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect();
            assert_eq!(str_hex(&bytes), reference(&bytes));
        }
    }

    #[test]
    fn push_appends_without_clobbering_prefix() {
        let mut out = String::from("0x");
        push_hex_upper(&mut out, &[0xAB, 0x01]);
        push_hex_upper(&mut out, &[]);
        assert_eq!(out, "0xAB01");
    }

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
