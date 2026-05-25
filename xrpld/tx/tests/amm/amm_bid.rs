//! Integration tests that pin the narrowed Rust `AMMBid.cpp`
//! feature-gate, `preflight(...)`, and `preclaim(...)` shells
//! to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    AUCTION_SLOT_MAX_AUTH_ACCOUNTS, AmmBidPreclaimFacts, AmmBidPreflightFacts,
    AmmBidSlotPricePreclaimFacts, amm_bid_check_extra_features, run_amm_bid_preclaim,
    run_amm_bid_preflight,
};

fn price_facts() -> AmmBidSlotPricePreclaimFacts {
    AmmBidSlotPricePreclaimFacts {
        issue_matches_lp_token: true,
        exceeds_lp_tokens: false,
        reaches_pool_balance: false,
    }
}

#[test]
fn amm_bid_check_extra_features_gate() {
    assert!(amm_bid_check_extra_features(true));
    assert!(!amm_bid_check_extra_features(false));
}

#[test]
fn amm_bid_preflight_passthroughs_invalid_asset_pair_and_price_amount_errors() {
    let invalid_pair = run_amm_bid_preflight(AmmBidPreflightFacts {
        invalid_asset_pair: Some(Ter::TEM_BAD_CURRENCY),
        bid_min_invalid: None,
        bid_max_invalid: None,
        auth_accounts: Vec::<&str>::new(),
        account: "alice",
        fix_amm_v1_3_enabled: false,
    });
    let invalid_min = run_amm_bid_preflight(AmmBidPreflightFacts {
        invalid_asset_pair: None,
        bid_min_invalid: Some(Ter::TEM_BAD_AMOUNT),
        bid_max_invalid: None,
        auth_accounts: Vec::<&str>::new(),
        account: "alice",
        fix_amm_v1_3_enabled: false,
    });
    let invalid_max = run_amm_bid_preflight(AmmBidPreflightFacts {
        invalid_asset_pair: None,
        bid_min_invalid: None,
        bid_max_invalid: Some(Ter::TEM_BAD_AMOUNT),
        auth_accounts: Vec::<&str>::new(),
        account: "alice",
        fix_amm_v1_3_enabled: false,
    });

    assert_eq!(invalid_pair, Ter::TEM_BAD_CURRENCY);
    assert_eq!(invalid_min, Ter::TEM_BAD_AMOUNT);
    assert_eq!(invalid_max, Ter::TEM_BAD_AMOUNT);
}

#[test]
fn amm_bid_preflight_rejects_too_many_auth_accounts() {
    let result = run_amm_bid_preflight(AmmBidPreflightFacts {
        invalid_asset_pair: None,
        bid_min_invalid: None,
        bid_max_invalid: None,
        auth_accounts: vec!["a", "b", "c", "d", "e"],
        account: "owner",
        fix_amm_v1_3_enabled: false,
    });

    assert_eq!(AUCTION_SLOT_MAX_AUTH_ACCOUNTS, 4);
    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(result), "temMALFORMED");
}

#[test]
fn amm_bid_preflight_fix_amm_v1_3_rejects_self_and_duplicates() {
    let duplicate = run_amm_bid_preflight(AmmBidPreflightFacts {
        invalid_asset_pair: None,
        bid_min_invalid: None,
        bid_max_invalid: None,
        auth_accounts: vec!["bob", "carol", "bob"],
        account: "alice",
        fix_amm_v1_3_enabled: true,
    });
    let self_auth = run_amm_bid_preflight(AmmBidPreflightFacts {
        invalid_asset_pair: None,
        bid_min_invalid: None,
        bid_max_invalid: None,
        auth_accounts: vec!["bob", "alice"],
        account: "alice",
        fix_amm_v1_3_enabled: true,
    });
    let legacy = run_amm_bid_preflight(AmmBidPreflightFacts {
        invalid_asset_pair: None,
        bid_min_invalid: None,
        bid_max_invalid: None,
        auth_accounts: vec!["bob", "alice", "bob"],
        account: "alice",
        fix_amm_v1_3_enabled: false,
    });

    assert_eq!(duplicate, Ter::TEM_MALFORMED);
    assert_eq!(self_auth, Ter::TEM_MALFORMED);
    assert_eq!(legacy, Ter::TES_SUCCESS);
}

#[test]
fn amm_bid_preclaim_checks_amm_auth_accounts_and_lp_ownership() {
    let missing_amm = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: false,
        lp_token_balance_is_zero: false,
        auth_accounts_exist: vec![true],
        lp_tokens_is_zero: false,
        bid_min: None,
        bid_max: None,
        bid_min_exceeds_bid_max: false,
    });
    let empty_amm = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: true,
        auth_accounts_exist: vec![true],
        lp_tokens_is_zero: false,
        bid_min: None,
        bid_max: None,
        bid_min_exceeds_bid_max: false,
    });
    let missing_auth = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: false,
        auth_accounts_exist: vec![true, false],
        lp_tokens_is_zero: false,
        bid_min: None,
        bid_max: None,
        bid_min_exceeds_bid_max: false,
    });
    let not_lp = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: false,
        auth_accounts_exist: vec![true],
        lp_tokens_is_zero: true,
        bid_min: None,
        bid_max: None,
        bid_min_exceeds_bid_max: false,
    });

    assert_eq!(missing_amm, Ter::TER_NO_AMM);
    assert_eq!(empty_amm, Ter::TEC_AMM_EMPTY);
    assert_eq!(missing_auth, Ter::TER_NO_ACCOUNT);
    assert_eq!(not_lp, Ter::TEC_AMM_INVALID_TOKENS);
}

#[test]
fn amm_bid_preclaim_rejects_invalid_price_issue_and_bounds() {
    let bad_min_issue = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: false,
        auth_accounts_exist: vec![],
        lp_tokens_is_zero: false,
        bid_min: Some(AmmBidSlotPricePreclaimFacts {
            issue_matches_lp_token: false,
            ..price_facts()
        }),
        bid_max: None,
        bid_min_exceeds_bid_max: false,
    });
    let bad_max_tokens = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: false,
        auth_accounts_exist: vec![],
        lp_tokens_is_zero: false,
        bid_min: None,
        bid_max: Some(AmmBidSlotPricePreclaimFacts {
            exceeds_lp_tokens: true,
            ..price_facts()
        }),
        bid_min_exceeds_bid_max: false,
    });
    let min_gt_max = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: false,
        auth_accounts_exist: vec![],
        lp_tokens_is_zero: false,
        bid_min: Some(price_facts()),
        bid_max: Some(price_facts()),
        bid_min_exceeds_bid_max: true,
    });

    assert_eq!(bad_min_issue, Ter::TEM_BAD_AMM_TOKENS);
    assert_eq!(bad_max_tokens, Ter::TEC_AMM_INVALID_TOKENS);
    assert_eq!(min_gt_max, Ter::TEC_AMM_INVALID_TOKENS);
}

#[test]
fn amm_bid_preclaim_accepts_valid_bid_bounds() {
    let result = run_amm_bid_preclaim(AmmBidPreclaimFacts {
        amm_exists: true,
        lp_token_balance_is_zero: false,
        auth_accounts_exist: vec![true, true],
        lp_tokens_is_zero: false,
        bid_min: Some(price_facts()),
        bid_max: Some(price_facts()),
        bid_min_exceeds_bid_max: false,
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}
