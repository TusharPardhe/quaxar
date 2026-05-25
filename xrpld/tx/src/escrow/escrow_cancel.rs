//! Deterministic the reference implementation shells.
//!
//! This ports the current compatibility-safe surface for:
//!
//! - the no-op `preflight(...)`,
//! - the issue and MPT token `preclaim(...)` helper ordering,
//! - the outer `preclaim(...)` wrapper,
//! - and the loaded `doApply()` mutation ordering.

use protocol::{NotTec, Ter, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EscrowCancelIssuePreclaimFacts {
    pub issuer_equals_account: bool,
    pub require_auth_result: Ter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EscrowCancelMptPreclaimFacts {
    pub issuer_equals_account: bool,
    pub issuance_exists: bool,
    pub require_auth_result: Ter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EscrowCancelPreclaimFacts {
    pub token_escrow_enabled: bool,
    pub escrow_exists: bool,
    pub amount_is_xrp: bool,
    pub asset_preclaim_result: Ter,
}

pub trait EscrowCancelApplySink {
    fn escrow_exists(&mut self) -> bool;
    fn token_escrow_enabled(&mut self) -> bool;
    fn cancel_after_present(&mut self) -> bool;
    fn cancel_after_passed(&mut self) -> bool;
    fn remove_owner_dir(&mut self) -> bool;
    fn destination_node_present(&mut self) -> bool;
    fn remove_destination_dir(&mut self) -> bool;
    fn amount_is_xrp(&mut self) -> bool;
    fn credit_owner_xrp(&mut self);
    fn apply_token_unlock(&mut self) -> Ter;
    fn issuer_node_present(&mut self) -> bool;
    fn remove_issuer_dir(&mut self) -> bool;
    fn owner_exists(&mut self) -> bool;
    fn adjust_owner_count(&mut self, delta: i32);
    fn update_owner(&mut self);
    fn erase_escrow(&mut self);
}

pub const fn run_escrow_cancel_preflight() -> NotTec {
    Ter::TES_SUCCESS
}

pub fn run_escrow_cancel_issue_preclaim(facts: EscrowCancelIssuePreclaimFacts) -> Ter {
    if facts.issuer_equals_account {
        return Ter::TEC_INTERNAL;
    }

    facts.require_auth_result
}

pub fn run_escrow_cancel_mpt_preclaim(facts: EscrowCancelMptPreclaimFacts) -> Ter {
    if facts.issuer_equals_account {
        return Ter::TEC_INTERNAL;
    }

    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    facts.require_auth_result
}

pub fn run_escrow_cancel_preclaim(facts: EscrowCancelPreclaimFacts) -> Ter {
    if facts.token_escrow_enabled {
        if !facts.escrow_exists {
            return Ter::TEC_NO_TARGET;
        }

        if !facts.amount_is_xrp && !is_tes_success(facts.asset_preclaim_result) {
            return facts.asset_preclaim_result;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_escrow_cancel_do_apply<S: EscrowCancelApplySink>(sink: &mut S) -> Ter {
    if !sink.escrow_exists() {
        return if sink.token_escrow_enabled() {
            Ter::TEC_INTERNAL
        } else {
            Ter::TEC_NO_TARGET
        };
    }

    if !sink.cancel_after_present() || !sink.cancel_after_passed() {
        return Ter::TEC_NO_PERMISSION;
    }

    if !sink.remove_owner_dir() {
        return Ter::TEF_BAD_LEDGER;
    }

    if sink.destination_node_present() && !sink.remove_destination_dir() {
        return Ter::TEF_BAD_LEDGER;
    }

    if sink.amount_is_xrp() {
        sink.credit_owner_xrp();
    } else {
        if !sink.token_escrow_enabled() {
            return Ter::TEM_DISABLED;
        }

        let token_unlock = sink.apply_token_unlock();
        if !is_tes_success(token_unlock) {
            return token_unlock;
        }

        if sink.issuer_node_present() && !sink.remove_issuer_dir() {
            return Ter::TEF_BAD_LEDGER;
        }
    }

    if !sink.owner_exists() {
        return Ter::TEC_INTERNAL;
    }

    sink.adjust_owner_count(-1);
    sink.update_owner();
    sink.erase_escrow();
    Ter::TES_SUCCESS
}
