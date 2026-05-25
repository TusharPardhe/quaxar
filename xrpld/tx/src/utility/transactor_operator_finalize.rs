//! Current Rust helper mirroring the final apply/metadata slice of
//! `Transactor::operator()()`.
//!
//! This module preserves the exact current tail policy:
//!
//! - skip all finalization work when `applied` is already false,
//! - reject negative fees before any later side effect,
//! - destroy XRP only on closed ledgers and only when the charged fee is
//!   non-zero,
//! - call the final `apply(result)` after any fee destruction work, and
//! - force the returned `applied` flag back to false on `tapDRY_RUN` without
//!   discarding the produced metadata.

use crate::{ApplyFlags, any_apply_flags};
use protocol::Ter;

pub const TRANSACTOR_OPERATOR_FINALIZE_NEGATIVE_FEE_MESSAGE: &str = "fee charged is negative!";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactorOperatorFinalize<Meta> {
    pub result: Ter,
    pub applied: bool,
    pub metadata: Option<Meta>,
}

pub fn run_transactor_operator_finalize<Fee, Meta>(
    result: Ter,
    applied: bool,
    fee: Fee,
    flags: ApplyFlags,
    view_open: bool,
    destroy_xrp: impl FnOnce(Fee),
    apply: impl FnOnce(Ter) -> Meta,
) -> TransactorOperatorFinalize<Meta>
where
    Fee: Clone + Default + PartialEq + PartialOrd,
{
    let metadata = if applied {
        let zero = Fee::default();
        if fee < zero {
            panic!("{TRANSACTOR_OPERATOR_FINALIZE_NEGATIVE_FEE_MESSAGE}");
        }

        if !view_open && fee != zero {
            destroy_xrp(fee);
        }

        Some(apply(result))
    } else {
        None
    };

    let applied = applied && !any_apply_flags(flags & ApplyFlags::DRY_RUN);

    TransactorOperatorFinalize {
        result,
        applied,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        panic::{AssertUnwindSafe, catch_unwind},
    };

    use protocol::{Ter, trans_token};

    use crate::ApplyFlags;

    use super::{
        TRANSACTOR_OPERATOR_FINALIZE_NEGATIVE_FEE_MESSAGE, TransactorOperatorFinalize,
        run_transactor_operator_finalize,
    };

    #[test]
    fn transactor_operator_finalize_skips_tail_when_not_applied() {
        let destroyed = Cell::new(false);
        let applied = Cell::new(false);

        let result = run_transactor_operator_finalize(
            Ter::TEC_CLAIM,
            false,
            10_i64,
            ApplyFlags::NONE,
            false,
            |_| destroyed.set(true),
            |_| {
                applied.set(true);
                "metadata"
            },
        );

        assert_eq!(
            result,
            TransactorOperatorFinalize {
                result: Ter::TEC_CLAIM,
                applied: false,
                metadata: None::<&'static str>,
            }
        );
        assert_eq!(trans_token(result.result), "tecCLAIM");
        assert!(!destroyed.get());
        assert!(!applied.get());
    }

    #[test]
    fn transactor_operator_finalize_destroys_fee_before_apply_on_closed_ledger() {
        let trace = RefCell::new(Vec::new());

        let result = run_transactor_operator_finalize(
            Ter::TES_SUCCESS,
            true,
            12_i64,
            ApplyFlags::NONE,
            false,
            |fee| {
                assert_eq!(fee, 12);
                trace.borrow_mut().push("destroy");
            },
            |incoming| {
                assert_eq!(incoming, Ter::TES_SUCCESS);
                trace.borrow_mut().push("apply");
                "metadata"
            },
        );

        assert_eq!(
            result,
            TransactorOperatorFinalize {
                result: Ter::TES_SUCCESS,
                applied: true,
                metadata: Some("metadata"),
            }
        );
        assert_eq!(trace.into_inner(), vec!["destroy", "apply"]);
    }

    #[test]
    fn transactor_operator_finalize_keeps_metadata_but_clears_applied_on_dry_run() {
        let result = run_transactor_operator_finalize(
            Ter::TES_SUCCESS,
            true,
            0_i64,
            ApplyFlags::DRY_RUN,
            true,
            |_| panic!("zero fee on open ledger should skip destroy"),
            |incoming| {
                assert_eq!(incoming, Ter::TES_SUCCESS);
                "metadata"
            },
        );

        assert_eq!(
            result,
            TransactorOperatorFinalize {
                result: Ter::TES_SUCCESS,
                applied: false,
                metadata: Some("metadata"),
            }
        );
    }

    #[test]
    fn transactor_operator_finalize_rejects_negative_fee_before_side_effects() {
        let destroyed = Cell::new(false);
        let applied = Cell::new(false);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = run_transactor_operator_finalize(
                Ter::TES_SUCCESS,
                true,
                -1_i64,
                ApplyFlags::NONE,
                false,
                |_| destroyed.set(true),
                |_| {
                    applied.set(true);
                    "metadata"
                },
            );
        }))
        .expect_err("negative fee should panic");

        let message = if let Some(message) = panic.downcast_ref::<String>() {
            message.as_str()
        } else if let Some(message) = panic.downcast_ref::<&'static str>() {
            message
        } else {
            panic!("unexpected panic payload");
        };

        assert!(message.contains(TRANSACTOR_OPERATOR_FINALIZE_NEGATIVE_FEE_MESSAGE));
        assert!(!destroyed.get());
        assert!(!applied.get());
    }
}
