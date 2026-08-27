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
    use protocol::{Rules, fix_cleanup_3_2_0, fix_cleanup_3_3_0, is_feature_enabled};

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

    #[test]
    fn cleanup_amendments_enter_the_current_rules_step_scope() {
        for amendment in [fix_cleanup_3_2_0(), fix_cleanup_3_3_0()] {
            let rules = Rules::new([amendment]);
            assert!(with_transaction_step_runtime(&rules, || {
                is_feature_enabled(&amendment)
            }));
            assert!(!is_feature_enabled(&amendment));
        }
    }
}
