//! Rust port of `xrpl/basics/base_uint.h`.

use crate::partitioned_unordered_map::PartitionKey;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::{
    Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not,
};
use std::slice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseHexError {
    BadLength,
    BadChar,
}

#[derive(Clone, Copy)]
pub struct BaseUInt<const BYTES: usize, Tag = ()> {
    bytes: [u8; BYTES],
    tag: PhantomData<Tag>,
}

impl<const BYTES: usize, Tag> BaseUInt<BYTES, Tag> {
    pub const BYTES: usize = BYTES;

    pub const fn zero() -> Self {
        Self {
            bytes: [0; BYTES],
            tag: PhantomData,
        }
    }

    pub const fn from_array(bytes: [u8; BYTES]) -> Self {
        Self {
            bytes,
            tag: PhantomData,
        }
    }

    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let array: [u8; BYTES] = bytes.try_into().ok()?;
        Some(Self::from_array(array))
    }

    pub fn from_void(bytes: &[u8; BYTES]) -> Self {
        Self::from_array(*bytes)
    }

    pub fn from_void_checked<T: AsRef<[u8]>>(from: T) -> Option<Self> {
        Self::from_slice(from.as_ref())
    }

    pub fn from_u64(value: u64) -> Self {
        let mut bytes = [0; BYTES];
        let source = value.to_be_bytes();
        let offset = BYTES.saturating_sub(source.len());
        bytes[offset..].copy_from_slice(&source[source.len().saturating_sub(BYTES)..]);
        Self::from_array(bytes)
    }

    pub fn from_hex(input: &str) -> Result<Self, ParseHexError> {
        if input == "0" {
            return Ok(Self::zero());
        }
        if input.len() != BYTES * 2 {
            return Err(ParseHexError::BadLength);
        }

        let mut bytes = [0; BYTES];
        for (index, chunk) in input.as_bytes().chunks_exact(2).enumerate() {
            let high = decode_hex_nibble(chunk[0])?;
            let low = decode_hex_nibble(chunk[1])?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self::from_array(bytes))
    }

    pub fn parse_hex(&mut self, input: &str) -> bool {
        match Self::from_hex(input) {
            Ok(parsed) => {
                *self = parsed;
                true
            }
            Err(_) => false,
        }
    }

    pub const fn size() -> usize {
        BYTES
    }

    pub fn data(&self) -> &[u8; BYTES] {
        &self.bytes
    }

    pub fn data_mut(&mut self) -> &mut [u8; BYTES] {
        &mut self.bytes
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    pub fn iter(&self) -> std::slice::Iter<'_, u8> {
        self.bytes.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, u8> {
        self.bytes.iter_mut()
    }

    pub fn signum(&self) -> i32 {
        if self.bytes.iter().any(|byte| *byte != 0) {
            1
        } else {
            0
        }
    }

    pub fn is_zero(&self) -> bool {
        self.signum() == 0
    }

    pub fn is_non_zero(&self) -> bool {
        !self.is_zero()
    }

    pub fn increment(&mut self) -> &mut Self {
        for byte in self.bytes.iter_mut().rev() {
            let (next, carry) = byte.overflowing_add(1);
            *byte = next;
            if !carry {
                break;
            }
        }
        self
    }

    pub fn decrement(&mut self) -> &mut Self {
        for byte in self.bytes.iter_mut().rev() {
            let previous = *byte;
            *byte = byte.wrapping_sub(1);
            if previous != 0 {
                break;
            }
        }
        self
    }

    pub fn next(self) -> Self {
        let mut result = self;
        result.increment();
        result
    }

    pub fn prev(self) -> Self {
        let mut result = self;
        result.decrement();
        result
    }

    #[allow(non_snake_case)]
    pub fn isZero(&self) -> bool {
        self.is_zero()
    }

    #[allow(non_snake_case)]
    pub fn isNonZero(&self) -> bool {
        self.is_non_zero()
    }

    pub fn assign_from<T: AsRef<[u8]>>(&mut self, from: T) -> bool {
        if let Some(value) = Self::from_void_checked(from) {
            *self = value;
            true
        } else {
            false
        }
    }
}

