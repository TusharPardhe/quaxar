//! Rust equivalent of `xrpl/basics/tagged_integer.h`.
//!
//! This is a strong "Rust for JS/TS engineers" example because it shows how we
//! can create a type-safe wrapper around a primitive integer without runtime
//! overhead. In TypeScript this is similar in spirit to branded types. In Rust,
//! the compiler can enforce the distinction directly.

use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::{
    Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Div, DivAssign,
    Mul, MulAssign, Neg, Not, Rem, RemAssign, Shl, ShlAssign, Shr, ShrAssign, Sub, SubAssign,
};
use std::str::FromStr;

/// Marker trait restricting the wrapper to built-in integer primitives.
pub trait TaggedIntegerPrimitive:
    Copy + Clone + Default + Eq + Ord + Hash + fmt::Debug + fmt::Display + 'static
{
}

macro_rules! impl_tagged_integer_primitive {
    ($($ty:ty),* $(,)?) => {
        $(impl TaggedIntegerPrimitive for $ty {})*
    };
}

impl_tagged_integer_primitive!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

/// A zero-cost tagged integer.
///
/// `Tag` is never stored at runtime. We only keep `PhantomData<Tag>` so the
/// compiler treats `TaggedInteger<u32, A>` and `TaggedInteger<u32, B>` as
/// different types.
#[repr(transparent)]
#[derive(Copy, Clone, Default)]
pub struct TaggedInteger<Int, Tag> {
    value: Int,
    _tag: PhantomData<Tag>,
}

impl<Int, Tag> TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive,
{
    pub const fn new(value: Int) -> Self {
        Self {
            value,
            _tag: PhantomData,
        }
    }

    pub const fn value(self) -> Int {
        self.value
    }
}

impl<Int, Tag, Other> From<Other> for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive + From<Other>,
    Other: TaggedIntegerPrimitive,
{
    fn from(value: Other) -> Self {
        Self::new(Int::from(value))
    }
}

impl<Int, Tag> fmt::Display for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.value, formatter)
    }
}

impl<Int, Tag> fmt::Debug for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TaggedInteger")
            .field(&self.value)
            .finish()
    }
}

impl<Int, Tag> PartialEq for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive,
{
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<Int, Tag> Eq for TaggedInteger<Int, Tag> where Int: TaggedIntegerPrimitive {}

impl<Int, Tag> PartialOrd for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<Int, Tag> Ord for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<Int, Tag> std::hash::Hash for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<Int, Tag> FromStr for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive + FromStr,
{
    type Err = Int::Err;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        source.parse::<Int>().map(Self::new)
    }
}

macro_rules! impl_binary_operator {
    ($trait:ident, $method:ident) => {
        impl<Int, Tag> $trait for TaggedInteger<Int, Tag>
        where
            Int: TaggedIntegerPrimitive + $trait<Output = Int>,
        {
            type Output = Self;

            fn $method(self, rhs: Self) -> Self::Output {
                Self::new(self.value.$method(rhs.value))
            }
        }
    };
}

macro_rules! impl_assignment_operator {
    ($trait:ident, $method:ident) => {
        impl<Int, Tag> $trait for TaggedInteger<Int, Tag>
        where
            Int: TaggedIntegerPrimitive + $trait<Int>,
        {
            fn $method(&mut self, rhs: Self) {
                self.value.$method(rhs.value);
            }
        }
    };
}

impl_binary_operator!(Add, add);
impl_binary_operator!(Sub, sub);
impl_binary_operator!(Mul, mul);
impl_binary_operator!(Div, div);
impl_binary_operator!(Rem, rem);
impl_binary_operator!(BitOr, bitor);
impl_binary_operator!(BitAnd, bitand);
impl_binary_operator!(BitXor, bitxor);
impl_binary_operator!(Shl, shl);
impl_binary_operator!(Shr, shr);

impl_assignment_operator!(AddAssign, add_assign);
impl_assignment_operator!(SubAssign, sub_assign);
impl_assignment_operator!(MulAssign, mul_assign);
impl_assignment_operator!(DivAssign, div_assign);
impl_assignment_operator!(RemAssign, rem_assign);
impl_assignment_operator!(BitOrAssign, bitor_assign);
impl_assignment_operator!(BitAndAssign, bitand_assign);
impl_assignment_operator!(BitXorAssign, bitxor_assign);
impl_assignment_operator!(ShlAssign, shl_assign);
impl_assignment_operator!(ShrAssign, shr_assign);

impl<Int, Tag> Not for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive + Not<Output = Int>,
{
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::new(!self.value)
    }
}

