//! Typed ledger setup helpers for `Ledger::setup()` logic.
//!
//! The caller-visible decision logic lives here. Raw XRPL SLE decoding flows
//! through the dedicated state-map adapter, while decoded fee-field shapes come
//! from the protocol layer so ledger does not duplicate those structs.

use basics::base_uint::Uint256;
use protocol::{DecodedAmountField, DecodedFeeSettingsEntry};

pub type AmountField = DecodedAmountField;
pub type FeeSettingsFields = DecodedFeeSettingsEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentsEntry {
    pub digest: Uint256,
    pub amendments: Vec<Uint256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SetupLookup<T> {
    MissingNode,
    #[default]
    MissingObject,
    Present(T),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerSetupEntries {
    pub amendments: SetupLookup<AmendmentsEntry>,
    pub fees: SetupLookup<FeeSettingsFields>,
}
