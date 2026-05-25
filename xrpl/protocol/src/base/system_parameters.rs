//! Narrow protocol/system constants from `xrpl/protocol/SystemParameters.h`.

use std::time::Duration;

use crate::{DROPS_PER_XRP, XRPAmount};

pub const INITIAL_XRP: XRPAmount =
    XRPAmount::from_drops(100_000_000_000i64 * DROPS_PER_XRP.drops());
pub const XRP_LEDGER_EARLIEST_SEQ: u32 = 32_570;
pub const XRP_LEDGER_EARLIEST_FEES: u32 = 562_177;
pub const DEFAULT_PEER_PORT: u16 = 2459;
pub const DEFAULT_AMENDMENT_MAJORITY_TIME: Duration = Duration::from_secs(14 * 24 * 60 * 60);
pub const AMENDMENT_MAJORITY_NUMERATOR: u32 = 80;
pub const AMENDMENT_MAJORITY_DENOMINATOR: u32 = 100;

pub fn system_name() -> &'static str {
    "xrpld"
}

pub fn is_legal_amount(amount: XRPAmount) -> bool {
    amount <= INITIAL_XRP
}

pub fn is_legal_amount_signed(amount: XRPAmount) -> bool {
    amount >= -INITIAL_XRP && amount <= INITIAL_XRP
}

pub fn system_currency_code() -> &'static str {
    "XRP"
}
