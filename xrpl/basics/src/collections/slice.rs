//! Rust model of the reference `xrpl::Slice` type.
//!
//! Teaching note:
//! The reference `Slice` type is a lightweight non-owning view over bytes.
//! In Rust, the closest built-in concept is `&[u8]`, which is already an
//! immutable borrowed view into bytes.
//!
//! We still wrap it in a struct here because:
//! - it mirrors the reference migration target more directly,
//! - it gives us a place to expose compatibility-friendly methods,
//! - it teaches how lifetimes work on borrowed data.
//!
//! `Slice<'a>` means:
//! - "this value borrows some bytes"
//! - "those bytes must live at least as long as `'a`"

use crate::contract::throw;
use std::cmp::min;
use std::error::Error;
use std::fmt;

/// Immutable borrowed byte range, modeled after reference `xrpl::Slice`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Slice<'a> {
    data: Option<&'a [u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceAdvanceError;

impl fmt::Display for SliceAdvanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("too small")
    }
}

impl Error for SliceAdvanceError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceSubsliceError;

impl fmt::Display for SliceSubsliceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Requested sub-slice is out of bounds")
    }
}

impl Error for SliceSubsliceError {}

impl<'a> Slice<'a> {
    /// Create a new borrowed byte slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data: Some(data) }
    }

    /// Return `true` when no bytes are present.
    pub fn empty(&self) -> bool {
        self.size() == 0
    }

    /// Number of bytes in the slice.
    pub fn size(&self) -> usize {
        self.data.map_or(0, <[u8]>::len)
    }

    /// Alias for `size`, matching the reference API shape.
    pub fn length(&self) -> usize {
        self.size()
    }

    /// Access the raw bytes.
    pub fn data(&self) -> &'a [u8] {
        self.data.unwrap_or(&[])
    }

    /// Access the underlying raw pointer.
    pub fn as_ptr(&self) -> *const u8 {
        self.data.map_or(std::ptr::null(), <[u8]>::as_ptr)
    }

    /// Return the byte at the given index.
    pub fn at(&self, index: usize) -> u8 {
        self.data()[index]
    }

    /// Advance the front of the slice by `count` bytes.
    ///
    /// This mirrors the semantics of reference `operator+=`.
    pub fn advance(&mut self, count: usize) {
        if count > self.size() {
            throw(SliceAdvanceError);
        }

        match self.data {
            Some(data) if count > 0 => self.data = Some(&data[count..]),
            _ => {}
        }
    }

    /// Return a new advanced view without mutating the original.
    pub fn advanced(self, count: usize) -> Self {
        let mut next = self;
        next.advance(count);
        next
    }

    /// Remove bytes from the front.
    pub fn remove_prefix(&mut self, count: usize) {
        match self.data {
            Some(data) if count > 0 => self.data = Some(&data[count..]),
            _ => {}
        }
    }

    /// Remove bytes from the back.
    pub fn remove_suffix(&mut self, count: usize) {
        let data = self.data();
        if count == 0 && self.data.is_none() {
            return;
        }
        self.data = Some(&data[..data.len() - count]);
    }

    /// Return a sub-slice starting at `pos` with up to `count` bytes.
    pub fn substr(&self, pos: usize, count: usize) -> Self {
        let data = self.data();
        if pos > data.len() {
            throw(SliceSubsliceError);
        }
        if self.data.is_none() && pos == 0 {
            return Self::default();
        }
        Self::new(&data[pos..pos + min(count, data.len() - pos)])
    }

    /// Iterator over bytes.
    pub fn iter(&self) -> std::slice::Iter<'a, u8> {
        self.data().iter()
    }
}

impl<'a> PartialEq for Slice<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.data() == other.data()
    }
}

impl<'a> Eq for Slice<'a> {}

impl<'a> PartialOrd for Slice<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> Ord for Slice<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.data().cmp(other.data())
    }
}

impl<'a> From<&'a [u8]> for Slice<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::new(value)
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for Slice<'a> {
    fn from(value: &'a [u8; N]) -> Self {
        Self::new(value)
    }
}

impl<'a, const N: usize> From<&'a [char; N]> for Slice<'a> {
    fn from(_value: &'a [char; N]) -> Self {
        panic!("char arrays are not supported for byte Slice conversion")
    }
}

