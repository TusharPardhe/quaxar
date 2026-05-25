//! Rust port of `xrpl/basics/contract.h`.
//!
//! Important Rust note:
//! - Normal recoverable errors should usually use `Result<T, E>`.
//! - These helpers are for contract/invariant failures and exception-like
//!   control flow in the existing reference code.
//! - The closest Rust primitive is panic unwinding, so this module wraps
//!   `panic_any` / `resume_unwind`.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::panic::{panic_any, resume_unwind};
use std::sync::{Arc, OnceLock, RwLock};

pub trait ContractLogger: Any + Send + Sync + 'static {
    fn log_throw(&self, title: &str);
}

#[derive(Debug, Default)]
pub struct NullContractLogger;

impl ContractLogger for NullContractLogger {
    fn log_throw(&self, _title: &str) {}
}

fn contract_logger() -> &'static RwLock<Arc<dyn ContractLogger>> {
    static LOGGER: OnceLock<RwLock<Arc<dyn ContractLogger>>> = OnceLock::new();
    LOGGER.get_or_init(|| RwLock::new(Arc::new(NullContractLogger)))
}

/// Logging hook matching the role of reference `LogThrow`.
pub fn log_throw(title: &str) {
    let logger = {
        contract_logger()
            .read()
            .expect("contract logger lock should not be poisoned")
            .clone()
    };
    logger.log_throw(title);
}

pub fn replace_contract_logger(logger: Arc<dyn ContractLogger>) -> Arc<dyn ContractLogger> {
    std::mem::replace(
        &mut *contract_logger()
            .write()
            .expect("contract logger lock should not be poisoned"),
        logger,
    )
}

pub fn reset_contract_logger() -> Arc<dyn ContractLogger> {
    replace_contract_logger(Arc::new(NullContractLogger))
}

/// Panic with a typed payload, mirroring the reference `Throw<E>(...)` helper.
///
/// This is intentionally not the default Rust way to model ordinary failures.
/// We use it only for compatibility boundaries where exception-style
/// unwinding is part of the current implementation.
pub fn throw<E>(error: E) -> !
where
    E: Error + Send + 'static,
{
    log_throw(&format!(
        "Throwing exception of type {}: {}",
        std::any::type_name::<E>(),
        error
    ));
    panic_any(error);
}

/// Resume unwinding with an already-caught panic payload.
pub fn rethrow(payload: Box<dyn Any + Send>) -> ! {
    log_throw("Re-throwing exception");
    resume_unwind(payload);
}

/// Payload used for invariant-breaking logic errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    message: String,
}

impl ContractViolation {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ContractViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ContractViolation {}

/// Rust equivalent of the reference `LogicError`.
///
/// The reference implementation logs, emits instrumentation, and aborts. We keep the
/// message and invariant-failure meaning here while preserving the throw-log
/// hook and using panic unwinding so the behavior remains testable.
pub fn logic_error(message: impl Into<String>) -> ! {
    let violation = ContractViolation::new(message);
    log_throw(&format!("Logic error: {}", violation));
    panic_any(violation);
}

#[cfg(test)]
mod tests {
    use super::{
        ContractLogger, ContractViolation, logic_error, replace_contract_logger,
        reset_contract_logger, rethrow, throw,
    };
    use std::any::Any;
    use std::error::Error;
    use std::fmt;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
    use std::thread::ThreadId;

    #[derive(Debug)]
    struct RuntimeError(String);

    impl RuntimeError {
        fn new(message: impl Into<String>) -> Self {
            Self(message.into())
        }
    }

    impl fmt::Display for RuntimeError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(&self.0)
        }
    }

    impl Error for RuntimeError {}

    #[derive(Debug, Default)]
    struct RecordingLogger {
        owner: Mutex<Option<ThreadId>>,
        entries: Mutex<Vec<String>>,
    }

    impl ContractLogger for RecordingLogger {
        fn log_throw(&self, title: &str) {
            let current = std::thread::current().id();
            if self
                .owner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_some_and(|owner| owner != current)
            {
                return;
            }
            self.entries
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(title.to_string());
        }
    }

    fn logger_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct LoggerGuard {
        previous: Option<Arc<dyn ContractLogger>>,
        _lock: MutexGuard<'static, ()>,
    }

    impl LoggerGuard {
        fn install(logger: Arc<dyn ContractLogger>) -> Self {
            let lock = logger_test_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(recording) = (logger.as_ref() as &dyn Any).downcast_ref::<RecordingLogger>()
            {
                *recording
                    .owner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(std::thread::current().id());
            }
            let previous = replace_contract_logger(logger);
            Self {
                previous: Some(previous),
                _lock: lock,
            }
        }
    }

    impl Drop for LoggerGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                let _ = replace_contract_logger(previous);
            }
        }
    }

    fn panic_payload_to_runtime_error(payload: Box<dyn Any + Send>) -> Box<RuntimeError> {
        payload
            .downcast::<RuntimeError>()
            .expect("expected RuntimeError payload")
    }

    #[test]
    fn throw_and_rethrow_preserve_type_and_message() {
        let logger = Arc::new(RecordingLogger::default());
        let _guard = LoggerGuard::install(logger.clone());

        let first_payload = catch_unwind(AssertUnwindSafe(|| {
            throw(RuntimeError::new("Throw test"));
        }))
        .expect_err("throw should unwind");

        let first_error = panic_payload_to_runtime_error(first_payload);
        assert_eq!(first_error.to_string(), "Throw test");

        let second_payload = catch_unwind(AssertUnwindSafe(|| {
            rethrow(first_error as Box<dyn Any + Send>);
        }))
        .expect_err("rethrow should unwind");

        let second_error = panic_payload_to_runtime_error(second_payload);
        assert_eq!(second_error.to_string(), "Throw test");

        let entries = logger
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].contains("Throwing exception of type"));
        assert_eq!(entries[1], "Re-throwing exception");
    }

    #[test]
    fn logic_error_preserves_message() {
        let logger = Arc::new(RecordingLogger::default());
        let _guard = LoggerGuard::install(logger.clone());

        let payload = catch_unwind(AssertUnwindSafe(|| {
            logic_error("broken invariant");
        }))
        .expect_err("logic_error should unwind");

        let violation = payload
            .downcast::<ContractViolation>()
            .expect("expected ContractViolation payload");

        assert_eq!(violation.message(), "broken invariant");

        let entries = logger
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(entries, vec!["Logic error: broken invariant"]);
    }

    #[test]
    fn reset_contract_logger_restores_default_logger() {
        let logger = Arc::new(RecordingLogger::default());
        let _guard = LoggerGuard::install(logger);
        let restored = reset_contract_logger();
        drop(restored);
    }
}
