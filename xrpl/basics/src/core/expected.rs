//! Rust port of `xrpl/basics/Expected.h`.
//!
//! This is a compatibility layer over `Result<T, E>` with reference-style accessors
//! and invalid-access behavior that panics with `BadExpectedAccess`.

use crate::contract::throw;
use std::error::Error;
use std::fmt;
use std::ops::Deref;

/// Error raised when `value()` or `error()` is called on the wrong variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadExpectedAccess;

impl fmt::Display for BadExpectedAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bad expected access")
    }
}

impl Error for BadExpectedAccess {}

/// Error wrapper used to construct the error variant of `Expected<T, E>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unexpected<E> {
    value: E,
}

impl<E> Unexpected<E> {
    pub fn new(value: E) -> Self {
        Self { value }
    }

    pub fn value(&self) -> &E {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut E {
        &mut self.value
    }

    pub fn into_value(self) -> E {
        self.value
    }
}

/// Rust compatibility wrapper mirroring the role of reference `Expected<T, E>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected<T, E> {
    inner: Result<T, E>,
}

impl<T, E> Expected<T, E> {
    pub fn from_value<U>(value: U) -> Self
    where
        U: Into<T>,
    {
        Self {
            inner: Ok(value.into()),
        }
    }

    pub fn from_unexpected<U>(error: Unexpected<U>) -> Self
    where
        U: Into<E>,
    {
        Self {
            inner: Err(error.into_value().into()),
        }
    }

    pub fn has_value(&self) -> bool {
        self.inner.is_ok()
    }

    pub fn as_bool(&self) -> bool {
        self.has_value()
    }

    pub fn value(&self) -> &T {
        match &self.inner {
            Ok(value) => value,
            Err(_) => throw(BadExpectedAccess),
        }
    }

    pub fn value_mut(&mut self) -> &mut T {
        match &mut self.inner {
            Ok(value) => value,
            Err(_) => throw(BadExpectedAccess),
        }
    }

    pub fn error(&self) -> &E {
        match &self.inner {
            Ok(_) => throw(BadExpectedAccess),
            Err(error) => error,
        }
    }

    pub fn error_mut(&mut self) -> &mut E {
        match &mut self.inner {
            Ok(_) => throw(BadExpectedAccess),
            Err(error) => error,
        }
    }

    pub fn into_result(self) -> Result<T, E> {
        self.inner
    }
}

impl<T, E, U> From<Unexpected<U>> for Expected<T, E>
where
    U: Into<E>,
{
    fn from(error: Unexpected<U>) -> Self {
        Self::from_unexpected(error)
    }
}

impl<E> Default for Expected<(), E> {
    fn default() -> Self {
        Self { inner: Ok(()) }
    }
}

impl<T, E> Deref for Expected<T, E> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value()
    }
}

impl<T, E> From<&Expected<T, E>> for bool {
    fn from(value: &Expected<T, E>) -> Self {
        value.has_value()
    }
}

#[cfg(test)]
mod tests {
    use super::{BadExpectedAccess, Expected, Unexpected};
    use std::fmt;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestError(&'static str);

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    #[test]
    fn expected_holds_success_values() {
        let mut expected: Expected<String, TestError> = Expected::from_value("xrpl");

        assert!(expected.has_value());
        assert_eq!(expected.value(), "xrpl");
        assert_eq!(&*expected, "xrpl");

        expected.value_mut().push_str("-rust");
        assert_eq!(expected.value(), "xrpl-rust");
    }

    #[test]
    fn expected_holds_errors_via_unexpected() {
        let expected: Expected<String, TestError> = Unexpected::new(TestError("boom")).into();

        assert!(!expected.has_value());
        assert_eq!(expected.error(), &TestError("boom"));
    }

    #[test]
    fn default_expected_void_is_success() {
        let expected = Expected::<(), TestError>::default();
        assert!(expected.has_value());
    }

    #[test]
    fn invalid_value_access_panics_with_bad_expected_access() {
        let expected: Expected<String, TestError> = Unexpected::new(TestError("boom")).into();

        let payload = catch_unwind(AssertUnwindSafe(|| {
            let _ = expected.value();
        }))
        .expect_err("value access should unwind");

        let error = payload
            .downcast::<BadExpectedAccess>()
            .expect("expected BadExpectedAccess");
        assert_eq!(error.to_string(), "bad expected access");
    }

    #[test]
    fn invalid_error_access_panics_with_bad_expected_access() {
        let expected: Expected<String, TestError> = Expected::from_value("ok");

        let payload = catch_unwind(AssertUnwindSafe(|| {
            let _ = expected.error();
        }))
        .expect_err("error access should unwind");

        let error = payload
            .downcast::<BadExpectedAccess>()
            .expect("expected BadExpectedAccess");
        assert_eq!(error.to_string(), "bad expected access");
    }

    #[test]
    fn expected_accepts_non_error_payloads() {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct Marker(u8);

        let ok: Expected<u32, Marker> = Expected::from_value(7u32);
        assert_eq!(ok.value(), &7u32);

        let err: Expected<u32, Marker> = Unexpected::new(Marker(9)).into();
        assert_eq!(err.error(), &Marker(9));
        assert!(bool::from(&ok));
        assert!(!bool::from(&err));
        assert!(ok.as_bool());
        assert!(!err.as_bool());
    }
}
