//! Explicit no-`multiTxn` `canBeHeld(...)` fallback inside `TxQ::apply(...)`.
//!
//! This ports only the deterministic control-flow rule after the clear-ahead
//! heuristic: if `multiTxn` already exists, `canBeHeld(...)` has already been
//! verified and this fallback is skipped; otherwise the exact `canBeHeld(...)`
//! result either allows queueing to continue or rejects immediately.

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyHoldFallback {
    AlreadyVerifiedByMultiTxn,
    HoldAllowed,
    RejectCannotHold(Ter),
}

impl QueueApplyHoldFallback {
    pub const fn ter(self) -> Option<Ter> {
        match self {
            Self::RejectCannotHold(ter) => Some(ter),
            Self::AlreadyVerifiedByMultiTxn | Self::HoldAllowed => None,
        }
    }
}

pub fn evaluate_queue_apply_hold_fallback(
    has_multi_txn: bool,
    can_be_held_result: Ter,
) -> QueueApplyHoldFallback {
    if has_multi_txn {
        return QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn;
    }

    if can_be_held_result == Ter::TES_SUCCESS {
        return QueueApplyHoldFallback::HoldAllowed;
    }

    QueueApplyHoldFallback::RejectCannotHold(can_be_held_result)
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{QueueApplyHoldFallback, evaluate_queue_apply_hold_fallback};

    #[test]
    fn hold_fallback_skips_canbeheld_when_multitxn_already_exists() {
        let result = evaluate_queue_apply_hold_fallback(true, Ter::TEL_CAN_NOT_QUEUE_FULL);

        assert_eq!(result, QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn);
        assert_eq!(result.ter(), None);
    }

    #[test]
    fn hold_fallback_allows_queueing_when_canbeheld_succeeds() {
        let result = evaluate_queue_apply_hold_fallback(false, Ter::TES_SUCCESS);

        assert_eq!(result, QueueApplyHoldFallback::HoldAllowed);
        assert_eq!(result.ter(), None);
    }

    #[test]
    fn hold_fallback_propagates_exact_canbeheld_failure() {
        let result = evaluate_queue_apply_hold_fallback(false, Ter::TEL_CAN_NOT_QUEUE);

        assert_eq!(
            result,
            QueueApplyHoldFallback::RejectCannotHold(Ter::TEL_CAN_NOT_QUEUE)
        );
        assert_eq!(result.ter(), Some(Ter::TEL_CAN_NOT_QUEUE));
    }
}
