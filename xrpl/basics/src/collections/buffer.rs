//! Rust port of `xrpl/basics/Buffer.h`.
//!
//! The reference type owns a byte buffer and allows reallocation that discards old
//! contents. This Rust port uses `Vec<u8>` as the owned storage.
//!
//! One deliberate safety improvement: new allocations are zero-filled rather
//! than left uninitialized. Reading uninitialized bytes is not supported
//! behavior we should preserve across the migration.

use crate::slice::Slice;

/// Owning byte buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Buffer {
    bytes: Vec<u8>,
}

impl Buffer {
    /// Create an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a buffer with `size` bytes of storage.
    pub fn with_size(size: usize) -> Self {
        Self {
            bytes: vec![0; size],
        }
    }

    /// Create a buffer as a copy of existing bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            bytes: data.to_vec(),
        }
    }

    /// Construct from a borrowed `Slice`.
    pub fn from_slice(slice: Slice<'_>) -> Self {
        Self::from_bytes(slice.data())
    }

    /// Number of bytes in the buffer.
    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    pub fn empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn data(&self) -> &[u8] {
        &self.bytes
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    pub fn clear(&mut self) {
        self.bytes = Vec::new();
    }

    /// Reallocate the storage and discard existing contents.
    pub fn alloc(&mut self, size: usize) -> &mut [u8] {
        if size != self.bytes.len() {
            self.bytes = vec![0; size];
        }
        self.bytes.as_mut_slice()
    }

    /// Assign from a slice by copying bytes.
    pub fn assign_slice(&mut self, slice: Slice<'_>) {
        self.bytes.clear();
        self.bytes.extend_from_slice(slice.data());
    }

    pub fn as_slice(&self) -> Slice<'_> {
        Slice::new(&self.bytes)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, u8> {
        self.bytes.iter()
    }
}

impl<'a> From<Slice<'a>> for Buffer {
    fn from(value: Slice<'a>) -> Self {
        Self::from_slice(value)
    }
}

impl From<&[u8]> for Buffer {
    fn from(value: &[u8]) -> Self {
        Self::from_bytes(value)
    }
}

impl<'a> From<&'a Buffer> for Slice<'a> {
    fn from(value: &'a Buffer) -> Self {
        value.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::Buffer;
    use crate::slice::Slice;

    #[test]
    fn size_and_empty_match_cpp_role() {
        let empty = Buffer::new();
        assert!(empty.empty());
        assert_eq!(empty.size(), 0);

        let filled = Buffer::with_size(4);
        assert!(!filled.empty());
        assert_eq!(filled.size(), 4);
        assert_eq!(filled.data(), &[0, 0, 0, 0]);
    }

    #[test]
    fn copying_and_assignment_preserve_bytes() {
        let source = Buffer::from_bytes(&[1, 2, 3, 4]);
        let copy = source.clone();
        assert_eq!(copy, source);

        let mut assigned = Buffer::new();
        assigned.assign_slice(Slice::new(&[9, 8, 7]));
        assert_eq!(assigned.data(), &[9, 8, 7]);
    }

    #[test]
    fn alloc_discards_old_contents_and_resizes() {
        let mut buffer = Buffer::from_bytes(&[1, 2, 3]);
        let new_data = buffer.alloc(5);
        assert_eq!(new_data, &[0, 0, 0, 0, 0]);
        assert_eq!(buffer.size(), 5);
    }

    #[test]
    fn clear_resets_to_empty() {
        let mut buffer = Buffer::from_bytes(&[1, 2, 3]);
        buffer.clear();
        assert!(buffer.empty());
        assert_eq!(buffer.data(), &[]);

        let replacement = buffer.alloc(3);
        assert_eq!(replacement, &[0, 0, 0]);
    }
}
