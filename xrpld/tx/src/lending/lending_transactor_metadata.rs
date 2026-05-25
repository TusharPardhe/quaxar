//! Remaining static lending transactor metadata helpers that currently just
//! delegate into the shared lending gate or return literal flag masks.

use crate::run_check_lending_protocol_dependencies;

pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = 0x8000_0000;
pub const INNER_BATCH_TRANSACTION_FLAG: u32 = 0x4000_0000;

pub const LOAN_PAY_OVERPAYMENT_FLAG: u32 = 0x0001_0000;
pub const LOAN_FULL_PAYMENT_FLAG: u32 = 0x0002_0000;
pub const LOAN_LATE_PAYMENT_FLAG: u32 = 0x0004_0000;
pub const LOAN_PAY_FLAGS_MASK: u32 = !(FULLY_CANONICAL_SIGNATURE_FLAG
    | INNER_BATCH_TRANSACTION_FLAG
    | LOAN_PAY_OVERPAYMENT_FLAG
    | LOAN_FULL_PAYMENT_FLAG
    | LOAN_LATE_PAYMENT_FLAG);

pub const LOAN_DEFAULT_FLAG: u32 = 0x0001_0000;
pub const LOAN_IMPAIR_FLAG: u32 = 0x0002_0000;
pub const LOAN_UNIMPAIR_FLAG: u32 = 0x0004_0000;
pub const LOAN_MANAGE_FLAGS_MASK: u32 = !(FULLY_CANONICAL_SIGNATURE_FLAG
    | INNER_BATCH_TRANSACTION_FLAG
    | LOAN_DEFAULT_FLAG
    | LOAN_IMPAIR_FLAG
    | LOAN_UNIMPAIR_FLAG);

macro_rules! lending_check_extra_features_wrapper {
    ($name:ident) => {
        pub fn $name(
            single_asset_vault_enabled: bool,
            check_vault_create_extra_features: impl FnOnce() -> bool,
        ) -> bool {
            run_check_lending_protocol_dependencies(
                single_asset_vault_enabled,
                check_vault_create_extra_features,
            )
        }
    };
}

lending_check_extra_features_wrapper!(run_loan_broker_cover_clawback_check_extra_features);
lending_check_extra_features_wrapper!(run_loan_broker_cover_deposit_check_extra_features);
lending_check_extra_features_wrapper!(run_loan_broker_cover_withdraw_check_extra_features);
lending_check_extra_features_wrapper!(run_loan_broker_delete_check_extra_features);
lending_check_extra_features_wrapper!(run_loan_broker_set_check_extra_features);
lending_check_extra_features_wrapper!(run_loan_delete_check_extra_features);
lending_check_extra_features_wrapper!(run_loan_manage_check_extra_features);
lending_check_extra_features_wrapper!(run_loan_pay_check_extra_features);

pub const fn get_loan_pay_flags_mask() -> u32 {
    LOAN_PAY_FLAGS_MASK
}

