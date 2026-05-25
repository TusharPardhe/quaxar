//! Narrow the reference implementation top-level feature gate, `preflight(...)`, and
//! `preclaim(...)` shells.
//!
//! This ports the deterministic guard ordering above the still-missing AMM
//! apply/runtime substrate.

use std::collections::BTreeSet;

use protocol::{NotTec, Ter};

pub const AUCTION_SLOT_MAX_AUTH_ACCOUNTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmmBidPreflightFacts<AccountId> {
    pub invalid_asset_pair: Option<NotTec>,
    pub bid_min_invalid: Option<NotTec>,
    pub bid_max_invalid: Option<NotTec>,
    pub auth_accounts: Vec<AccountId>,
    pub account: AccountId,
    pub fix_amm_v1_3_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmmBidSlotPricePreclaimFacts {
    pub issue_matches_lp_token: bool,
    pub exceeds_lp_tokens: bool,
    pub reaches_pool_balance: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmmBidPreclaimFacts {
    pub amm_exists: bool,
    pub lp_token_balance_is_zero: bool,
    pub auth_accounts_exist: Vec<bool>,
    pub lp_tokens_is_zero: bool,
    pub bid_min: Option<AmmBidSlotPricePreclaimFacts>,
    pub bid_max: Option<AmmBidSlotPricePreclaimFacts>,
    pub bid_min_exceeds_bid_max: bool,
}

pub const fn amm_bid_check_extra_features(amm_enabled: bool) -> bool {
    amm_enabled
}

pub fn run_amm_bid_preflight<AccountId>(facts: AmmBidPreflightFacts<AccountId>) -> NotTec
where
    AccountId: Clone + Ord,
{
    let AmmBidPreflightFacts {
        invalid_asset_pair,
        bid_min_invalid,
        bid_max_invalid,
        auth_accounts,
        account,
        fix_amm_v1_3_enabled,
    } = facts;

    if let Some(err) = invalid_asset_pair {
        return err;
    }

    if let Some(err) = bid_min_invalid {
        return err;
    }

    if let Some(err) = bid_max_invalid {
        return err;
    }

    if auth_accounts.len() > AUCTION_SLOT_MAX_AUTH_ACCOUNTS {
        return Ter::TEM_MALFORMED;
    }

    if fix_amm_v1_3_enabled {
        let mut unique = BTreeSet::new();
        for auth_account in auth_accounts {
            if auth_account == account || !unique.insert(auth_account) {
                return Ter::TEM_MALFORMED;
            }
        }
    }

    Ter::TES_SUCCESS
}

pub fn run_amm_bid_preclaim(facts: AmmBidPreclaimFacts) -> Ter {
    if !facts.amm_exists {
        return Ter::TER_NO_AMM;
    }

    if facts.lp_token_balance_is_zero {
        return Ter::TEC_AMM_EMPTY;
    }

    let mut idx = 0;
    while idx < facts.auth_accounts_exist.len() {
        if !facts.auth_accounts_exist[idx] {
            return Ter::TER_NO_ACCOUNT;
        }
        idx += 1;
    }

    if facts.lp_tokens_is_zero {
        return Ter::TEC_AMM_INVALID_TOKENS;
    }

    if let Some(bid_min) = facts.bid_min {
        if !bid_min.issue_matches_lp_token {
            return Ter::TEM_BAD_AMM_TOKENS;
        }
        if bid_min.exceeds_lp_tokens || bid_min.reaches_pool_balance {
            return Ter::TEC_AMM_INVALID_TOKENS;
        }
    }

    if let Some(bid_max) = facts.bid_max {
        if !bid_max.issue_matches_lp_token {
            return Ter::TEM_BAD_AMM_TOKENS;
        }
        if bid_max.exceeds_lp_tokens || bid_max.reaches_pool_balance {
            return Ter::TEC_AMM_INVALID_TOKENS;
        }
    }

    if facts.bid_min.is_some() && facts.bid_max.is_some() && facts.bid_min_exceeds_bid_max {
        return Ter::TEC_AMM_INVALID_TOKENS;
    }

    Ter::TES_SUCCESS
}