impl<'a> From<&'a Vec<u8>> for Slice<'a> {
    fn from(value: &'a Vec<u8>) -> Self {
        Self::new(value.as_slice())
    }
}

impl<'a> From<&'a str> for Slice<'a> {
    fn from(value: &'a str) -> Self {
        Self::new(value.as_bytes())
    }
}

/// Helper mirroring the reference `makeSlice` family.
pub fn make_slice<'a, T>(value: T) -> Slice<'a>
where
    T: Into<Slice<'a>>,
{
    value.into()
}

#[cfg(test)]
mod tests {
    use super::{Slice, make_slice};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    const DATA: [u8; 32] = [
        0xa8, 0xa1, 0x38, 0x45, 0x23, 0xec, 0xe4, 0x23, 0x71, 0x6d, 0x2a, 0x18, 0xb4, 0x70, 0xcb,
        0xf5, 0xac, 0x2d, 0x89, 0x4d, 0x19, 0x9c, 0xf0, 0x2c, 0x15, 0xd1, 0xf9, 0x9b, 0x66, 0xd2,
        0x30, 0xd3,
    ];

    #[test]
    fn equality_and_inequality_match_cpp_behavior() {
        let s0 = Slice::default();

        assert_eq!(s0.size(), 0);
        assert_eq!(s0.data(), &[]);
        assert!(s0.as_ptr().is_null());
        assert_eq!(s0, s0);

        for i in 0..DATA.len() {
            let s1 = Slice::new(&DATA[..i]);

            assert_eq!(s1.size(), i);

            if i == 0 {
                assert_eq!(s1, s0);
            } else {
                assert_ne!(s1, s0);
            }

            for j in 0..DATA.len() {
                let s2 = Slice::new(&DATA[..j]);

                if i == j {
                    assert_eq!(s1, s2);
                } else {
                    assert_ne!(s1, s2);
                }
            }
        }

        let mut a = DATA;
        let mut b = DATA;

        assert_eq!(make_slice(&a), make_slice(&b));
        b[7] = b[7].wrapping_add(1);
        assert_ne!(make_slice(&a), make_slice(&b));
        a[7] = a[7].wrapping_add(1);
        assert_eq!(make_slice(&a), make_slice(&b));
    }

    #[test]
    fn indexing_behavior() {
        let s = Slice::new(&DATA);

        for (i, byte) in DATA.iter().enumerate() {
            assert_eq!(s.at(i), *byte);
        }
    }

    #[test]
    fn advancing_behavior() {
        for i in 0..DATA.len() {
            for j in 0..(DATA.len() - i) {
                let mut s = Slice::new(&DATA[i..]);
                s.advance(j);

                assert_eq!(s.data(), &DATA[i + j..]);
                assert_eq!(s.size(), DATA.len() - i - j);
            }
        }
    }

    #[test]
    fn remove_prefix_and_suffix_follow_cpp_unchecked_shape() {
        let mut slice = Slice::new(&DATA[..8]);
        slice.remove_prefix(3);
        assert_eq!(slice.data(), &DATA[3..8]);

        slice.remove_suffix(2);
        assert_eq!(slice.data(), &DATA[3..6]);
    }

    #[test]
    fn invalid_advance_panics_with_typed_error() {
        let mut slice = Slice::new(&DATA[..4]);

        let payload = catch_unwind(AssertUnwindSafe(|| slice.advance(5)))
            .expect_err("advance past end should unwind");
        let error = payload
            .downcast::<super::SliceAdvanceError>()
            .expect("expected SliceAdvanceError");
        assert_eq!(error.to_string(), "too small");
    }

    #[test]
    fn invalid_substr_panics_with_typed_error() {
        let slice = Slice::new(&DATA[..4]);

        let payload = catch_unwind(AssertUnwindSafe(|| slice.substr(5, 1)))
            .expect_err("substr past end should unwind");
        let error = payload
            .downcast::<super::SliceSubsliceError>()
            .expect("expected SliceSubsliceError");
        assert_eq!(error.to_string(), "Requested sub-slice is out of bounds");
    }
}
