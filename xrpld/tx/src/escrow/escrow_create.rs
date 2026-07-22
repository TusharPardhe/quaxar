//! Deterministic the reference implementation shells.
//!
//! This ports the current compatibility-safe surface for:
//!
//! - `makeTxConsequences(...)`,
//! - the ordered `preflight(...)` validation,
//! - the issue and MPT token `preclaim(...)` helper ordering,
//! - the outer `preclaim(...)` wrapper,
//! - and the loaded `doApply()` mutation ordering.

use crate::TxConsequences;
use crate::consequences::{TxConsequencesShape, build_tx_consequences};
use protocol::{NotTec, SeqProxy, Ter, is_tes_success};

pub const MAX_MPTOKEN_AMOUNT: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscrowCreateAmountKind {
    Xrp,
    Issue,
    Mpt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscrowCreatePreflightFacts {
    pub amount_kind: EscrowCreateAmountKind,
    pub amount_positive: bool,
    pub feature_token_escrow_enabled: bool,
    pub feature_mptokens_enabled: bool,
    pub issue_has_bad_currency: bool,
    pub mpt_amount_within_limit: bool,
    pub cancel_after_present: bool,
    pub finish_after_present: bool,
    pub cancel_after_strictly_after_finish_after: bool,
    pub condition_present: bool,
    pub condition_valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EscrowCreateIssuePreclaimFacts {
    pub issuer_equals_account: bool,
    pub issuer_exists: bool,
    pub issuer_allows_trustline_locking: bool,
    pub trustline_exists: bool,
    pub trustline_balance_sign_valid: bool,
    pub sender_auth_result: Ter,
    pub destination_auth_result: Ter,
    pub sender_frozen: bool,
    pub destination_frozen: bool,
    pub spendable_amount_positive: bool,
    pub spendable_amount_covers_amount: bool,
    pub can_add_amount: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EscrowCreateMptPreclaimFacts {
    pub issuer_equals_account: bool,
    pub issuance_exists: bool,
    pub issuance_can_escrow: bool,
    pub issuance_issuer_matches: bool,
    pub sender_token_exists: bool,
    pub sender_auth_result: Ter,
    pub destination_auth_result: Ter,
    pub sender_locked: bool,
    pub destination_locked: bool,
    pub can_transfer_result: Ter,
    pub spendable_amount_positive: bool,
    pub spendable_amount_covers_amount: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscrowCreatePreclaimFacts {
    pub destination_exists: bool,
    pub destination_is_pseudo_account: bool,
    pub amount_kind: EscrowCreateAmountKind,
    pub token_escrow_enabled: bool,
    pub asset_preclaim_result: Ter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EscrowCreateApplyFacts {
    pub cancel_after_expired: bool,
    pub finish_after_expired: bool,
    pub owner_exists: bool,
    pub reserve_sufficient: bool,
    pub amount_is_xrp: bool,
    pub xrp_balance_covers_amount: bool,
    pub destination_exists: bool,
    pub destination_requires_tag: bool,
    pub destination_tag_present: bool,
    pub include_sequence_field: bool,
    pub should_set_transfer_rate: bool,
    pub destination_is_sender: bool,
    pub issuer_owner_dir_required: bool,
}

pub trait EscrowCreateApplySink {
    fn create_escrow_entry(&mut self);
    fn set_sequence_field(&mut self);
    fn set_transfer_rate(&mut self);
    fn insert_sender_owner_dir(&mut self) -> Option<u64>;
    fn set_sender_owner_node(&mut self, page: u64);
    fn insert_destination_owner_dir(&mut self) -> Option<u64>;
    fn set_destination_owner_node(&mut self, page: u64);
    fn insert_issuer_owner_dir(&mut self) -> Option<u64>;
    fn set_issuer_owner_node(&mut self, page: u64);
    fn deduct_xrp_owner_balance(&mut self);
    fn lock_non_xrp_amount(&mut self) -> Ter;
    fn adjust_owner_count(&mut self, delta: i32);
    fn update_owner(&mut self);
}

pub fn run_escrow_create_make_tx_consequences(
    fee_drops: u64,
    seq_proxy: SeqProxy,
    amount_kind: EscrowCreateAmountKind,
    xrp_amount_drops: u64,
) -> TxConsequences {
    let shape = match amount_kind {
        EscrowCreateAmountKind::Xrp => TxConsequencesShape::PotentialSpend(xrp_amount_drops),
        EscrowCreateAmountKind::Issue | EscrowCreateAmountKind::Mpt => TxConsequencesShape::Normal,
    };

    build_tx_consequences(fee_drops, seq_proxy, shape)
}

pub fn run_escrow_create_preflight(facts: EscrowCreatePreflightFacts) -> NotTec {
    match facts.amount_kind {
        EscrowCreateAmountKind::Xrp => {
            if !facts.amount_positive {
                return Ter::TEM_BAD_AMOUNT;
            }
        }
        EscrowCreateAmountKind::Issue => {
            if !facts.feature_token_escrow_enabled || !facts.amount_positive {
                return Ter::TEM_BAD_AMOUNT;
            }
            if facts.issue_has_bad_currency {
                return Ter::TEM_BAD_CURRENCY;
            }
        }
        EscrowCreateAmountKind::Mpt => {
            if !facts.feature_token_escrow_enabled {
                return Ter::TEM_BAD_AMOUNT;
            }
            if !facts.feature_mptokens_enabled {
                return Ter::TEM_DISABLED;
            }
            if !facts.amount_positive || !facts.mpt_amount_within_limit {
                return Ter::TEM_BAD_AMOUNT;
            }
        }
    }

    if !facts.cancel_after_present && !facts.finish_after_present {
        return Ter::TEM_BAD_EXPIRATION;
    }

    if facts.cancel_after_present
        && facts.finish_after_present
        && !facts.cancel_after_strictly_after_finish_after
    {
        return Ter::TEM_BAD_EXPIRATION;
    }

    if facts.condition_present && !facts.condition_valid {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_escrow_create_issue_preclaim(facts: EscrowCreateIssuePreclaimFacts) -> Ter {
    if facts.issuer_equals_account {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.issuer_exists {
        return Ter::TEC_NO_ISSUER;
    }

    if !facts.issuer_allows_trustline_locking {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.trustline_exists {
        return Ter::TEC_NO_LINE;
    }

    if !facts.trustline_balance_sign_valid {
        return Ter::TEC_NO_PERMISSION;
    }

    if !is_tes_success(facts.sender_auth_result) {
        return facts.sender_auth_result;
    }

    if !is_tes_success(facts.destination_auth_result) {
        return facts.destination_auth_result;
    }

    if facts.sender_frozen || facts.destination_frozen {
        return Ter::TEC_FROZEN;
    }

    if !facts.spendable_amount_positive || !facts.spendable_amount_covers_amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    if !facts.can_add_amount {
        return Ter::TEC_PRECISION_LOSS;
    }

    Ter::TES_SUCCESS
}

pub fn run_escrow_create_mpt_preclaim(facts: EscrowCreateMptPreclaimFacts) -> Ter {
    if facts.issuer_equals_account {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_can_escrow || !facts.issuance_issuer_matches {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.sender_token_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !is_tes_success(facts.sender_auth_result) {
        return facts.sender_auth_result;
    }

    if !is_tes_success(facts.destination_auth_result) {
        return facts.destination_auth_result;
    }

    if facts.sender_locked || facts.destination_locked {
        return Ter::TEC_LOCKED;
    }

    if !is_tes_success(facts.can_transfer_result) {
        return facts.can_transfer_result;
    }

    if !facts.spendable_amount_positive || !facts.spendable_amount_covers_amount {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    Ter::TES_SUCCESS
}

pub fn run_escrow_create_preclaim(facts: EscrowCreatePreclaimFacts) -> Ter {
    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    if facts.destination_is_pseudo_account {
        return Ter::TEC_NO_PERMISSION;
    }

    if !matches!(facts.amount_kind, EscrowCreateAmountKind::Xrp) {
        if !facts.token_escrow_enabled {
            return Ter::TEM_DISABLED;
        }

        if !is_tes_success(facts.asset_preclaim_result) {
            return facts.asset_preclaim_result;
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_escrow_create_do_apply<S: EscrowCreateApplySink>(
    facts: EscrowCreateApplyFacts,
    sink: &mut S,
) -> Ter {
    if facts.cancel_after_expired || facts.finish_after_expired {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.owner_exists {
        return Ter::TEF_INTERNAL;
    }

    if !facts.reserve_sufficient {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    if facts.amount_is_xrp && !facts.xrp_balance_covers_amount {
        return Ter::TEC_UNFUNDED;
    }

    if !facts.destination_exists {
        return Ter::TEC_NO_DST;
    }

    if facts.destination_requires_tag && !facts.destination_tag_present {
        return Ter::TEC_DST_TAG_NEEDED;
    }

    sink.create_escrow_entry();

    if facts.include_sequence_field {
        sink.set_sequence_field();
    }

    if facts.should_set_transfer_rate {
        sink.set_transfer_rate();
    }

    let Some(sender_page) = sink.insert_sender_owner_dir() else {
        return Ter::TEC_DIR_FULL;
    };
    sink.set_sender_owner_node(sender_page);

    if !facts.destination_is_sender {
        let Some(destination_page) = sink.insert_destination_owner_dir() else {
            return Ter::TEC_DIR_FULL;
        };
        sink.set_destination_owner_node(destination_page);
    }

    if facts.issuer_owner_dir_required {
        let Some(issuer_page) = sink.insert_issuer_owner_dir() else {
            return Ter::TEC_DIR_FULL;
        };
        sink.set_issuer_owner_node(issuer_page);
    }

    if facts.amount_is_xrp {
        sink.deduct_xrp_owner_balance();
    } else {
        let lock_result = sink.lock_non_xrp_amount();
        if !is_tes_success(lock_result) {
            return lock_result;
        }
    }

    sink.adjust_owner_count(1);
    sink.update_owner();
    Ter::TES_SUCCESS
}
