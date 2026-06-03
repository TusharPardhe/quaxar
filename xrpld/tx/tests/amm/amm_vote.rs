//! Tests for `AMMVote` parity helpers.

use protocol::{AccountID, Asset, TRADING_FEE_THRESHOLD, Ter, VOTE_MAX_SLOTS};
use tx::{
    AMMVoteApplyFacts, AMMVotePreclaimFacts, AMMVotePreflightFacts, AMMVoteSlot,
    amm_vote_check_extra_features, run_amm_vote_apply_facts, run_amm_vote_preclaim_facts,
    run_amm_vote_preflight_facts,
};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn mpt_asset(fill: u8) -> Asset {
    Asset::from(protocol::MPTIssue::new(protocol::make_mpt_id(
        u32::from(fill),
        account(fill),
    )))
}

#[test]
fn amm_vote_check_extra_features_matches_cpp_mpt_gate() {
    assert!(amm_vote_check_extra_features(true, false, false, false));
    assert!(amm_vote_check_extra_features(true, true, true, false));
    assert!(!amm_vote_check_extra_features(true, false, true, false));
    assert!(!amm_vote_check_extra_features(true, false, false, true));
    assert!(!amm_vote_check_extra_features(false, true, false, false));
    assert!(matches!(mpt_asset(7), Asset::MPTIssue(_)));
}

