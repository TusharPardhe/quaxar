//! Compatibility shim for `xrpl/basics/spinlock.h`.

use std::sync::atomic::{AtomicUsize, Ordering};

pub fn spin_pause() {
    std::hint::spin_loop();
}

#[derive(Debug)]
pub struct PackedSpinLock<'a> {
    bits: &'a AtomicUsize,
    mask: usize,
}

impl<'a> PackedSpinLock<'a> {
    pub fn new(bits: &'a AtomicUsize, index: usize) -> Self {
        let mask = 1usize
            .checked_shl(index as u32)
            .expect("packed spinlock index must be valid");
        Self { bits, mask }
    }

    pub fn try_lock(&self) -> bool {
        (self.bits.fetch_or(self.mask, Ordering::Acquire) & self.mask) == 0
    }

    pub fn lock(&self) {
        while !self.try_lock() {
            while (self.bits.load(Ordering::Relaxed) & self.mask) != 0 {
                spin_pause();
            }
        }
    }

    pub fn unlock(&self) {
        self.bits.fetch_and(!self.mask, Ordering::Release);
    }
}

#[derive(Debug)]
pub struct SpinLock<'a> {
    bits: &'a AtomicUsize,
}

impl<'a> SpinLock<'a> {
    pub fn new(bits: &'a AtomicUsize) -> Self {
        Self { bits }
    }

    pub fn try_lock(&self) -> bool {
        self.bits
            .compare_exchange(0, usize::MAX, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    pub fn lock(&self) {
        while !self.try_lock() {
            while self.bits.load(Ordering::Relaxed) != 0 {
                spin_pause();
            }
        }
    }

    pub fn unlock(&self) {
        self.bits.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::{PackedSpinLock, SpinLock};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn packed_spinlock_grabs_and_releases_single_bit() {
        let bits = AtomicUsize::new(0);
        let lock = PackedSpinLock::new(&bits, 3);
        assert!(lock.try_lock());
        assert!(!lock.try_lock());
        assert_eq!(bits.load(Ordering::SeqCst), 1 << 3);
        lock.unlock();
        assert_eq!(bits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn spinlock_grabs_all_bits() {
        let bits = AtomicUsize::new(0);
        let lock = SpinLock::new(&bits);
        assert!(lock.try_lock());
        assert_eq!(bits.load(Ordering::SeqCst), usize::MAX);
        lock.unlock();
        assert_eq!(bits.load(Ordering::SeqCst), 0);
    }
}
