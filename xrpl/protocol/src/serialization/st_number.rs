//!
//! This now ports:
//! - the ambient runtime switch used by `NumberSO`, and
//! - the bounded string/JSON-like number parsing rules behind
//!   `partsFromString(...)` and the front half of `numberFromJson(...)`.
//!
//! It also ports a thin `STNumber` value wrapper with `getText(...)` and
//! `isDefault(...)`-style behavior.
//!

use basics::{
    local_value::LocalValue,
    number::{
        MANTISSA_LARGE_MAX, MANTISSA_LARGE_MIN, MANTISSA_SMALL_MAX, MANTISSA_SMALL_MIN,
        NUMBER_MAX_EXPONENT, NUMBER_MAX_REP, NUMBER_MIN_EXPONENT, NumberParts as RuntimeNumber,
        RoundingMode, get_mantissa_scale, get_rounding_mode,
    },
};
use std::{fmt, sync::OnceLock};

use crate::{
    Asset, JsonOptions, JsonValue, MPTAmount, SField, SerialIter, SerializedTypeId, Serializer,
    StBase, StBaseCore, XRPAmount, downcast_stbase_ref, sf_generic,
};

fn st_number_switchover_ref() -> &'static LocalValue<bool> {
    static ST_NUMBER_SWITCHOVER: OnceLock<LocalValue<bool>> = OnceLock::new();
    ST_NUMBER_SWITCHOVER.get_or_init(|| LocalValue::new(true))
}

pub fn get_st_number_switchover() -> bool {
    st_number_switchover_ref().get_cloned()
}

pub fn set_st_number_switchover(enabled: bool) {
    st_number_switchover_ref().set(enabled);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NumberParts {
    pub mantissa: u64,
    pub exponent: i32,
    pub negative: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberJsonInput<'a> {
    Int(i64),
    UInt(u64),
    String(&'a str),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberPartsError {
    NotANumber(String),
    MantissaOverflow,
    ExponentOverflow,
}

/// Thin value wrapper over the currently normalized runtime `Number` type.
///
/// This deliberately stops short of the reference serializer and `STBase` plumbing,
/// but it does preserve the value-centric behavior that the current Rust
/// callers can observe directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct STNumber {
    core: StBaseCore,
    value: RuntimeNumber,
    associated_asset: Option<Asset>,
}

impl Default for STNumber {
    fn default() -> Self {
        Self::new(RuntimeNumber::zero())
    }
}

impl STNumber {
    pub fn new(value: RuntimeNumber) -> Self {
        Self {
            core: StBaseCore::with_field(sf_generic()),
            value,
            associated_asset: None,
        }
    }

    pub fn with_field(field: &'static SField, value: RuntimeNumber) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value,
            associated_asset: None,
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        let mantissa = sit.geti64();
        let exponent = sit.geti32();
        let value =
            RuntimeNumber::try_from_external_parts(mantissa, exponent, get_mantissa_scale())
                .unwrap_or_else(|_| {
                    RuntimeNumber::unchecked(mantissa < 0, mantissa.unsigned_abs(), exponent)
                });
        Self::with_field(field, value)
    }

    pub const fn value(self) -> RuntimeNumber {
        self.value
    }

    pub fn set_value(&mut self, value: RuntimeNumber) {
        self.value = value;
    }

    pub fn associated_asset(&self) -> Option<Asset> {
        self.associated_asset
    }

    pub fn associate_asset(&mut self, asset: Asset) {
        self.value = round_value_to_asset(asset, self.value);
        self.associated_asset = Some(asset);
    }

    pub fn get_text(&self) -> String {
        self.value.to_string()
    }

    pub fn is_default(&self) -> bool {
        self.value == RuntimeNumber::zero()
    }

    pub fn from_json_input(input: NumberJsonInput<'_>) -> Result<Self, NumberPartsError> {
        normalized_parts_from_json_input(input).map(Self::new)
    }
}

impl From<RuntimeNumber> for STNumber {
    fn from(value: RuntimeNumber) -> Self {
        Self::new(value)
    }
}

impl From<STNumber> for RuntimeNumber {
    fn from(value: STNumber) -> Self {
        value.value
    }
}

impl fmt::Display for STNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_text())
    }
}

