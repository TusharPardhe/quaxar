//! `IOUAmount` port from `xrpl/protocol/IOUAmount.h`.

use std::{
    cmp::Ordering,
    fmt,
    ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign},
};

use basics::number::{
    MantissaScale, NUMBER_MAX_EXPONENT, NUMBER_MAX_REP, NUMBER_MIN_EXPONENT, NUMBER_ZERO_EXPONENT,
    NumberArithmeticError, NumberParts as RuntimeNumber, RoundingMode,
    current_mantissa_range as current_runtime_mantissa_range, external_to_internal_mantissa,
    get_rounding_mode,
};

use crate::st_number::get_st_number_switchover;

pub const MIN_IOU_EXPONENT: i32 = -96;
pub const MAX_IOU_EXPONENT: i32 = 80;
pub const MIN_IOU_MANTISSA: i64 = 1_000_000_000_000_000;
pub const MAX_IOU_MANTISSA: i64 = 9_999_999_999_999_999;
pub const IOU_ZERO_EXPONENT: i32 = -100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IOUAmount {
    mantissa: i64,
    exponent: i32,
}

impl Default for IOUAmount {
    fn default() -> Self {
        Self::new()
    }
}

impl IOUAmount {
    pub const fn new() -> Self {
        Self {
            mantissa: 0,
            exponent: IOU_ZERO_EXPONENT,
        }
    }

    pub fn from_parts(mantissa: i64, exponent: i32) -> Result<Self, NumberArithmeticError> {
        let mut value = Self { mantissa, exponent };
        value.normalize()?;
        Ok(value)
    }

    pub fn from_number(number: RuntimeNumber) -> Result<Self, NumberArithmeticError> {
        let normalized = normalize_runtime_number_to_range(
            number,
            MIN_IOU_MANTISSA as u64,
            MAX_IOU_MANTISSA as u64,
        )?;
        let mut value = Self {
            mantissa: signed_runtime_mantissa(normalized)?,
            exponent: normalized.exponent,
        };

        if value.exponent > MAX_IOU_EXPONENT {
            return Err(NumberArithmeticError::Overflow);
        }
        if value.exponent < MIN_IOU_EXPONENT {
            value = Self::new();
        }

        Ok(value)
    }

    pub const fn mantissa(self) -> i64 {
        self.mantissa
    }

    pub const fn exponent(self) -> i32 {
        self.exponent
    }

    pub const fn is_zero(self) -> bool {
        self.mantissa == 0
    }

    pub const fn signum(self) -> i32 {
        if self.mantissa < 0 {
            -1
        } else if self.mantissa == 0 {
            0
        } else {
            1
        }
    }

