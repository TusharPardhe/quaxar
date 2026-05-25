//! Deterministic the reference implementation shells.
//!
//! This ports the current compatibility-safe surface for:
//!
//! - trivial `preflight(...)`,
//! - ordered `preclaim(...)` existence / issuer / obligations checks,
//! - and the loaded `doApply()` owner-dir removal and owner-count update.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MPTokenIssuanceDestroyPreclaimFacts {
    pub issuance_exists: bool,
    pub issuer_matches: bool,
    pub outstanding_amount_is_zero: bool,
    pub locked_amount_is_zero: bool,
}

pub trait MPTokenIssuanceDestroyApplySink {
    fn loaded_issuance_exists(&mut self) -> bool;
    fn account_matches_loaded_issuer(&mut self) -> bool;
    fn remove_from_owner_dir(&mut self) -> bool;
    fn erase_issuance(&mut self);
    fn adjust_owner_count(&mut self, delta: i32);
}

pub const fn run_mp_token_issuance_destroy_preflight() -> NotTec {
    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_destroy_preclaim(facts: MPTokenIssuanceDestroyPreclaimFacts) -> Ter {
    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuer_matches {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.outstanding_amount_is_zero || !facts.locked_amount_is_zero {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    Ter::TES_SUCCESS
}

pub fn run_mp_token_issuance_destroy_do_apply<S: MPTokenIssuanceDestroyApplySink>(
    sink: &mut S,
) -> Ter {
    if !sink.loaded_issuance_exists() {
        return Ter::TEC_INTERNAL;
    }

    if !sink.account_matches_loaded_issuer() {
        return Ter::TEC_INTERNAL;
    }

    if !sink.remove_from_owner_dir() {
        return Ter::TEF_BAD_LEDGER;
    }

    sink.erase_issuance();
    sink.adjust_owner_count(-1);
    Ter::TES_SUCCESS
}
