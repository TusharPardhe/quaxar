//! Shared `STAmount` range and wire helpers.
//!
//! This ports the reference constants that currently live on `xrpl/protocol/STAmount.h`
//! into a narrow Rust seam that `IOUAmount` and `STAmount` can share without
//! recreating the same literals in multiple modules.

use crate::mpt_amount::MAX_MP_TOKEN_AMOUNT;

pub const ST_AMOUNT_MIN_OFFSET: i32 = -96;
pub const ST_AMOUNT_MAX_OFFSET: i32 = 80;

pub const ST_AMOUNT_MIN_MANTISSA: u64 = 1_000_000_000_000_000;
pub const ST_AMOUNT_MAX_MANTISSA: u64 = ST_AMOUNT_MIN_MANTISSA * 10 - 1;

pub const ST_AMOUNT_MAX_NATIVE: u64 = 9_000_000_000_000_000_000;
pub const ST_AMOUNT_MAX_NATIVE_NETWORK: u64 = 100_000_000_000_000_000;

pub const ST_AMOUNT_ISSUED_CURRENCY_FLAG: u64 = 0x8_000_000_000_000_000;
pub const ST_AMOUNT_POSITIVE_FLAG: u64 = 0x4_000_000_000_000_000;
pub const ST_AMOUNT_MP_TOKEN_FLAG: u64 = 0x2_000_000_000_000_000;
pub const ST_AMOUNT_VALUE_MASK: u64 = !(ST_AMOUNT_POSITIVE_FLAG | ST_AMOUNT_MP_TOKEN_FLAG);

pub const ST_AMOUNT_ISSUED_HEADER_SHIFT: u32 = 64 - 10;
pub const ST_AMOUNT_ISSUED_HEADER_MASK: u64 = 1023u64 << ST_AMOUNT_ISSUED_HEADER_SHIFT;
pub const ST_AMOUNT_ISSUED_NON_NATIVE_BITS: u16 = 512;
pub const ST_AMOUNT_ISSUED_POSITIVE_BITS: u16 = 256;
pub const ST_AMOUNT_ISSUED_EXPONENT_BIAS: i32 = 97;

pub const fn is_valid_st_amount_offset(offset: i32) -> bool {
    offset >= ST_AMOUNT_MIN_OFFSET && offset <= ST_AMOUNT_MAX_OFFSET
}

pub const fn is_valid_st_amount_mantissa(value: u64) -> bool {
    value >= ST_AMOUNT_MIN_MANTISSA && value <= ST_AMOUNT_MAX_MANTISSA
}

pub const fn is_valid_st_amount_nonzero_iou(value: u64, offset: i32) -> bool {
    is_valid_st_amount_mantissa(value) && is_valid_st_amount_offset(offset)
}

pub const fn is_valid_st_amount_native_internal_value(value: u64) -> bool {
    value <= ST_AMOUNT_MAX_NATIVE
}

pub const fn is_valid_st_amount_native_network_value(value: u64) -> bool {
    value <= ST_AMOUNT_MAX_NATIVE_NETWORK
}

pub const fn is_valid_st_amount_mpt_value(value: u64) -> bool {
    value <= MAX_MP_TOKEN_AMOUNT as u64
}

pub const fn issued_zero_header_bits() -> u16 {
    ST_AMOUNT_ISSUED_NON_NATIVE_BITS
}

pub const fn issued_zero_header_word() -> u64 {
    ST_AMOUNT_ISSUED_CURRENCY_FLAG
}

pub const fn is_issued_zero_header_bits(header_bits: u16) -> bool {
    header_bits == issued_zero_header_bits()
}

pub const fn issued_header_bits_from_word(word: u64) -> u16 {
    (word >> ST_AMOUNT_ISSUED_HEADER_SHIFT) as u16
}

pub const fn issued_mantissa_from_word(word: u64) -> u64 {
    word & !ST_AMOUNT_ISSUED_HEADER_MASK
}

pub const fn issued_header_is_negative(header_bits: u16) -> bool {
    (header_bits & ST_AMOUNT_ISSUED_POSITIVE_BITS) == 0
}

pub const fn issued_exponent_from_nonzero_header_bits(header_bits: u16) -> i32 {
    ((header_bits & 255) as i32) - ST_AMOUNT_ISSUED_EXPONENT_BIAS
}

pub const fn issued_header_bits(offset: i32, is_negative: bool) -> Option<u16> {
    if !is_valid_st_amount_offset(offset) {
        return None;
    }

    let offset_bits = (offset + ST_AMOUNT_ISSUED_EXPONENT_BIAS) as u16;
    let mut header_bits = ST_AMOUNT_ISSUED_NON_NATIVE_BITS | offset_bits;
    if !is_negative {
        header_bits |= ST_AMOUNT_ISSUED_POSITIVE_BITS;
    }
    Some(header_bits)
}

pub const fn issued_header_word(mantissa: u64, offset: i32, is_negative: bool) -> Option<u64> {
    if !is_valid_st_amount_nonzero_iou(mantissa, offset) {
        return None;
    }

    match issued_header_bits(offset, is_negative) {
        Some(header_bits) => {
            Some(mantissa | ((header_bits as u64) << ST_AMOUNT_ISSUED_HEADER_SHIFT))
        }
        None => None,
    }
}

pub const fn native_wire_word(value: u64, is_negative: bool) -> u64 {
    if is_negative {
        value
    } else {
        value | ST_AMOUNT_POSITIVE_FLAG
    }
}

pub const fn mpt_wire_header_byte(is_negative: bool) -> u8 {
    let mut header = (ST_AMOUNT_MP_TOKEN_FLAG >> 56) as u8;
    if !is_negative {
        header |= (ST_AMOUNT_POSITIVE_FLAG >> 56) as u8;
    }
    header
}