    pub const fn min_positive_amount() -> Self {
        Self {
            mantissa: MIN_IOU_MANTISSA,
            exponent: MIN_IOU_EXPONENT,
        }
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, NumberArithmeticError> {
        let mut value = self;
        value.checked_add_assign(rhs)?;
        Ok(value)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, NumberArithmeticError> {
        let mut value = self;
        value.checked_sub_assign(rhs)?;
        Ok(value)
    }

    pub fn checked_add_assign(&mut self, rhs: Self) -> Result<(), NumberArithmeticError> {
        if rhs.is_zero() {
            return Ok(());
        }

        if self.is_zero() {
            *self = rhs;
            return Ok(());
        }

        if get_st_number_switchover() {
            let sum = RuntimeNumber::from(*self).try_add(RuntimeNumber::from(rhs))?;
            *self = Self::from_number(sum)?;
            return Ok(());
        }

        let mut rhs_mantissa = rhs.mantissa;
        let mut rhs_exponent = rhs.exponent;

        while self.exponent < rhs_exponent {
            self.mantissa /= 10;
            self.exponent += 1;
        }

        while rhs_exponent < self.exponent {
            rhs_mantissa /= 10;
            rhs_exponent += 1;
        }

        self.mantissa = self
            .mantissa
            .checked_add(rhs_mantissa)
            .ok_or(NumberArithmeticError::Overflow)?;

        if (-10..=10).contains(&self.mantissa) {
            *self = Self::new();
            return Ok(());
        }

        self.normalize()
    }

    pub fn checked_sub_assign(&mut self, rhs: Self) -> Result<(), NumberArithmeticError> {
        self.checked_add_assign(-rhs)
    }

    fn normalize(&mut self) -> Result<(), NumberArithmeticError> {
        if self.mantissa == 0 {
            *self = Self::new();
            return Ok(());
        }

        if get_st_number_switchover() {
            let runtime = runtime_number_from_external_parts(
                self.mantissa,
                self.exponent,
                current_runtime_mantissa_range().scale,
            )?;
            *self = Self::from_number(runtime)?;
            return Ok(());
        }

        self.normalize_without_stnumber()
    }

    fn normalize_without_stnumber(&mut self) -> Result<(), NumberArithmeticError> {
        let negative = self.mantissa < 0;
        let mut mantissa = i128::from(self.mantissa).unsigned_abs();
        let mut exponent = self.exponent;

        while mantissa < MIN_IOU_MANTISSA as u128 && exponent > MIN_IOU_EXPONENT {
            mantissa = mantissa
                .checked_mul(10)
                .ok_or(NumberArithmeticError::Overflow)?;
            exponent -= 1;
        }

        while mantissa > MAX_IOU_MANTISSA as u128 {
            if exponent >= MAX_IOU_EXPONENT {
                return Err(NumberArithmeticError::Overflow);
            }

            mantissa /= 10;
            exponent += 1;
        }

        if exponent < MIN_IOU_EXPONENT || mantissa < MIN_IOU_MANTISSA as u128 {
            *self = Self::new();
            return Ok(());
        }

        if exponent > MAX_IOU_EXPONENT {
            return Err(NumberArithmeticError::Overflow);
        }

        let mantissa = i64::try_from(mantissa).map_err(|_| NumberArithmeticError::Overflow)?;
        self.mantissa = if negative { -mantissa } else { mantissa };
        self.exponent = exponent;
        Ok(())
    }
}

impl TryFrom<RuntimeNumber> for IOUAmount {
    type Error = NumberArithmeticError;

    fn try_from(value: RuntimeNumber) -> Result<Self, Self::Error> {
        Self::from_number(value)
    }
}

impl From<IOUAmount> for RuntimeNumber {
    fn from(value: IOUAmount) -> Self {
        runtime_number_from_external_parts(
            value.mantissa,
            value.exponent,
            current_runtime_mantissa_range().scale,
        )
        .expect("IOUAmount should remain representable in the current Number runtime")
    }
}

impl From<IOUAmount> for bool {
    fn from(value: IOUAmount) -> Self {
        !value.is_zero()
    }
}

impl AddAssign for IOUAmount {
    fn add_assign(&mut self, rhs: Self) {
        self.checked_add_assign(rhs).expect(
            "IOUAmount addition should preserve the reference implementation overflow behavior",
        );
    }
}

impl Add for IOUAmount {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        self += rhs;
        self
    }
}

impl SubAssign for IOUAmount {
    fn sub_assign(&mut self, rhs: Self) {
        self.checked_sub_assign(rhs).expect(
            "IOUAmount subtraction should preserve the reference implementation overflow behavior",
        );
    }
}

impl Sub for IOUAmount {
    type Output = Self;

    fn sub(mut self, rhs: Self) -> Self::Output {
        self -= rhs;
        self
    }
}

impl Mul<i64> for IOUAmount {
    type Output = Self;

    fn mul(self, rhs: i64) -> Self::Output {
        let mut result = mul_ratio(
            self,
            rhs.unsigned_abs() as u32,
            1,
            get_rounding_mode() == RoundingMode::Upward,
        )
        .expect("IOUAmount multiplication overflow");
        if rhs < 0 {
            result = -result;
        }
        result
    }
}

impl MulAssign<i64> for IOUAmount {
    fn mul_assign(&mut self, rhs: i64) {
        *self = *self * rhs;
    }
}

impl Div<i64> for IOUAmount {
    type Output = Self;

    fn div(self, rhs: i64) -> Self::Output {
        let mut result = mul_ratio(
            self,
            1,
            rhs.unsigned_abs() as u32,
            get_rounding_mode() == RoundingMode::Upward,
        )
        .expect("IOUAmount division overflow");
        if rhs < 0 {
            result = -result;
        }
        result
    }
}

impl DivAssign<i64> for IOUAmount {
    fn div_assign(&mut self, rhs: i64) {
        *self = *self / rhs;
    }
}

impl Mul<IOUAmount> for i64 {
    type Output = IOUAmount;

    fn mul(self, rhs: IOUAmount) -> Self::Output {
        rhs * self
    }
}

impl Neg for IOUAmount {
    type Output = Self;

    fn neg(self) -> Self::Output {
        if self.is_zero() {
            self
        } else {
            Self {
                mantissa: -self.mantissa,
                exponent: self.exponent,
            }
        }
    }
}

impl PartialOrd for IOUAmount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IOUAmount {
    fn cmp(&self, other: &Self) -> Ordering {
        RuntimeNumber::from(*self).compare(RuntimeNumber::from(*other))
    }
}

impl fmt::Display for IOUAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        RuntimeNumber::from(*self).fmt(f)
    }
}

