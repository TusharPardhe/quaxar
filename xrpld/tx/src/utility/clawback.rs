//! Narrow the reference implementation top-level preflight and preclaim shells.
//!
//! This ports the deterministic token-versus-MPT branching and the ordered
//! top-level permission/account guards from the the reference implementation transactor without
//! pretending the full ledger mutation path is available here.

use std::cmp::Ordering;

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClawbackAssetKind {
    Issue,
    Mpt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClawbackTrustlineBalanceSign {
    Positive,
    Zero,
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClawbackPreflightFacts {
    pub asset_kind: ClawbackAssetKind,
    pub holder_field_present: bool,
    pub mptokens_v1_enabled: bool,
    pub issuer_equals_holder: bool,
    pub amount_is_xrp: bool,
    pub amount_positive: bool,
    pub mpt_amount_exceeds_max: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClawbackIssuePreclaimFacts {
    pub allow_trustline_clawback: bool,
    pub issuer_no_freeze: bool,
    pub ripple_state_exists: bool,
    pub trustline_balance_sign: ClawbackTrustlineBalanceSign,
    pub issuer_holder_ordering: Ordering,
    pub account_holds_positive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClawbackMptPreclaimFacts {
    pub issuance_exists: bool,
    pub issuance_can_clawback: bool,
    pub issuance_issuer_matches: bool,
    pub holder_token_exists: bool,
    pub account_holds_positive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClawbackPreclaimAssetFacts {
    Issue(ClawbackIssuePreclaimFacts),
    Mpt(ClawbackMptPreclaimFacts),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClawbackPreclaimFacts {
    pub issuer_exists: bool,
    pub holder_exists: bool,
    pub single_asset_vault_enabled: bool,
    pub holder_is_pseudo_account: bool,
    pub holder_is_amm_account: bool,
    pub asset: ClawbackPreclaimAssetFacts,
}

pub const fn run_clawback_preflight(facts: ClawbackPreflightFacts) -> NotTec {
    match facts.asset_kind {
        ClawbackAssetKind::Issue => {
            if facts.holder_field_present {
                return Ter::TEM_MALFORMED;
            }

            if facts.issuer_equals_holder || facts.amount_is_xrp || !facts.amount_positive {
                return Ter::TEM_BAD_AMOUNT;
            }
        }
        ClawbackAssetKind::Mpt => {
            if !facts.mptokens_v1_enabled {
                return Ter::TEM_DISABLED;
            }

            if !facts.holder_field_present || facts.issuer_equals_holder {
                return Ter::TEM_MALFORMED;
            }

            if facts.mpt_amount_exceeds_max || !facts.amount_positive {
                return Ter::TEM_BAD_AMOUNT;
            }
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_clawback_preclaim(facts: ClawbackPreclaimFacts) -> Ter {
    if !facts.issuer_exists || !facts.holder_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if facts.single_asset_vault_enabled && facts.holder_is_pseudo_account {
        return Ter::TEC_PSEUDO_ACCOUNT;
    }

    if facts.holder_is_amm_account {
        return Ter::TEC_AMM_ACCOUNT;
    }

    match facts.asset {
        ClawbackPreclaimAssetFacts::Issue(issue) => {
            if !issue.allow_trustline_clawback || issue.issuer_no_freeze {
                return Ter::TEC_NO_PERMISSION;
            }

            if !issue.ripple_state_exists {
                return Ter::TEC_NO_LINE;
            }

            if issue.trustline_balance_sign == ClawbackTrustlineBalanceSign::Positive
                && issue.issuer_holder_ordering == Ordering::Less
            {
                return Ter::TEC_NO_PERMISSION;
            }

            if issue.trustline_balance_sign == ClawbackTrustlineBalanceSign::Negative
                && issue.issuer_holder_ordering == Ordering::Greater
            {
                return Ter::TEC_NO_PERMISSION;
            }

            if !issue.account_holds_positive {
                return Ter::TEC_INSUFFICIENT_FUNDS;
            }
        }
        ClawbackPreclaimAssetFacts::Mpt(mpt) => {
            if !mpt.issuance_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }

            if !mpt.issuance_can_clawback || !mpt.issuance_issuer_matches {
                return Ter::TEC_NO_PERMISSION;
            }

            if !mpt.holder_token_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }

            if !mpt.account_holds_positive {
                return Ter::TEC_INSUFFICIENT_FUNDS;
            }
        }
    }

    Ter::TES_SUCCESS
}

pub struct ClawbackApplyFacts {
    pub account: protocol::AccountID,
    pub amount: protocol::STAmount,
    pub holder: Option<protocol::AccountID>,
}

pub trait ClawbackApplySink {
    fn clawback_iou(
        &mut self,
        issuer: &protocol::AccountID,
        holder: &protocol::AccountID,
        amount: &protocol::STAmount,
    ) -> Ter;
    fn clawback_mpt(
        &mut self,
        issuer: &protocol::AccountID,
        holder: &protocol::AccountID,
        amount: &protocol::STAmount,
    ) -> Ter;
}

pub fn run_clawback_do_apply<S: ClawbackApplySink>(facts: ClawbackApplyFacts, sink: &mut S) -> Ter {
    if facts.amount.holds_mpt_issue() {
        let holder = facts.holder.ok_or(Ter::TEM_MALFORMED).unwrap(); // Holder must be present for MPT
        sink.clawback_mpt(&facts.account, &holder, &facts.amount)
    } else {
        let holder = facts.amount.issue().account;
        sink.clawback_iou(&facts.account, &holder, &facts.amount)
    }
}
