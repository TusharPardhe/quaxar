//! Current Rust helpers mirroring the small default static helpers in
//! the reference implementation.
//!
//! This module preserves the exact current default behavior for:
//!
//! - `validDataLength(...)`,
//! - `getFlagsMask(...)`,
//! - `preflightSigValidated(...)`,
//! - and `calculateBaseFee(...)`.

use std::ops::{Add, Mul};

use protocol::{INNER_BATCH_TRANSACTION_FLAG, NotTec, Ter};

pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = 0x8000_0000;
pub const TRANSACTOR_FLAGS_MASK: u32 =
    !(FULLY_CANONICAL_SIGNATURE_FLAG | INNER_BATCH_TRANSACTION_FLAG);

pub const fn run_transactor_valid_data_length(slice_len: Option<usize>, max_length: usize) -> bool {
    match slice_len {
        None => true,
        Some(0) => false,
        Some(len) => len <= max_length,
    }
}

pub const fn run_transactor_get_flags_mask() -> u32 {
    TRANSACTOR_FLAGS_MASK
}

pub const fn run_transactor_preflight_sig_validated() -> NotTec {
    Ter::TES_SUCCESS
}

pub fn run_transactor_calculate_base_fee<Fee>(base_fee: Fee, signer_count: usize) -> Fee
where
    Fee: Copy + Add<Output = Fee> + Mul<u64, Output = Fee>,
{
    base_fee + (base_fee * u64::try_from(signer_count).expect("signer count should fit into u64"))
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{
        FULLY_CANONICAL_SIGNATURE_FLAG, TRANSACTOR_FLAGS_MASK, run_transactor_calculate_base_fee,
        run_transactor_get_flags_mask, run_transactor_preflight_sig_validated,
        run_transactor_valid_data_length,
    };

    #[test]
    fn transactor_valid_data_length_shape() {
        assert!(run_transactor_valid_data_length(None, 256));
        assert!(!run_transactor_valid_data_length(Some(0), 256));
        assert!(run_transactor_valid_data_length(Some(256), 256));
        assert!(!run_transactor_valid_data_length(Some(257), 256));
    }

    #[test]
    fn transactor_get_flags_mask_txflags() {
        assert_eq!(FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
        assert_eq!(TRANSACTOR_FLAGS_MASK, 0x3fff_ffff);
        assert_eq!(run_transactor_get_flags_mask(), 0x3fff_ffff);
    }

    #[test]
    fn transactor_preflight_sig_validated_is_success() {
        assert_eq!(run_transactor_preflight_sig_validated(), Ter::TES_SUCCESS);
    }

    #[test]
    fn transactor_calculate_base_fee_adds_one_base_fee_per_signer() {
        assert_eq!(run_transactor_calculate_base_fee(10_u64, 0), 10);
        assert_eq!(run_transactor_calculate_base_fee(10_u64, 3), 40);
    }
}