pub const fn get_loan_manage_flags_mask() -> u32 {
    LOAN_MANAGE_FLAGS_MASK
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        FULLY_CANONICAL_SIGNATURE_FLAG, INNER_BATCH_TRANSACTION_FLAG, LOAN_DEFAULT_FLAG,
        LOAN_FULL_PAYMENT_FLAG, LOAN_IMPAIR_FLAG, LOAN_LATE_PAYMENT_FLAG, LOAN_MANAGE_FLAGS_MASK,
        LOAN_PAY_FLAGS_MASK, LOAN_PAY_OVERPAYMENT_FLAG, LOAN_UNIMPAIR_FLAG,
        get_loan_manage_flags_mask, get_loan_pay_flags_mask,
        run_loan_broker_cover_clawback_check_extra_features,
        run_loan_broker_cover_deposit_check_extra_features,
        run_loan_broker_cover_withdraw_check_extra_features,
        run_loan_broker_delete_check_extra_features, run_loan_broker_set_check_extra_features,
        run_loan_delete_check_extra_features, run_loan_manage_check_extra_features,
        run_loan_pay_check_extra_features,
    };

    macro_rules! assert_lending_wrapper {
        ($wrapper:ident) => {{
            let helper_called = Cell::new(false);
            let result = $wrapper(false, || {
                helper_called.set(true);
                true
            });
            assert!(!result);
            assert!(!helper_called.get());
            assert!($wrapper(true, || true));
            assert!(!$wrapper(true, || false));
        }};
    }

    #[test]
    fn lending_transactor_check_extra_features_wrappers_delegate() {
        assert_lending_wrapper!(run_loan_broker_cover_clawback_check_extra_features);
        assert_lending_wrapper!(run_loan_broker_cover_deposit_check_extra_features);
        assert_lending_wrapper!(run_loan_broker_cover_withdraw_check_extra_features);
        assert_lending_wrapper!(run_loan_broker_delete_check_extra_features);
        assert_lending_wrapper!(run_loan_broker_set_check_extra_features);
        assert_lending_wrapper!(run_loan_delete_check_extra_features);
        assert_lending_wrapper!(run_loan_manage_check_extra_features);
        assert_lending_wrapper!(run_loan_pay_check_extra_features);
    }

    #[test]
    fn loan_pay_flags_mask_txflags() {
        assert_eq!(LOAN_PAY_OVERPAYMENT_FLAG, 0x0001_0000);
        assert_eq!(LOAN_FULL_PAYMENT_FLAG, 0x0002_0000);
        assert_eq!(LOAN_LATE_PAYMENT_FLAG, 0x0004_0000);
        assert_eq!(LOAN_PAY_FLAGS_MASK, 0x3ff8_ffff);
        assert_eq!(get_loan_pay_flags_mask(), 0x3ff8_ffff);
    }

    #[test]
    fn loan_manage_flags_mask_txflags() {
        assert_eq!(LOAN_DEFAULT_FLAG, 0x0001_0000);
        assert_eq!(LOAN_IMPAIR_FLAG, 0x0002_0000);
        assert_eq!(LOAN_UNIMPAIR_FLAG, 0x0004_0000);
        assert_eq!(LOAN_MANAGE_FLAGS_MASK, 0x3ff8_ffff);
        assert_eq!(get_loan_manage_flags_mask(), 0x3ff8_ffff);
    }

    #[test]
    fn lending_flag_masks_reject_universal_and_tx_specific_bits() {
        assert_eq!(
            get_loan_pay_flags_mask() & FULLY_CANONICAL_SIGNATURE_FLAG,
            0
        );
        assert_eq!(get_loan_pay_flags_mask() & INNER_BATCH_TRANSACTION_FLAG, 0);
        assert_eq!(get_loan_pay_flags_mask() & LOAN_PAY_OVERPAYMENT_FLAG, 0);
        assert_eq!(get_loan_pay_flags_mask() & LOAN_FULL_PAYMENT_FLAG, 0);
        assert_eq!(get_loan_pay_flags_mask() & LOAN_LATE_PAYMENT_FLAG, 0);
        assert_eq!(get_loan_pay_flags_mask() & 0x0008_0000, 0x0008_0000);

        assert_eq!(
            get_loan_manage_flags_mask() & FULLY_CANONICAL_SIGNATURE_FLAG,
            0
        );
        assert_eq!(
            get_loan_manage_flags_mask() & INNER_BATCH_TRANSACTION_FLAG,
            0
        );
        assert_eq!(get_loan_manage_flags_mask() & LOAN_DEFAULT_FLAG, 0);
        assert_eq!(get_loan_manage_flags_mask() & LOAN_IMPAIR_FLAG, 0);
        assert_eq!(get_loan_manage_flags_mask() & LOAN_UNIMPAIR_FLAG, 0);
        assert_eq!(get_loan_manage_flags_mask() & 0x0008_0000, 0x0008_0000);
    }
}
