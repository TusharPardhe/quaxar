//! Narrow unit wrappers from `xrpl/protocol/Units.h`.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, RemAssign, Sub, SubAssign};

use crate::JsonValue;

pub mod unit {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct DropTag;
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct FeeLevelTag;
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct UnitlessTag;
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct BipsTag;
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct TenthBipsTag;
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ValueUnit<UnitTag, T> {
    value: T,
    _tag: PhantomData<UnitTag>,
}

impl<UnitTag, T> ValueUnit<UnitTag, T> {
    pub const fn new(value: T) -> Self {
        Self {
            value,
            _tag: PhantomData,
        }
    }

    pub const fn value(self) -> T
    where
        T: Copy,
    {
        self.value
    }
}

impl<UnitTag, T> ValueUnit<UnitTag, T>
where
    T: Copy + Default + PartialOrd + From<i8>,
{
    pub fn signum(self) -> i32 {
        let zero = T::from(0);
        if self.value < zero {
            -1
        } else if self.value > zero {
            1
        } else {
            0
        }
    }
}

impl<UnitTag, T> ValueUnit<UnitTag, T>
where
    T: Copy + Into<i128>,
{
    pub fn json_clipped(self) -> JsonValue {
        let value = self.value.into();
        if value < i128::from(i32::MIN) {
            JsonValue::Signed(i64::from(i32::MIN))
        } else if value > i128::from(u32::MAX) {
            JsonValue::Unsigned(u64::from(u32::MAX))
        } else if value < 0 {
            JsonValue::Signed(value as i64)
        } else {
            JsonValue::Unsigned(value as u64)
        }
    }
}

impl<UnitTag, T> fmt::Display for ValueUnit<UnitTag, T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

macro_rules! impl_value_unit_ops {
    ($trait:ident, $method:ident) => {
        impl<UnitTag, T> $trait for ValueUnit<UnitTag, T>
        where
            T: $trait<Output = T> + Copy,
        {
            type Output = Self;

            fn $method(self, rhs: Self) -> Self::Output {
                Self::new(self.value.$method(rhs.value))
            }
        }
    };
}

macro_rules! impl_value_unit_assign_ops {
    ($trait:ident, $method:ident) => {
        impl<UnitTag, T> $trait for ValueUnit<UnitTag, T>
        where
            T: $trait<T> + Copy,
        {
            fn $method(&mut self, rhs: Self) {
                self.value.$method(rhs.value);
            }
        }
    };
}

impl_value_unit_ops!(Add, add);
impl_value_unit_ops!(Sub, sub);
impl_value_unit_assign_ops!(AddAssign, add_assign);
impl_value_unit_assign_ops!(SubAssign, sub_assign);

impl<UnitTag, T> Mul<T> for ValueUnit<UnitTag, T>
where
    T: Mul<Output = T> + Copy,
{
    type Output = Self;

    fn mul(self, rhs: T) -> Self::Output {
        Self::new(self.value * rhs)
    }
}

impl<UnitTag, T> MulAssign<T> for ValueUnit<UnitTag, T>
where
    T: MulAssign + Copy,
{
    fn mul_assign(&mut self, rhs: T) {
        self.value *= rhs;
    }
}

impl<UnitTag, T> Div<T> for ValueUnit<UnitTag, T>
where
    T: Div<Output = T> + Copy,
{
    type Output = Self;

    fn div(self, rhs: T) -> Self::Output {
        Self::new(self.value / rhs)
    }
}

impl<UnitTag, T> DivAssign<T> for ValueUnit<UnitTag, T>
where
    T: DivAssign + Copy,
{
    fn div_assign(&mut self, rhs: T) {
        self.value /= rhs;
    }
}

impl<UnitTag, T> RemAssign<T> for ValueUnit<UnitTag, T>
where
    T: RemAssign + Copy,
{
    fn rem_assign(&mut self, rhs: T) {
        self.value %= rhs;
    }
}

impl<UnitTag, T> Neg for ValueUnit<UnitTag, T>
where
    T: Neg<Output = T> + Copy,
{
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.value)
    }
}

pub type FeeLevel<T> = ValueUnit<unit::FeeLevelTag, T>;
pub type FeeLevel64 = FeeLevel<u64>;
pub type FeeLevelDouble = FeeLevel<f64>;
pub type Bips<T> = ValueUnit<unit::BipsTag, T>;
pub type Bips16 = Bips<u16>;
pub type Bips32 = Bips<u32>;
pub type TenthBips<T> = ValueUnit<unit::TenthBipsTag, T>;
pub type TenthBips16 = TenthBips<u16>;
pub type TenthBips32 = TenthBips<u32>;

pub const fn scalar<T>(value: T) -> ValueUnit<unit::UnitlessTag, T> {
    ValueUnit::new(value)
}