#[test]
fn amm_vote_preflight_preserves_reference_error_ordering() {
    assert_eq!(
        run_amm_vote_preflight_facts(AMMVotePreflightFacts {
            asset_pair_invalid: Some(Ter::TEM_BAD_CURRENCY),
            trading_fee: TRADING_FEE_THRESHOLD + 1,
        }),
        Ter::TEM_BAD_CURRENCY
    );

    assert_eq!(
        run_amm_vote_preflight_facts(AMMVotePreflightFacts {
            asset_pair_invalid: None,
            trading_fee: TRADING_FEE_THRESHOLD + 1,
        }),
        Ter::TEM_BAD_FEE
    );

    assert_eq!(
        run_amm_vote_preflight_facts(AMMVotePreflightFacts {
            asset_pair_invalid: None,
            trading_fee: TRADING_FEE_THRESHOLD,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn amm_vote_apply_updates_existing_slot_and_weighted_fee() {
    let result = run_amm_vote_apply_facts(&AMMVoteApplyFacts {
        account: account(1),
        asset1: protocol::xrp_issue().into(),
        asset2: protocol::Issue::new(protocol::Currency::from_array([1; 20]), account(9)).into(),
        trading_fee: 90,
        lp_token_balance: 1_000,
        account_lp_tokens: 400,
        vote_slots: vec![
            AMMVoteSlot {
                account: account(1),
                trading_fee: 10,
                lp_tokens: 300,
            },
            AMMVoteSlot {
                account: account(2),
                trading_fee: 30,
                lp_tokens: 600,
            },
        ],
    });

    assert_eq!(result.vote_slots.len(), 2);
    assert_eq!(result.vote_slots[0].account, account(1));
    assert_eq!(result.vote_slots[0].trading_fee, Some(90));
    assert_eq!(result.vote_slots[0].vote_weight, 40_000);
    assert_eq!(result.vote_slots[1].vote_weight, 60_000);
    assert_eq!(result.trading_fee, Some(54));
    assert_eq!(result.discounted_fee, Some(5));
}

#[test]
fn amm_vote_apply_adds_new_slot_until_max_slots() {
    let result = run_amm_vote_apply_facts(&AMMVoteApplyFacts {
        account: account(3),
        asset1: protocol::xrp_issue().into(),
        asset2: protocol::Issue::new(protocol::Currency::from_array([1; 20]), account(9)).into(),
        trading_fee: 20,
        lp_token_balance: 1_000,
        account_lp_tokens: 250,
        vote_slots: vec![AMMVoteSlot {
            account: account(1),
            trading_fee: 10,
            lp_tokens: 250,
        }],
    });

    assert_eq!(result.vote_slots.len(), 2);
    assert_eq!(result.vote_slots[1].account, account(3));
    assert_eq!(result.vote_slots[1].vote_weight, 25_000);
    assert_eq!(result.trading_fee, Some(15));
    assert_eq!(result.discounted_fee, Some(1));
}

#[test]
fn amm_vote_apply_replaces_min_slot_when_full_and_new_vote_has_more_tokens() {
    let mut vote_slots = Vec::new();
    for idx in 0..VOTE_MAX_SLOTS {
        vote_slots.push(AMMVoteSlot {
            account: account(idx as u8 + 1),
            trading_fee: 10 + idx,
            lp_tokens: 100 + u128::from(idx),
        });
    }

    let result = run_amm_vote_apply_facts(&AMMVoteApplyFacts {
        account: account(99),
        asset1: protocol::xrp_issue().into(),
        asset2: protocol::Issue::new(protocol::Currency::from_array([1; 20]), account(9)).into(),
        trading_fee: 50,
        lp_token_balance: 2_000,
        account_lp_tokens: 150,
        vote_slots,
    });

    assert_eq!(result.vote_slots.len(), usize::from(VOTE_MAX_SLOTS));
    assert_eq!(result.vote_slots[0].account, account(99));
    assert_eq!(result.vote_slots[0].trading_fee, Some(50));
}

#[test]
fn amm_vote_apply_keeps_full_slots_when_new_vote_cannot_replace_min() {
    let mut vote_slots = Vec::new();
    for idx in 0..VOTE_MAX_SLOTS {
        vote_slots.push(AMMVoteSlot {
            account: account(idx as u8 + 1),
            trading_fee: 10,
            lp_tokens: 100,
        });
    }

    let result = run_amm_vote_apply_facts(&AMMVoteApplyFacts {
        account: account(99),
        asset1: protocol::xrp_issue().into(),
        asset2: protocol::Issue::new(protocol::Currency::from_array([1; 20]), account(9)).into(),
        trading_fee: 9,
        lp_token_balance: 1_000,
        account_lp_tokens: 100,
        vote_slots: vote_slots.clone(),
    });

    assert_eq!(result.vote_slots.len(), usize::from(VOTE_MAX_SLOTS));
    assert!(
        result
            .vote_slots
            .iter()
            .all(|slot| slot.account != account(99))
    );
}

#[test]
fn amm_vote_apply_omits_zero_fee_fields_and_removes_weighted_fee_when_denominator_zero() {
    let result = run_amm_vote_apply_facts(&AMMVoteApplyFacts {
        account: account(1),
        asset1: protocol::xrp_issue().into(),
        asset2: protocol::Issue::new(protocol::Currency::from_array([1; 20]), account(9)).into(),
        trading_fee: 0,
        lp_token_balance: 1_000,
        account_lp_tokens: 0,
        vote_slots: vec![AMMVoteSlot {
            account: account(2),
            trading_fee: 30,
            lp_tokens: 0,
        }],
    });

    assert!(result.vote_slots.is_empty());
    assert_eq!(result.trading_fee, None);
    assert_eq!(result.discounted_fee, None);
}

#[test]
fn amm_vote_preclaim_matches_reference_ordering() {
    assert_eq!(
        run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
            amm_exists: false,
            lp_token_balance_signum: 0,
            account_lp_holds_signum: None,
        }),
        Ter::TER_NO_AMM
    );

    assert_eq!(
        run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
            amm_exists: true,
            lp_token_balance_signum: 0,
            account_lp_holds_signum: Some(1),
        }),
        Ter::TEC_AMM_EMPTY
    );

    assert_eq!(
        run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
            amm_exists: true,
            lp_token_balance_signum: 1,
            account_lp_holds_signum: None,
        }),
        Ter::TEC_AMM_INVALID_TOKENS
    );

    assert_eq!(
        run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
            amm_exists: true,
            lp_token_balance_signum: 1,
            account_lp_holds_signum: Some(0),
        }),
        Ter::TEC_AMM_INVALID_TOKENS
    );

    assert_eq!(
        run_amm_vote_preclaim_facts(AMMVotePreclaimFacts {
            amm_exists: true,
            lp_token_balance_signum: 1,
            account_lp_holds_signum: Some(1),
        }),
        Ter::TES_SUCCESS
    );
}