impl<const BYTES: usize, Tag> Default for BaseUInt<BYTES, Tag> {
    fn default() -> Self {
        Self::zero()
    }
}

impl<const BYTES: usize, Tag> From<u64> for BaseUInt<BYTES, Tag> {
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

impl<const BYTES: usize, Tag> From<[u8; BYTES]> for BaseUInt<BYTES, Tag> {
    fn from(bytes: [u8; BYTES]) -> Self {
        Self::from_array(bytes)
    }
}

impl<const BYTES: usize, Tag> TryFrom<&[u8]> for BaseUInt<BYTES, Tag> {
    type Error = ();

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::from_slice(value).ok_or(())
    }
}

impl<const BYTES: usize, Tag> PartialEq<u64> for BaseUInt<BYTES, Tag> {
    fn eq(&self, other: &u64) -> bool {
        *self == Self::from_u64(*other)
    }
}

impl<const BYTES: usize, Tag> Eq for BaseUInt<BYTES, Tag> {}

impl<const BYTES: usize, Tag> PartialEq for BaseUInt<BYTES, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl<const BYTES: usize, Tag> PartialOrd for BaseUInt<BYTES, Tag> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<const BYTES: usize, Tag> Ord for BaseUInt<BYTES, Tag> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.bytes.cmp(&other.bytes)
    }
}

impl<const BYTES: usize, Tag> Hash for BaseUInt<BYTES, Tag> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(&self.bytes);
    }
}

impl<const BYTES: usize, Tag> Not for BaseUInt<BYTES, Tag> {
    type Output = Self;

    fn not(self) -> Self::Output {
        let mut bytes = self.bytes;
        for byte in &mut bytes {
            *byte = !*byte;
        }
        Self::from_array(bytes)
    }
}

impl<const BYTES: usize, Tag> BitXorAssign for BaseUInt<BYTES, Tag> {
    fn bitxor_assign(&mut self, rhs: Self) {
        for (left, right) in self.bytes.iter_mut().zip(rhs.bytes) {
            *left ^= right;
        }
    }
}

impl<const BYTES: usize, Tag> BitXor for BaseUInt<BYTES, Tag> {
    type Output = Self;

    fn bitxor(mut self, rhs: Self) -> Self::Output {
        self ^= rhs;
        self
    }
}

impl<const BYTES: usize, Tag> BitAndAssign for BaseUInt<BYTES, Tag> {
    fn bitand_assign(&mut self, rhs: Self) {
        for (left, right) in self.bytes.iter_mut().zip(rhs.bytes) {
            *left &= right;
        }
    }
}

impl<const BYTES: usize, Tag> BitAnd for BaseUInt<BYTES, Tag> {
    type Output = Self;

    fn bitand(mut self, rhs: Self) -> Self::Output {
        self &= rhs;
        self
    }
}

impl<const BYTES: usize, Tag> BitOrAssign for BaseUInt<BYTES, Tag> {
    fn bitor_assign(&mut self, rhs: Self) {
        for (left, right) in self.bytes.iter_mut().zip(rhs.bytes) {
            *left |= right;
        }
    }
}

impl<const BYTES: usize, Tag> BitOr for BaseUInt<BYTES, Tag> {
    type Output = Self;

    fn bitor(mut self, rhs: Self) -> Self::Output {
        self |= rhs;
        self
    }
}

impl<const BYTES: usize, Tag> AddAssign for BaseUInt<BYTES, Tag> {
    fn add_assign(&mut self, rhs: Self) {
        let mut carry = 0u16;
        for (left, right) in self.bytes.iter_mut().rev().zip(rhs.bytes.iter().rev()) {
            let sum = *left as u16 + *right as u16 + carry;
            *left = sum as u8;
            carry = sum >> 8;
        }
    }
}

