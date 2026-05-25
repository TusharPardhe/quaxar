//! `xrpld/tx` caller-level runtime wrappers.
//!
//! These wrappers sit one layer above the protocol-owned ambient-state guards
//! and mirror the the reference implementation caller shapes:
//! - `apply(...)`, `TxQ::apply(...)`, and `Transactor::operator()`
//! - `applySteps::with_txn_type(...)`

use protocol::{Rules, TransactionApplyRuntimeGuard, TransactionStepRuntimeGuard};

pub fn with_transaction_apply_runtime<R>(rules: &Rules, f: impl FnOnce() -> R) -> R {
    let _guard = TransactionApplyRuntimeGuard::new(rules);
    f()
}

pub fn with_transaction_step_runtime<R>(rules: &Rules, f: impl FnOnce() -> R) -> R {
    let _guard = TransactionStepRuntimeGuard::new(rules);
    f()
}

#[cfg(test)]
mod tests {
    use super::{with_transaction_apply_runtime, with_transaction_step_runtime};
    use protocol::Rules;

    #[test]
    fn apply_runtime_returns_closure_result() {
        let rules = Rules::new(std::iter::empty());

        let value = with_transaction_apply_runtime(&rules, || 42_u32);

        assert_eq!(value, 42);
    }

    #[test]
    fn step_runtime_returns_closure_result() {
        let rules = Rules::new(std::iter::empty());

        let value = with_transaction_step_runtime(&rules, || "ok");

        assert_eq!(value, "ok");
    }
}