impl StBase for STNumber {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        &mut self.core
    }

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::Number
    }

    fn text(&self) -> String {
        self.get_text()
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        JsonValue::String(self.get_text())
    }

    fn add(&self, serializer: &mut Serializer) {
        let value = if self.fname().should_meta(SField::S_MD_NEEDS_ASSET) {
            self.associated_asset
                .map(|asset| round_value_to_asset(asset, self.value))
                .unwrap_or(self.value)
        } else {
            self.value
        };

        // Serialize zero as (0, 0). The canonical zero has exponent=NUMBER_ZERO_EXPONENT
        // (i32::MIN = 0x80000000), whose high byte 0x80 is parsed by get_field_id as
        // type=8 (Account) with name=0, misaligning the entire deserialization stream.
        // Deserializing (0, 0) via try_from_external_parts(0, 0, scale) normalizes back
        // to zero() correctly, so the round-trip is preserved.
        let (mantissa, exponent) = if value.mantissa == 0 {
            (0i64, 0i32)
        } else {
            value.external_parts().unwrap_or_else(|_| {
                panic!("STNumber must serialize from an exact normalized value")
            })
        };
        serializer.add_integer(mantissa);
        serializer.add_integer(exponent);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).value == self.value
    }

    fn is_default(&self) -> bool {
        self.value == RuntimeNumber::zero()
    }
}

impl NumberParts {
    pub const fn zero() -> Self {
        Self {
            mantissa: 0,
            exponent: 0,
            negative: false,
        }
    }

    pub fn from_signed_integer(value: i64) -> Self {
        if value >= 0 {
            Self {
                mantissa: value as u64,
                exponent: 0,
                negative: false,
            }
        } else {
            Self {
                mantissa: value.unsigned_abs(),
                exponent: 0,
                negative: true,
            }
        }
    }

    pub const fn from_unsigned_integer(value: u64) -> Self {
        Self {
            mantissa: value,
            exponent: 0,
            negative: false,
        }
    }

    pub fn from_json_input(input: NumberJsonInput<'_>) -> Result<Self, NumberPartsError> {
        match input {
            NumberJsonInput::Int(value) => Ok(Self::from_signed_integer(value)),
            NumberJsonInput::UInt(value) => Ok(Self::from_unsigned_integer(value)),
            NumberJsonInput::String(value) => parts_from_string(value),
            NumberJsonInput::Other => Err(NumberPartsError::NotANumber("not a number".to_owned())),
        }
    }

    pub const fn is_zero(self) -> bool {
        self.mantissa == 0
    }
}

fn round_value_to_asset(asset: Asset, value: RuntimeNumber) -> RuntimeNumber {
    match asset {
        Asset::Issue(issue) if issue.native() => RuntimeNumber::from(
            XRPAmount::from_number(value)
                .expect("native STNumber values should stay representable as XRPAmount"),
        ),
        Asset::Issue(_) => value,
        Asset::MPTIssue(_) => RuntimeNumber::from(
            MPTAmount::from_number(value)
                .expect("MPT STNumber values should stay representable as MPTAmount"),
        ),
    }
}

