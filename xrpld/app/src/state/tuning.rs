//! App-main tuning constants mirrored from `Tuning.h`.

use std::time::Duration;

pub const FULL_BELOW_TARGET_SIZE: usize = 524_288;
pub const FULL_BELOW_EXPIRATION: Duration = Duration::from_secs(10 * 60);
pub const MAX_POPPED_TRANSACTIONS: usize = 10;
