//! Static `LoanSet` metadata helpers mirrored from the reference implementation.
//!
//! This module exposes the current deterministic metadata behavior for:
//!
//! - delegating `checkExtraFeatures(...)` to the shared lending protocol gate,
//! - and returning the literal current `tfLoanSetMask` value from
//!   `getFlagsMask(...)`.

use crate::run_check_lending_protocol_dependencies;

pub const LOAN_SET_OVERPAYMENT_FLAG: u32 = 0x0001_0000;
pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = 0x8000_0000;
pub const INNER_BATCH_TRANSACTION_FLAG: u32 = 0x4000_0000;
pub const LOAN_SET_FLAGS_MASK: u32 =
    !(FULLY_CANONICAL_SIGNATURE_FLAG | INNER_BATCH_TRANSACTION_FLAG | LOAN_SET_OVERPAYMENT_FLAG);

pub fn run_loan_set_check_extra_features(
    single_asset_vault_enabled: bool,
    check_vault_create_extra_features: impl FnOnce() -> bool,
) -> bool {
    run_check_lending_protocol_dependencies(
        single_asset_vault_enabled,
        check_vault_create_extra_features,
    )
}

pub const fn get_loan_set_flags_mask() -> u32 {
    LOAN_SET_FLAGS_MASK
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        FULLY_CANONICAL_SIGNATURE_FLAG, INNER_BATCH_TRANSACTION_FLAG, LOAN_SET_FLAGS_MASK,
        LOAN_SET_OVERPAYMENT_FLAG, get_loan_set_flags_mask, run_loan_set_check_extra_features,
    };

    #[test]
    fn loan_set_check_extra_features_short_circuits_when_single_asset_vault_is_disabled() {
        let vault_helper_called = Cell::new(false);

        let result = run_loan_set_check_extra_features(false, || {
            vault_helper_called.set(true);
            true
        });

        assert!(!result);
        assert!(!vault_helper_called.get());
    }

    #[test]
    fn loan_set_check_extra_features_returns_vault_helper_result() {
        assert!(run_loan_set_check_extra_features(true, || true));
        assert!(!run_loan_set_check_extra_features(true, || false));
    }

    #[test]
    fn loan_set_flags_mask_txflags() {
        assert_eq!(LOAN_SET_OVERPAYMENT_FLAG, 0x0001_0000);
        assert_eq!(FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
        assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
        assert_eq!(LOAN_SET_FLAGS_MASK, 0x3ffe_ffff);
        assert_eq!(get_loan_set_flags_mask(), 0x3ffe_ffff);
    }

    #[test]
    fn loan_set_flags_mask_rejects_only_universal_and_overpayment_flags() {
        assert_eq!(
            get_loan_set_flags_mask() & FULLY_CANONICAL_SIGNATURE_FLAG,
            0
        );
        assert_eq!(get_loan_set_flags_mask() & INNER_BATCH_TRANSACTION_FLAG, 0);
        assert_eq!(get_loan_set_flags_mask() & LOAN_SET_OVERPAYMENT_FLAG, 0);
        assert_eq!(get_loan_set_flags_mask() & 0x0002_0000, 0x0002_0000);
    }
}