pub fn parts_from_string(number: &str) -> Result<NumberParts, NumberPartsError> {
    if number.is_empty() {
        return Err(NumberPartsError::NotANumber(number.to_owned()));
    }

    let bytes = number.as_bytes();
    let mut index = 0usize;
    let mut negative = false;

    if matches!(bytes[index], b'+' | b'-') {
        negative = bytes[index] == b'-';
        index += 1;
        if index == bytes.len() {
            return Err(NumberPartsError::NotANumber(number.to_owned()));
        }
    }

    let integer_start = index;
    match bytes[index] {
        b'0' => {
            index += 1;
            if index < bytes.len() && bytes[index].is_ascii_digit() {
                return Err(NumberPartsError::NotANumber(number.to_owned()));
            }
        }
        b'1'..=b'9' => {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
        }
        _ => return Err(NumberPartsError::NotANumber(number.to_owned())),
    }

    let integer_part = &number[integer_start..index];
    let mut digits = integer_part.to_owned();
    let mut exponent = 0i32;

    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if fraction_start == index {
            return Err(NumberPartsError::NotANumber(number.to_owned()));
        }

        let fraction = &number[fraction_start..index];
        digits.push_str(fraction);
        exponent =
            -(i32::try_from(fraction.len()).map_err(|_| NumberPartsError::ExponentOverflow)?);
    }

    if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
        index += 1;
        if index == bytes.len() {
            return Err(NumberPartsError::NotANumber(number.to_owned()));
        }

        let mut exponent_negative = false;
        if matches!(bytes[index], b'+' | b'-') {
            exponent_negative = bytes[index] == b'-';
            index += 1;
            if index == bytes.len() {
                return Err(NumberPartsError::NotANumber(number.to_owned()));
            }
        }

        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if exponent_start == index {
            return Err(NumberPartsError::NotANumber(number.to_owned()));
        }

        let parsed_exponent = number[exponent_start..index]
            .parse::<i32>()
            .map_err(|_| NumberPartsError::ExponentOverflow)?;
        exponent = if exponent_negative {
            exponent
                .checked_sub(parsed_exponent)
                .ok_or(NumberPartsError::ExponentOverflow)?
        } else {
            exponent
                .checked_add(parsed_exponent)
                .ok_or(NumberPartsError::ExponentOverflow)?
        };
    }

    if index != bytes.len() {
        return Err(NumberPartsError::NotANumber(number.to_owned()));
    }

    Ok(NumberParts {
        mantissa: parse_mantissa_digits(&digits)?,
        exponent,
        negative,
    })
}

fn parse_mantissa_digits(digits: &str) -> Result<u64, NumberPartsError> {
    digits
        .parse::<u64>()
        .map_err(|_| NumberPartsError::MantissaOverflow)
}

pub fn parts_from_json_input(input: NumberJsonInput<'_>) -> Result<NumberParts, NumberPartsError> {
    NumberParts::from_json_input(input)
}

pub fn normalized_parts_from_json_input(
    input: NumberJsonInput<'_>,
) -> Result<RuntimeNumber, NumberPartsError> {
    normalize_runtime_parts(parts_from_json_input(input)?)
}

pub fn normalized_parts_from_string(number: &str) -> Result<RuntimeNumber, NumberPartsError> {
    normalize_runtime_parts(parts_from_string(number)?)
}

pub fn number_from_json_input(input: NumberJsonInput<'_>) -> Result<STNumber, NumberPartsError> {
    STNumber::from_json_input(input)
}

fn normalize_runtime_parts(parts: NumberParts) -> Result<RuntimeNumber, NumberPartsError> {
    let (min_mantissa, max_mantissa) = match get_mantissa_scale() {
        basics::number::MantissaScale::Small => (MANTISSA_SMALL_MIN, MANTISSA_SMALL_MAX),
        basics::number::MantissaScale::LargeLegacy => (MANTISSA_LARGE_MIN, MANTISSA_LARGE_MAX),
        basics::number::MantissaScale::Large => (MANTISSA_LARGE_MIN, MANTISSA_LARGE_MAX),
    };

    normalize_parts_with_rounding(parts, min_mantissa, max_mantissa, get_rounding_mode())
}

