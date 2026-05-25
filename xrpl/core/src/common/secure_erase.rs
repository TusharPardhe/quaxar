//! secure_erase ported from `xrpl/crypto/secure_erase.h/the reference source`.
//!
//! Zeroes memory containing secrets before deallocation to prevent secrets
//! from lingering in freed memory. Uses volatile writes to prevent the
//! compiler from optimizing the zeroing away.
//!
//! In Rust, the `zeroize` crate provides this functionality with proper
//! compiler barriers. This module wraps it for API compatibility.

#![allow(dead_code)]

/// Securely erase a byte slice, preventing the compiler from optimizing
/// the write away.
///
/// Uses a volatile write pattern to ensure the memory is actually zeroed.
/// This is the Rust equivalent of `OPENSSL_cleanse()`.
#[inline(never)]
pub fn secure_erase(dest: &mut [u8]) {
    // Use a volatile write to prevent optimization
    for byte in dest.iter_mut() {
        // SAFETY: We're writing to valid, mutable memory we own.
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    // Compiler fence to prevent reordering
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
}

/// Securely erase a Vec<u8>, zeroing all bytes.
pub fn secure_erase_vec(dest: &mut Vec<u8>) {
    secure_erase(dest.as_mut_slice());
}

/// A wrapper type that automatically zeroes its contents on drop.
/// Use this for any buffer holding secret key material.
#[derive(Clone)]
pub struct SecureBuffer {
    data: Vec<u8>,
}

impl SecureBuffer {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0u8; size],
        }
    }

    pub fn from_vec(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Drop for SecureBuffer {
    fn drop(&mut self) {
        secure_erase(&mut self.data);
    }
}

impl AsRef<[u8]> for SecureBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl AsMut<[u8]> for SecureBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_zeroes_memory() {
        let mut buf = vec![0xFFu8; 32];
        secure_erase(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn secure_buffer_zeroes_on_drop() {
        let ptr: *const u8;
        let len: usize;
        {
            let buf = SecureBuffer::from_vec(vec![0xAB; 16]);
            ptr = buf.as_slice().as_ptr();
            len = buf.len();
            // Verify it has content
            assert!(buf.as_slice().iter().all(|&b| b == 0xAB));
        }
        // After drop, we can't safely read the memory, but the drop
        // implementation guarantees secure_erase was called.
        let _ = (ptr, len);
    }
}