impl<Int, Tag> Neg for TaggedInteger<Int, Tag>
where
    Int: TaggedIntegerPrimitive + Neg<Output = Int>,
{
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::TaggedInteger;
    use std::mem::size_of;

    #[derive(Debug)]
    struct Tag1;
    #[derive(Debug)]
    struct Tag2;

    type TagInt = TaggedInteger<i32, Tag1>;
    type TagUInt1 = TaggedInteger<u32, Tag1>;
    type TagUInt2 = TaggedInteger<u32, Tag2>;
    type TagUInt3 = TaggedInteger<u64, Tag1>;

    #[test]
    fn tagged_type_has_same_size_as_underlying_integer() {
        assert_eq!(size_of::<TagUInt1>(), size_of::<u32>());
        assert_eq!(size_of::<TagUInt3>(), size_of::<u64>());
    }

    #[test]
    fn construction_is_explicit_and_type_safe() {
        let same_width = TagUInt1::from(5u32);
        let widened = TagUInt3::from(5u32);

        assert_eq!(same_width.value(), 5u32);
        assert_eq!(widened.value(), 5u64);

        let a = TagUInt1::from(7u32);
        let b = TagUInt2::from(7u32);

        assert_eq!(a.value(), b.value());
    }

    #[test]
    fn comparison_operators_match_cpp_behavior() {
        let zero = TagInt::new(0);
        let one = TagInt::new(1);

        assert_eq!(one, one);
        assert_ne!(one, zero);
        assert!(zero < one);
        assert!(one > zero);
        assert!(one >= one);
        assert!(one >= zero);
        assert!(zero <= one);
        assert!(zero <= zero);
    }

    #[test]
    fn arithmetic_and_bitwise_operators_match_cpp_behavior() {
        let a = TagInt::new(-2);
        assert_eq!(a, TagInt::new(-2));
        assert_eq!(-a, TagInt::new(2));
        assert_eq!(TagInt::new(-3) + TagInt::new(4), TagInt::new(1));
        assert_eq!(TagInt::new(-3) - TagInt::new(4), TagInt::new(-7));
        assert_eq!(TagInt::new(-3) * TagInt::new(4), TagInt::new(-12));
        assert_eq!(TagInt::new(8) / TagInt::new(4), TagInt::new(2));
        assert_eq!(TagInt::new(7) % TagInt::new(4), TagInt::new(3));

        assert_eq!(!TagInt::new(8), TagInt::new(!8));
        assert_eq!(TagInt::new(6) & TagInt::new(3), TagInt::new(2));
        assert_eq!(TagInt::new(6) | TagInt::new(3), TagInt::new(7));
        assert_eq!(TagInt::new(6) ^ TagInt::new(3), TagInt::new(5));
        assert_eq!(TagInt::new(4) << TagInt::new(2), TagInt::new(16));
        assert_eq!(TagInt::new(16) >> TagInt::new(2), TagInt::new(4));
    }

    #[test]
    fn assignment_operators_match_cpp_behavior() {
        let mut a = TagInt::new(-2);
        let b = a;
        assert_eq!(b, TagInt::new(-2));

        a = TagInt::new(-3);
        a += TagInt::new(4);
        assert_eq!(a, TagInt::new(1));

        a = TagInt::new(-3);
        a -= TagInt::new(4);
        assert_eq!(a, TagInt::new(-7));

        a = TagInt::new(-3);
        a *= TagInt::new(4);
        assert_eq!(a, TagInt::new(-12));

        a = TagInt::new(8);
        a /= TagInt::new(4);
        assert_eq!(a, TagInt::new(2));

        a = TagInt::new(7);
        a %= TagInt::new(4);
        assert_eq!(a, TagInt::new(3));

        a = TagInt::new(6);
        a &= TagInt::new(3);
        assert_eq!(a, TagInt::new(2));

        a = TagInt::new(6);
        a |= TagInt::new(3);
        assert_eq!(a, TagInt::new(7));

        a = TagInt::new(6);
        a ^= TagInt::new(3);
        assert_eq!(a, TagInt::new(5));

        a = TagInt::new(4);
        a <<= TagInt::new(2);
        assert_eq!(a, TagInt::new(16));

        a = TagInt::new(16);
        a >>= TagInt::new(2);
        assert_eq!(a, TagInt::new(4));
    }

    #[test]
    fn display_and_parse_round_trip() {
        let value = TagInt::new(42);
        assert_eq!(value.to_string(), "42");
        assert_eq!("42".parse::<TagInt>().unwrap(), value);
    }
}
