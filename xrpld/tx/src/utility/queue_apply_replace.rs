//! Replacement-fee rule inside `TxQ::apply(...)`.
//!
//! This ports only the current `increase(existingFee, retrySequencePercent)`
//! threshold and the strict `>` comparison that decides whether a queued
//! transaction with the same `SeqProxy` may be replaced.

use basics::mul_div::{MULDIV_MAX, mul_div};
use protocol::Ter;

use crate::FeeLevel64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementFeeDecision {
    ReplaceAllowed { required_fee_level: FeeLevel64 },
    InsufficientFee { required_fee_level: FeeLevel64 },
}

impl ReplacementFeeDecision {
    pub const fn ter(self) -> Option<Ter> {
        match self {
            Self::ReplaceAllowed { .. } => None,
            Self::InsufficientFee { .. } => Some(Ter::TEL_CAN_NOT_QUEUE_FEE),
        }
    }

    pub const fn required_fee_level(self) -> FeeLevel64 {
        match self {
            Self::ReplaceAllowed { required_fee_level }
            | Self::InsufficientFee { required_fee_level } => required_fee_level,
        }
    }
}

pub fn increase_replacement_fee_level(
    existing_fee_level: FeeLevel64,
    retry_sequence_percent: u32,
) -> FeeLevel64 {
    mul_div(
        existing_fee_level,
        100 + u64::from(retry_sequence_percent),
        100,
    )
    .unwrap_or(MULDIV_MAX)
}

pub fn evaluate_replacement_fee(
    paid_fee_level: FeeLevel64,
    existing_fee_level: FeeLevel64,
    retry_sequence_percent: u32,
) -> ReplacementFeeDecision {
    let required_fee_level =
        increase_replacement_fee_level(existing_fee_level, retry_sequence_percent);

    if paid_fee_level > required_fee_level {
        ReplacementFeeDecision::ReplaceAllowed { required_fee_level }
    } else {
        ReplacementFeeDecision::InsufficientFee { required_fee_level }
    }
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{ReplacementFeeDecision, evaluate_replacement_fee, increase_replacement_fee_level};

    #[test]
    fn increase_replacement_fee_level_matches_current_cpp_percentage_rule() {
        assert_eq!(increase_replacement_fee_level(100, 25), 125);
        assert_eq!(increase_replacement_fee_level(101, 25), 126);
        assert_eq!(increase_replacement_fee_level(100, 0), 100);
    }

    #[test]
    fn increase_replacement_fee_level_saturates_on_mul_div_overflow() {
        assert_eq!(increase_replacement_fee_level(u64::MAX, 100), u64::MAX);
    }

    #[test]
    fn replacement_fee_requires_strictly_more_than_the_threshold() {
        let equal = evaluate_replacement_fee(125, 100, 25);
        let higher = evaluate_replacement_fee(126, 100, 25);

        assert_eq!(
            equal,
            ReplacementFeeDecision::InsufficientFee {
                required_fee_level: 125
            }
        );
        assert_eq!(equal.ter(), Some(Ter::TEL_CAN_NOT_QUEUE_FEE));
        assert_eq!(
            higher,
            ReplacementFeeDecision::ReplaceAllowed {
                required_fee_level: 125
            }
        );
        assert_eq!(higher.ter(), None);
    }
}