fn normalize_parts_with_rounding(
    parts: NumberParts,
    min_mantissa: u64,
    max_mantissa: u64,
    rounding_mode: RoundingMode,
) -> Result<RuntimeNumber, NumberPartsError> {
    if parts.mantissa == 0 {
        return Ok(RuntimeNumber::zero());
    }

    let negative = parts.negative;
    let mut mantissa = u128::from(parts.mantissa);
    let mut exponent = parts.exponent;
    let min_mantissa = u128::from(min_mantissa);
    let max_mantissa = u128::from(max_mantissa);
    let mut guard = GuardDigits::default();
    if negative {
        guard.set_negative();
    }

    while mantissa < min_mantissa && exponent > NUMBER_MIN_EXPONENT {
        mantissa *= 10;
        exponent -= 1;
    }

    while mantissa > max_mantissa {
        if exponent >= NUMBER_MAX_EXPONENT {
            return Err(NumberPartsError::ExponentOverflow);
        }
        guard.push((mantissa % 10) as u8);
        mantissa /= 10;
        exponent += 1;
    }

    if exponent < NUMBER_MIN_EXPONENT || mantissa < min_mantissa {
        return Ok(RuntimeNumber::zero());
    }

    if mantissa > NUMBER_MAX_REP as u128 {
        if exponent >= NUMBER_MAX_EXPONENT {
            return Err(NumberPartsError::ExponentOverflow);
        }
        guard.push((mantissa % 10) as u8);
        mantissa /= 10;
        exponent += 1;
    }

    let mut mantissa = u64::try_from(mantissa).map_err(|_| NumberPartsError::MantissaOverflow)?;

    if should_round_up(&guard, rounding_mode, mantissa) {
        mantissa = mantissa
            .checked_add(1)
            .ok_or(NumberPartsError::MantissaOverflow)?;
        if mantissa > max_mantissa as u64 || mantissa > NUMBER_MAX_REP as u64 {
            mantissa /= 10;
            exponent = exponent
                .checked_add(1)
                .ok_or(NumberPartsError::ExponentOverflow)?;
        }
    }

    if mantissa < min_mantissa as u64 {
        mantissa = mantissa
            .checked_mul(10)
            .ok_or(NumberPartsError::MantissaOverflow)?;
        exponent = exponent
            .checked_sub(1)
            .ok_or(NumberPartsError::ExponentOverflow)?;
    }

    if exponent < NUMBER_MIN_EXPONENT {
        return Ok(RuntimeNumber::zero());
    }
    if exponent > NUMBER_MAX_EXPONENT {
        return Err(NumberPartsError::ExponentOverflow);
    }

    Ok(RuntimeNumber::unchecked(
        negative && mantissa != 0,
        mantissa,
        exponent,
    ))
}

fn should_round_up(guard: &GuardDigits, rounding_mode: RoundingMode, mantissa: u64) -> bool {
    match guard.round(rounding_mode) {
        1 => true,
        0 => (mantissa & 1) == 1,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct GuardDigits {
    digits: u64,
    has_extra: bool,
    negative: bool,
}

impl GuardDigits {
    fn set_negative(&mut self) {
        self.negative = true;
    }

    fn push(&mut self, digit: u8) {
        self.has_extra = self.has_extra || ((self.digits & 0xF) != 0);
        self.digits >>= 4;
        self.digits |= u64::from(digit & 0x0F) << 60;
    }

    fn round(&self, rounding_mode: RoundingMode) -> i8 {
        match rounding_mode {
            RoundingMode::TowardsZero => -1,
            RoundingMode::Downward => {
                if self.negative && (self.digits > 0 || self.has_extra) {
                    1
                } else {
                    -1
                }
            }
            RoundingMode::Upward => {
                if self.negative {
                    -1
                } else if self.digits > 0 || self.has_extra {
                    1
                } else {
                    -1
                }
            }
            RoundingMode::ToNearest => {
                let half = 0x5000_0000_0000_0000;
                if self.digits > half {
                    1
                } else if self.digits < half {
                    -1
                } else if self.has_extra {
                    1
                } else {
                    0
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct NumberSo {
    saved: bool,
}

impl NumberSo {
    pub fn new(enabled: bool) -> Self {
        let saved = get_st_number_switchover();
        set_st_number_switchover(enabled);
        Self { saved }
    }
}

impl Drop for NumberSo {
    fn drop(&mut self) {
        set_st_number_switchover(self.saved);
    }
}