pub fn mul_ratio(
    amount: IOUAmount,
    num: u32,
    den: u32,
    round_up: bool,
) -> Result<IOUAmount, NumberArithmeticError> {
    if den == 0 {
        return Err(NumberArithmeticError::DivideByZero);
    }

    let negative = amount.mantissa() < 0;
    let denominator = u128::from(den);
    let multiplied = i128::from(amount.mantissa())
        .unsigned_abs()
        .checked_mul(u128::from(num))
        .ok_or(NumberArithmeticError::Overflow)?;

    let mut low = multiplied / denominator;
    let mut remainder = multiplied - low * denominator;
    let mut exponent = amount.exponent();

    if remainder != 0 {
        let room_to_grow = log10_floor_u128(NUMBER_MAX_REP as u128) - log10_ceil_u128(low);
        if room_to_grow > 0 {
            let scale = pow10_u128(room_to_grow as u32)?;
            exponent = exponent
                .checked_sub(room_to_grow)
                .ok_or(NumberArithmeticError::Overflow)?;
            low = low
                .checked_mul(scale)
                .ok_or(NumberArithmeticError::Overflow)?;
            remainder = remainder
                .checked_mul(scale)
                .ok_or(NumberArithmeticError::Overflow)?;
        }

        let add_remainder = remainder / denominator;
        low = low
            .checked_add(add_remainder)
            .ok_or(NumberArithmeticError::Overflow)?;
        remainder -= add_remainder * denominator;
    }

    let mut has_remainder = remainder != 0;
    let must_shrink = log10_ceil_u128(low) - log10_floor_u128(NUMBER_MAX_REP as u128);
    if must_shrink > 0 {
        let scale = pow10_u128(must_shrink as u32)?;
        let saved = low;
        exponent = exponent
            .checked_add(must_shrink)
            .ok_or(NumberArithmeticError::Overflow)?;
        low /= scale;
        if !has_remainder {
            has_remainder = saved - low * scale != 0;
        }
    }

    let mut mantissa = i64::try_from(low).map_err(|_| NumberArithmeticError::Overflow)?;
    if negative {
        mantissa = mantissa
            .checked_neg()
            .ok_or(NumberArithmeticError::Overflow)?;
    }

    let result = IOUAmount::from_parts(mantissa, exponent)?;
    if !has_remainder {
        return Ok(result);
    }

    if round_up && !negative {
        if result.is_zero() {
            return Ok(IOUAmount::min_positive_amount());
        }
        return IOUAmount::from_parts(
            result
                .mantissa()
                .checked_add(1)
                .ok_or(NumberArithmeticError::Overflow)?,
            result.exponent(),
        );
    }

    if !round_up && negative {
        if result.is_zero() {
            return IOUAmount::from_parts(-MIN_IOU_MANTISSA, MIN_IOU_EXPONENT);
        }
        return IOUAmount::from_parts(
            result
                .mantissa()
                .checked_sub(1)
                .ok_or(NumberArithmeticError::Overflow)?,
            result.exponent(),
        );
    }

    Ok(result)
}

fn runtime_number_from_external_parts(
    mantissa: i64,
    exponent: i32,
    scale: MantissaScale,
) -> Result<RuntimeNumber, NumberArithmeticError> {
    let range = basics::number::MantissaRange::new(scale);
    normalize_parts_to_range(
        mantissa < 0,
        external_to_internal_mantissa(mantissa),
        exponent,
        range.min,
        range.max,
    )
}

fn normalize_runtime_number_to_range(
    number: RuntimeNumber,
    min_mantissa: u64,
    max_mantissa: u64,
) -> Result<RuntimeNumber, NumberArithmeticError> {
    normalize_parts_to_range(
        number.negative,
        number.mantissa,
        number.exponent,
        min_mantissa,
        max_mantissa,
    )
}

