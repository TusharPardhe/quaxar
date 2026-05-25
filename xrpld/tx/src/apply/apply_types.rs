//! Deterministic `tx/applySteps.h` carrier helpers that do not require
//! transaction execution.

pub use protocol::{ApplyFlags, any_apply_flags};
use protocol::{Ter, is_tec_claim, is_tes_success};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    pub ter: Ter,
    pub applied: bool,
    pub metadata_present: bool,
}

impl ApplyResult {
    pub const fn new(ter: Ter, applied: bool, metadata_present: bool) -> Self {
        Self {
            ter,
            applied,
            metadata_present,
        }
    }
}

pub fn is_tec_claim_hard_fail(ter: Ter, flags: ApplyFlags) -> bool {
    is_tec_claim(ter) && !any_apply_flags(flags & ApplyFlags::RETRY)
}

pub fn likely_to_claim_fee(ter: Ter, flags: ApplyFlags) -> bool {
    is_tes_success(ter) || is_tec_claim_hard_fail(ter, flags)
}

#[cfg(test)]
mod tests {
    use super::{
        ApplyFlags, ApplyResult, any_apply_flags, is_tec_claim_hard_fail, likely_to_claim_fee,
    };
    use protocol::Ter;

    #[test]
    fn apply_flag_bitwise_behavior_matches_current_cpp_constants() {
        assert_eq!(ApplyFlags::NONE.bits(), 0x00);
        assert_eq!(ApplyFlags::FAIL_HARD.bits(), 0x10);
        assert_eq!(ApplyFlags::RETRY.bits(), 0x20);
        assert_eq!(ApplyFlags::UNLIMITED.bits(), 0x400);
        assert_eq!(ApplyFlags::BATCH.bits(), 0x800);
        assert_eq!(ApplyFlags::DRY_RUN.bits(), 0x1000);
        assert_eq!((ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).bits(), 0x30);
        assert_eq!((ApplyFlags::FAIL_HARD & ApplyFlags::RETRY).bits(), 0x00);
    }

    #[test]
    fn apply_result_preserves_constructor_inputs() {
        let result = ApplyResult::new(Ter::TEC_EXPIRED, true, false);

        assert_eq!(result.ter, Ter::TEC_EXPIRED);
        assert!(result.applied);
        assert!(!result.metadata_present);
    }

    #[test]
    fn fee_claim_helper_matches_current_cpp_rule() {
        assert!(is_tec_claim_hard_fail(Ter::TEC_CLAIM, ApplyFlags::NONE));
        assert!(!is_tec_claim_hard_fail(Ter::TEC_CLAIM, ApplyFlags::RETRY));
        assert!(likely_to_claim_fee(Ter::TES_SUCCESS, ApplyFlags::RETRY));
        assert!(likely_to_claim_fee(Ter::TEC_CLAIM, ApplyFlags::NONE));
        assert!(!likely_to_claim_fee(Ter::TER_RETRY, ApplyFlags::NONE));
        assert!(any_apply_flags(ApplyFlags::DRY_RUN));
    }

    #[test]
    fn apply_flag_display_uses_cpp_integer_stream_shape() {
        assert_eq!(ApplyFlags::NONE.to_string(), "0");
        assert_eq!(
            (ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).to_string(),
            "48"
        );
    }
}
