//! Staged `multiTxn` preclaim-view preparation block inside `TxQ::apply(...)`.
//!
//! This helper ports the current deterministic branch order:
//! 1. no staged view means `preclaim(...)` runs against the current view,
//! 2. a staged view must be prepared before `preclaim(...)` can use it,
//! 3. failure to expose the staged account object returns `tefINTERNAL`.

use protocol::Ter;

use crate::{ApplyResult, QueueApplyPreclaimViewSource, QueueApplyViewAdjustment};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueApplyPreclaimViewContext {
    pub view_source: QueueApplyPreclaimViewSource,
}

impl QueueApplyPreclaimViewContext {
    pub fn has_multi_txn(self) -> bool {
        self.view_source.has_multi_txn()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyPreclaimViewStage {
    RejectInternal,
    Ready(QueueApplyPreclaimViewContext),
}

impl QueueApplyPreclaimViewStage {
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::RejectInternal => ApplyResult::new(Ter::TEF_INTERNAL, false, false),
            Self::Ready(_) => ApplyResult::new(Ter::TES_SUCCESS, false, false),
        }
    }
}

pub fn run_queue_apply_preclaim_view_stage<PrepareMultiTxn>(
    view_adjustment: Option<QueueApplyViewAdjustment>,
    prepare_multitxn: PrepareMultiTxn,
) -> QueueApplyPreclaimViewStage
where
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
{
    match view_adjustment {
        None => QueueApplyPreclaimViewStage::Ready(QueueApplyPreclaimViewContext {
            view_source: QueueApplyPreclaimViewSource::CurrentView,
        }),
        Some(adjustment) => {
            if prepare_multitxn(adjustment) {
                QueueApplyPreclaimViewStage::Ready(QueueApplyPreclaimViewContext {
                    view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
                })
            } else {
                QueueApplyPreclaimViewStage::RejectInternal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::Ter;

    use super::{
        QueueApplyPreclaimViewContext, QueueApplyPreclaimViewStage,
        run_queue_apply_preclaim_view_stage,
    };
    use crate::{ApplyResult, QueueApplyPreclaimViewSource, QueueApplyViewAdjustment};

    #[test]
    fn preclaim_view_stage_uses_current_view_without_preparing_multitxn() {
        let prepared = Cell::new(false);

        let stage = run_queue_apply_preclaim_view_stage(None, |_| {
            prepared.set(true);
            true
        });

        assert_eq!(
            stage,
            QueueApplyPreclaimViewStage::Ready(QueueApplyPreclaimViewContext {
                view_source: QueueApplyPreclaimViewSource::CurrentView,
            })
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, false, false)
        );
        assert!(!prepared.get());
    }

    #[test]
    fn preclaim_view_stage_prepares_multitxn_view_when_adjustment_is_present() {
        let received = Cell::new(None::<QueueApplyViewAdjustment>);

        let stage = run_queue_apply_preclaim_view_stage(
            Some(QueueApplyViewAdjustment {
                potential_total_spend_drops: 250,
                adjusted_balance_drops: 750,
                applied_sequence_value: 7,
            }),
            |adjustment| {
                received.set(Some(adjustment));
                true
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreclaimViewStage::Ready(QueueApplyPreclaimViewContext {
                view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
            })
        );
        assert_eq!(
            received.get(),
            Some(QueueApplyViewAdjustment {
                potential_total_spend_drops: 250,
                adjusted_balance_drops: 750,
                applied_sequence_value: 7,
            })
        );
    }

    #[test]
    fn preclaim_view_stage_rejects_internal_when_multitxn_preparation_fails() {
        let stage = run_queue_apply_preclaim_view_stage(
            Some(QueueApplyViewAdjustment {
                potential_total_spend_drops: 250,
                adjusted_balance_drops: 750,
                applied_sequence_value: 7,
            }),
            |_| false,
        );

        assert_eq!(stage, QueueApplyPreclaimViewStage::RejectInternal);
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEF_INTERNAL, false, false)
        );
    }
}
