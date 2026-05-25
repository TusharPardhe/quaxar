//! Rust equivalents of the reference scope helpers in `xrpl/basics/scope.h`.
//!
//! This file is a good learning step because it teaches Rust's `Drop` trait.
//! In JS/TS there is no built-in deterministic "run this cleanup exactly when
//! the value leaves scope" mechanism. In Rust, that behavior is fundamental.
//!
//! We model three guards:
//! - `ScopeExit`: always runs unless released
//! - `ScopeFail`: only runs during panic unwinding unless released
//! - `ScopeSuccess`: only runs when not unwinding unless released

/// Guard that always runs its closure on drop unless released.
pub struct ScopeExit<F: FnOnce()> {
    callback: Option<F>,
}

impl<F: FnOnce()> ScopeExit<F> {
    pub fn new(callback: F) -> Self {
        Self {
            callback: Some(callback),
        }
    }

    pub fn release(&mut self) {
        self.callback = None;
    }
}

impl<F: FnOnce()> Drop for ScopeExit<F> {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            callback();
        }
    }
}

/// Guard that runs only if the current scope exits via panic unwinding.
pub struct ScopeFail<F: FnOnce()> {
    callback: Option<F>,
    panicking_on_creation: bool,
}

impl<F: FnOnce()> ScopeFail<F> {
    pub fn new(callback: F) -> Self {
        Self {
            callback: Some(callback),
            panicking_on_creation: std::thread::panicking(),
        }
    }

    pub fn release(&mut self) {
        self.callback = None;
    }
}

impl<F: FnOnce()> Drop for ScopeFail<F> {
    fn drop(&mut self) {
        if std::thread::panicking()
            && !self.panicking_on_creation
            && let Some(callback) = self.callback.take()
        {
            callback();
        }
    }
}

/// Guard that runs only if the current scope exits normally.
pub struct ScopeSuccess<F: FnOnce()> {
    callback: Option<F>,
    panicking_on_creation: bool,
}

impl<F: FnOnce()> ScopeSuccess<F> {
    pub fn new(callback: F) -> Self {
        Self {
            callback: Some(callback),
            panicking_on_creation: std::thread::panicking(),
        }
    }

    pub fn release(&mut self) {
        self.callback = None;
    }
}

impl<F: FnOnce()> Drop for ScopeSuccess<F> {
    fn drop(&mut self) {
        if std::thread::panicking() == self.panicking_on_creation
            && let Some(callback) = self.callback.take()
        {
            callback();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScopeExit, ScopeFail, ScopeSuccess};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn scope_exit_behavior() {
        let mut i = 0;
        {
            let _x = ScopeExit::new(|| i = 1);
        }
        assert_eq!(i, 1);
        {
            let mut x = ScopeExit::new(|| i = 2);
            x.release();
        }
        assert_eq!(i, 1);
        {
            let x = ScopeExit::new(|| i += 2);
            let _x2 = x;
        }
        assert_eq!(i, 3);
        {
            let mut x = ScopeExit::new(|| i = 4);
            x.release();
            let _x2 = x;
        }
        assert_eq!(i, 3);
        {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _x = ScopeExit::new(|| i = 5);
                panic!("forced panic");
            }));
        }
        assert_eq!(i, 5);
        {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let mut x = ScopeExit::new(|| i = 6);
                x.release();
                panic!("forced panic");
            }));
        }
        assert_eq!(i, 5);
    }

    #[test]
    fn scope_fail_behavior() {
        let mut i = 0;
        {
            let _x = ScopeFail::new(|| i = 1);
        }
        assert_eq!(i, 0);
        {
            let mut x = ScopeFail::new(|| i = 2);
            x.release();
        }
        assert_eq!(i, 0);
        {
            let x = ScopeFail::new(|| i = 3);
            let _x2 = x;
        }
        assert_eq!(i, 0);
        {
            let mut x = ScopeFail::new(|| i = 4);
            x.release();
            let _x2 = x;
        }
        assert_eq!(i, 0);
        {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _x = ScopeFail::new(|| i = 5);
                panic!("forced panic");
            }));
        }
        assert_eq!(i, 5);
        {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let mut x = ScopeFail::new(|| i = 6);
                x.release();
                panic!("forced panic");
            }));
        }
        assert_eq!(i, 5);
    }

    #[test]
    fn scope_success_behavior() {
        let mut i = 0;
        {
            let _x = ScopeSuccess::new(|| i = 1);
        }
        assert_eq!(i, 1);
        {
            let mut x = ScopeSuccess::new(|| i = 2);
            x.release();
        }
        assert_eq!(i, 1);
        {
            let x = ScopeSuccess::new(|| i += 2);
            let _x2 = x;
        }
        assert_eq!(i, 3);
        {
            let mut x = ScopeSuccess::new(|| i = 4);
            x.release();
            let _x2 = x;
        }
        assert_eq!(i, 3);
        {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _x = ScopeSuccess::new(|| i = 5);
                panic!("forced panic");
            }));
        }
        assert_eq!(i, 3);
        {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let mut x = ScopeSuccess::new(|| i = 6);
                x.release();
                panic!("forced panic");
            }));
        }
        assert_eq!(i, 3);
    }
}