fn normalize_parts_to_range(
    mut negative: bool,
    mut mantissa: u64,
    mut exponent: i32,
    min_mantissa: u64,
    max_mantissa: u64,
) -> Result<RuntimeNumber, NumberArithmeticError> {
    if mantissa == 0 {
        return Ok(RuntimeNumber::zero());
    }

    while mantissa < min_mantissa && exponent > NUMBER_MIN_EXPONENT {
        mantissa = mantissa
            .checked_mul(10)
            .ok_or(NumberArithmeticError::Overflow)?;
        exponent -= 1;
    }

    let mut guard = IouRoundGuard::default();
    if negative {
        guard.set_negative();
    }

    while mantissa > max_mantissa {
        if exponent >= NUMBER_MAX_EXPONENT {
            return Err(NumberArithmeticError::Overflow);
        }
        guard.push((mantissa % 10) as u8);
        mantissa /= 10;
        exponent += 1;
    }

    if exponent < NUMBER_MIN_EXPONENT || mantissa < min_mantissa {
        return Ok(RuntimeNumber::zero());
    }

    if mantissa > NUMBER_MAX_REP as u64 {
        if exponent >= NUMBER_MAX_EXPONENT {
            return Err(NumberArithmeticError::Overflow);
        }
        guard.push((mantissa % 10) as u8);
        mantissa /= 10;
        exponent += 1;
    }

    let mut widened = u128::from(mantissa);
    guard.do_round_up(
        &mut negative,
        &mut widened,
        &mut exponent,
        min_mantissa,
        max_mantissa,
    )?;

    Ok(RuntimeNumber::unchecked(
        negative && widened != 0,
        u64::try_from(widened).map_err(|_| NumberArithmeticError::Overflow)?,
        exponent,
    ))
}

fn signed_runtime_mantissa(number: RuntimeNumber) -> Result<i64, NumberArithmeticError> {
    let mantissa = i64::try_from(number.mantissa).map_err(|_| NumberArithmeticError::Overflow)?;
    if number.negative {
        mantissa
            .checked_neg()
            .ok_or(NumberArithmeticError::Overflow)
    } else {
        Ok(mantissa)
    }
}

fn pow10_u128(exponent: u32) -> Result<u128, NumberArithmeticError> {
    let mut value = 1u128;
    for _ in 0..exponent {
        value = value
            .checked_mul(10)
            .ok_or(NumberArithmeticError::Overflow)?;
    }
    Ok(value)
}

fn log10_floor_u128(mut value: u128) -> i32 {
    if value == 0 {
        return -1;
    }

    let mut log = 0i32;
    while value >= 10 {
        value /= 10;
        log += 1;
    }
    log
}

fn log10_ceil_u128(value: u128) -> i32 {
    if value == 0 {
        return 0;
    }

    let floor = log10_floor_u128(value);
    let power = pow10_u128(floor as u32).expect("log10 floor should stay within u128 pow10 range");
    if power == value { floor } else { floor + 1 }
}

#[derive(Debug, Clone, Copy, Default)]
struct IouRoundGuard {
    digits: u64,
    has_extra: bool,
    negative: bool,
}

impl IouRoundGuard {
    fn set_negative(&mut self) {
        self.negative = true;
    }

    fn push(&mut self, digit: u8) {
        self.has_extra = self.has_extra || ((self.digits & 0xF) != 0);
        self.digits >>= 4;
        self.digits |= u64::from(digit & 0x0F) << 60;
    }

    fn round(&self) -> i8 {
        match get_rounding_mode() {
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

    fn bring_into_range(
        &self,
        negative: &mut bool,
        mantissa: &mut u128,
        exponent: &mut i32,
        min_mantissa: u64,
    ) {
        if *mantissa < u128::from(min_mantissa) {
            *mantissa *= 10;
            *exponent -= 1;
        }

        if *exponent < NUMBER_MIN_EXPONENT {
            *negative = false;
            *mantissa = 0;
            *exponent = NUMBER_ZERO_EXPONENT;
        }
    }

    fn do_round_up(
        &self,
        negative: &mut bool,
        mantissa: &mut u128,
        exponent: &mut i32,
        min_mantissa: u64,
        max_mantissa: u64,
    ) -> Result<(), NumberArithmeticError> {
        let rounded = self.round();
        if rounded == 1 || (rounded == 0 && (*mantissa & 1) == 1) {
            *mantissa = mantissa
                .checked_add(1)
                .ok_or(NumberArithmeticError::Overflow)?;
            if *mantissa > u128::from(max_mantissa) || *mantissa > u128::from(NUMBER_MAX_REP as u64)
            {
                *mantissa /= 10;
                *exponent = exponent
                    .checked_add(1)
                    .ok_or(NumberArithmeticError::Overflow)?;
            }
        }

        self.bring_into_range(negative, mantissa, exponent, min_mantissa);
        if *exponent > NUMBER_MAX_EXPONENT {
            return Err(NumberArithmeticError::Overflow);
        }
        Ok(())
    }
}
