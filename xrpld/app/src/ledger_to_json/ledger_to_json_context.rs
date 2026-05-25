//! Narrow app-owned context seam for `xrpld/app/ledger/LedgerToJson.*`.
//!
//! The reference helper pulls API version, validation status, close-time lookup, and
//! owner-funds calculation from `RPC::Context` plus surrounding application
//! owners. Rust does not have that full owner graph yet, so this trait keeps
//! only the compatibility-relevant queries explicit.

use basics::chrono::NetClockTimePoint;
use ledger::Ledger;
use protocol::{AccountID, STAmount};

pub trait LedgerToJsonContext {
    fn api_version(&self) -> u32;

    fn is_validated(&self, ledger: &Ledger) -> bool;

    fn get_close_time_by_seq(&self, ledger_seq: u32) -> Option<NetClockTimePoint>;

    fn account_funds(
        &self,
        _ledger: &Ledger,
        _account: AccountID,
        _amount: &STAmount,
    ) -> Option<String> {
        None
    }
}