impl<const BYTES: usize, Tag> Add for BaseUInt<BYTES, Tag> {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        self += rhs;
        self
    }
}

impl<const BYTES: usize, Tag> fmt::Display for BaseUInt<BYTES, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&to_string(self))
    }
}

impl<const BYTES: usize, Tag> fmt::Debug for BaseUInt<BYTES, Tag> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BaseUInt")
            .field(&to_string(self))
            .finish()
    }
}

impl<const BYTES: usize, Tag> AsRef<[u8]> for BaseUInt<BYTES, Tag> {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<const BYTES: usize, Tag> AsMut<[u8]> for BaseUInt<BYTES, Tag> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}

impl<'a, const BYTES: usize, Tag> IntoIterator for &'a BaseUInt<BYTES, Tag> {
    type Item = &'a u8;
    type IntoIter = slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, const BYTES: usize, Tag> IntoIterator for &'a mut BaseUInt<BYTES, Tag> {
    type Item = &'a mut u8;
    type IntoIter = slice::IterMut<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<const BYTES: usize, Tag> IntoIterator for BaseUInt<BYTES, Tag> {
    type Item = u8;
    type IntoIter = std::array::IntoIter<u8, BYTES>;

    fn into_iter(self) -> Self::IntoIter {
        self.bytes.into_iter()
    }
}

pub type Uint128 = BaseUInt<16>;
pub type Uint160 = BaseUInt<20>;
pub type Uint192 = BaseUInt<24>;
pub type Uint256 = BaseUInt<32>;

pub fn to_string<const BYTES: usize, Tag>(value: &BaseUInt<BYTES, Tag>) -> String {
    value
        .bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

pub fn to_short_string<const BYTES: usize, Tag>(value: &BaseUInt<BYTES, Tag>) -> String {
    assert!(BYTES > 4, "For 4 bytes or less, use a native type");
    value
        .bytes
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>()
        + "..."
}

impl PartitionKey for Uint256 {
    fn partition_key(&self) -> usize {
        let mut bytes = [0u8; std::mem::size_of::<usize>()];
        bytes.copy_from_slice(&self.data()[..std::mem::size_of::<usize>()]);
        usize::from_ne_bytes(bytes)
    }
}

fn decode_hex_nibble(input: u8) -> Result<u8, ParseHexError> {
    match input {
        b'0'..=b'9' => Ok(input - b'0'),
        b'a'..=b'f' => Ok(input - b'a' + 10),
        b'A'..=b'F' => Ok(input - b'A' + 10),
        _ => Err(ParseHexError::BadChar),
    }
}

#[cfg(test)]
mod tests {
    use super::{BaseUInt, ParseHexError, Uint256, to_short_string, to_string};
    use crate::partitioned_unordered_map::PartitionKey;
    use std::collections::HashSet;
    use std::hash::{Hash, Hasher};

    type Test96 = BaseUInt<12>;

    struct CaptureHasher {
        bytes: Vec<u8>,
    }

    impl Hasher for CaptureHasher {
        fn finish(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn write(&mut self, bytes: &[u8]) {
            self.bytes.extend_from_slice(bytes);
        }
    }

    #[test]
    fn comparisons_match_cpp_examples() {
        let args = [
            ("0000000000000000", "0000000000000001"),
            ("0000000000000000", "FFFFFFFFFFFFFFFF"),
            ("1234567812345678", "2345678923456789"),
            ("8000000000000000", "8000000000000001"),
            ("AAAAAAAAAAAAAAA9", "AAAAAAAAAAAAAAAA"),
            ("FFFFFFFFFFFFFFFE", "FFFFFFFFFFFFFFFF"),
        ];

        for (left, right) in args {
            let left = BaseUInt::<8>::from_hex(left).expect("hex should parse");
            let right = BaseUInt::<8>::from_hex(right).expect("hex should parse");

            assert!(left < right);
            assert!(right > left);
            assert_eq!(left, left);
            assert_eq!(right, right);
        }
    }

    #[test]
    fn general_purpose_behavior_role() {
        let raw = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let u = Test96::from_array(raw);

        let mut set = HashSet::new();
        set.insert(u);

        assert_eq!(Test96::BYTES, raw.len());
        assert_eq!(to_string(&u), "0102030405060708090A0B0C");
        assert_eq!(to_short_string(&u), "01020304...");
        assert_eq!(u.data()[0], 1);
        assert_eq!(u.signum(), 1);
        assert!(u.is_non_zero());
        assert!(!u.is_zero());
        assert_eq!(u.iter().copied().collect::<Vec<_>>(), raw);
        assert_eq!((&u).into_iter().copied().collect::<Vec<_>>(), raw);

        let mut hasher = CaptureHasher { bytes: Vec::new() };
        u.hash(&mut hasher);
        assert_eq!(hasher.bytes, raw);

        let v = !u;
        set.insert(v);
        assert_eq!(to_string(&v), "FEFDFCFBFAF9F8F7F6F5F4F3");
        assert_eq!(to_short_string(&v), "FEFDFCFB...");

        let z = Test96::zero();
        set.insert(z);
        assert_eq!(to_string(&z), "000000000000000000000000");
        assert_eq!(z.signum(), 0);
        assert!(z.is_zero());
        assert!(!z.is_non_zero());

        let mut n = z;
        n.increment();
        assert_eq!(n, Test96::from_u64(1));
        n.decrement();
        assert_eq!(n, z);
        n.decrement();
        assert_eq!(to_string(&n), "FFFFFFFFFFFFFFFFFFFFFFFF");

        let x = (z.prev()) ^ (z.next());
        set.insert(x);
        assert_eq!(to_string(&x), "FFFFFFFFFFFFFFFFFFFFFFFE");
        assert_eq!(set.len(), 4);
    }

    #[test]
    fn hex_parsing_role() {
        let u = Test96::from_hex("0102030405060708090A0B0C").expect("hex should parse");
        let mut tmp = Test96::zero();
        assert!(tmp.parse_hex(&to_string(&u)));
        assert_eq!(tmp, u);

        assert_eq!(Test96::from_hex("0"), Ok(Test96::zero()));
        assert_eq!(
            Test96::from_hex("A0102030405060708090A0B0C"),
            Err(ParseHexError::BadLength)
        );
        assert_eq!(
            Test96::from_hex("0102030405060708090A0B0CA"),
            Err(ParseHexError::BadLength)
        );
        assert_eq!(
            Test96::from_hex("0102030405060708090A0B0G"),
            Err(ParseHexError::BadChar)
        );
    }

    #[test]
    fn raw_buffer_and_contiguous_container_helpers_match_cpp_role() {
        let raw = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let mut value = Test96::from_void(&raw);
        assert_eq!(value.as_slice(), raw);
        assert_eq!(Test96::from_void_checked(raw), Some(value));
        assert_eq!(Test96::from_void_checked(raw.as_slice()), Some(value));
        assert_eq!(Test96::from_void_checked(&raw[..11]), None);

        value
            .iter_mut()
            .for_each(|byte| *byte = byte.wrapping_add(1));
        assert_eq!(value.as_slice(), &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);

        let mut assigned = Test96::zero();
        assert!(assigned.assign_from(raw));
        assert_eq!(assigned, Test96::from_array(raw));
        assert!(!assigned.assign_from(&raw[..11]));

        assert!(assigned.isZero() == assigned.is_zero());
        assert!(value.isNonZero());
    }

    #[test]
    fn uint256_partition_key_uses_leading_bytes_extract() {
        let value =
            Uint256::from_hex("0102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F20")
                .expect("hex should parse");

        let mut bytes = [0u8; std::mem::size_of::<usize>()];
        bytes.copy_from_slice(&value.data()[..std::mem::size_of::<usize>()]);
        assert_eq!(value.partition_key(), usize::from_ne_bytes(bytes));
    }
}
