//! `MPTAmount` port from `xrpl/protocol/MPTAmount.h`.
#[cfg(test)]
mod tests {
    use super::MPTAmount;
    use crate::{CurrentTransactionRulesGuard, Rules, fix_cleanup_3_5_0};

    fn assert_overflow(operation: impl FnOnce()) {
        let payload = basics::contract::try_catch(operation).expect_err("overflow must unwind");
        assert!(basics::contract::exception_kind(payload.as_ref()).is_overflow());
    }

    #[test]
    fn cleanup_3_5_mpt_amount_overflow_throws_and_preserves_lhs() {
        let _rules = CurrentTransactionRulesGuard::new(Rules::new([fix_cleanup_3_5_0()]));

        let mut max = MPTAmount::from_value(i64::MAX);
        assert_overflow(|| max += MPTAmount::from_value(1));
        assert_eq!(max, MPTAmount::from_value(i64::MAX));

        let mut min = MPTAmount::from_value(i64::MIN);
        assert_overflow(|| min -= MPTAmount::from_value(1));
        assert_eq!(min, MPTAmount::from_value(i64::MIN));
    }

    #[test]
    fn mpt_amount_overflow_defaults_to_cleanup_behavior_without_rules() {
        let mut amount = MPTAmount::from_value(i64::MAX);
        assert_overflow(|| amount += MPTAmount::from_value(1));
        assert_eq!(amount, MPTAmount::from_value(i64::MAX));
    }
}

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use basics::number::{NumberArithmeticError, NumberParts};

pub const MAX_MP_TOKEN_AMOUNT: i64 = 0x7FFF_FFFF_FFFF_FFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MPTAmount {
    value: i64,
}

impl MPTAmount {
    pub const fn new() -> Self {
        Self::from_value(0)
    }

    pub const fn from_value(value: i64) -> Self {
        Self { value }
    }

    pub fn from_number(value: NumberParts) -> Result<Self, NumberArithmeticError> {
        Ok(Self::from_value(value.try_to_i64()?))
    }

    pub const fn signum(self) -> i32 {
        if self.value < 0 {
            -1
        } else if self.value == 0 {
            0
        } else {
            1
        }
    }

    pub const fn value(self) -> i64 {
        self.value
    }

    pub const fn is_zero(self) -> bool {
        self.value == 0
    }

    pub const fn min_positive_amount() -> Self {
        Self::from_value(1)
    }
}

impl From<i64> for MPTAmount {
    fn from(value: i64) -> Self {
        Self::from_value(value)
    }
}

impl TryFrom<NumberParts> for MPTAmount {
    type Error = NumberArithmeticError;

    fn try_from(value: NumberParts) -> Result<Self, Self::Error> {
        Self::from_number(value)
    }
}

impl From<MPTAmount> for NumberParts {
    fn from(value: MPTAmount) -> Self {
        NumberParts::from_i64(value.value)
    }
}

impl From<MPTAmount> for bool {
    fn from(value: MPTAmount) -> Self {
        !value.is_zero()
    }
}

// rippled #8453: fixCleanup3_5_0 changes MPTAmount overflow from legacy
// signed arithmetic to std::overflow_error. With no transaction rules (RPC
// and pathfinding), rippled applies the fixed behavior.
fn checked_or_legacy_overflow(result: Option<i64>, legacy: impl FnOnce() -> i64, operation: &str) -> i64 {
    match result {
        Some(value) => value,
        None if crate::is_feature_enabled_or(&crate::fix_cleanup_3_5_0(), true) => {
            basics::contract::throw_overflow(operation)
        }
        None => legacy(),
    }
}

impl AddAssign for MPTAmount {
    fn add_assign(&mut self, rhs: Self) {
        self.value = checked_or_legacy_overflow(
            self.value.checked_add(rhs.value),
            || self.value.wrapping_add(rhs.value),
            "MPTAmount::operator+= overflow",
        );
    }
}

impl Add for MPTAmount {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        self += rhs;
        self
    }
}

impl SubAssign for MPTAmount {
    fn sub_assign(&mut self, rhs: Self) {
        self.value = checked_or_legacy_overflow(
            self.value.checked_sub(rhs.value),
            || self.value.wrapping_sub(rhs.value),
            "MPTAmount::operator-= overflow",
        );
    }
}

impl Sub for MPTAmount {
    type Output = Self;

    fn sub(mut self, rhs: Self) -> Self::Output {
        self -= rhs;
        self
    }
}

impl Mul<i64> for MPTAmount {
    type Output = Self;

    fn mul(self, rhs: i64) -> Self::Output {
        Self::from_value(self.value.wrapping_mul(rhs))
    }
}

impl MulAssign<i64> for MPTAmount {
    fn mul_assign(&mut self, rhs: i64) {
        self.value = self.value.wrapping_mul(rhs);
    }
}

impl Div<i64> for MPTAmount {
    type Output = Self;

    fn div(self, rhs: i64) -> Self::Output {
        Self::from_value(self.value / rhs)
    }
}

impl DivAssign<i64> for MPTAmount {
    fn div_assign(&mut self, rhs: i64) {
        self.value /= rhs;
    }
}

impl Mul<MPTAmount> for i64 {
    type Output = MPTAmount;

    fn mul(self, rhs: MPTAmount) -> Self::Output {
        rhs * self
    }
}

impl Neg for MPTAmount {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::from_value(self.value.wrapping_neg())
    }
}

pub fn mul_ratio(
    amount: MPTAmount,
    num: u32,
    den: u32,
    round_up: bool,
) -> Result<MPTAmount, NumberArithmeticError> {
    if den == 0 {
        return Err(NumberArithmeticError::DivideByZero);
    }

    let amount128 = i128::from(amount.value());
    let negative = amount.value() < 0;
    let multiplied = amount128
        .checked_mul(i128::from(num))
        .ok_or(NumberArithmeticError::Overflow)?;
    let mut result = multiplied / i128::from(den);

    if multiplied % i128::from(den) != 0 {
        if !negative && round_up {
            result += 1;
        }
        if negative && !round_up {
            result -= 1;
        }
    }

    let result = i64::try_from(result).map_err(|_| NumberArithmeticError::Overflow)?;
    Ok(MPTAmount::from_value(result))
}
