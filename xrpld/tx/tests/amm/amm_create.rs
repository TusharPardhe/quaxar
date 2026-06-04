//! Tests for `AMMCreate` parity helpers.

use protocol::{AccountID, Asset, Currency, Issue, TRADING_FEE_THRESHOLD, Ter};
use tx::{
    AMMCreatePreclaimFacts, AMMCreatePreflightFacts, amm_create_check_extra_features,
    run_amm_create_preclaim_facts, run_amm_create_preflight_facts,
};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn asset(fill: u8) -> Asset {
    Asset::from(Issue::new(Currency::from_array([fill; 20]), account(fill)))
}

fn mpt_asset(fill: u8) -> Asset {
    Asset::from(protocol::MPTIssue::new(protocol::make_mpt_id(
        u32::from(fill),
        account(fill),
    )))
}

#[test]
fn amm_create_check_extra_features_matches_cpp_mpt_gate() {
    assert!(amm_create_check_extra_features(true, false, false, false));
    assert!(amm_create_check_extra_features(true, true, true, false));
    assert!(!amm_create_check_extra_features(true, false, true, false));
    assert!(!amm_create_check_extra_features(true, false, false, true));
    assert!(!amm_create_check_extra_features(false, true, false, false));
    assert!(matches!(mpt_asset(7), Asset::MPTIssue(_)));
}

#[test]
fn amm_create_preflight_preserves_reference_ordering() {
    assert_eq!(
        run_amm_create_preflight_facts(AMMCreatePreflightFacts {
            amount_asset: asset(1),
            amount_invalid: Some(Ter::TEM_BAD_AMOUNT),
            amount2_asset: asset(1),
            amount2_invalid: None,
            trading_fee: TRADING_FEE_THRESHOLD + 1,
        }),
        Ter::TEM_BAD_AMM_TOKENS
    );

    assert_eq!(
        run_amm_create_preflight_facts(AMMCreatePreflightFacts {
            amount_asset: asset(1),
            amount_invalid: Some(Ter::TEM_BAD_AMOUNT),
            amount2_asset: asset(2),
            amount2_invalid: Some(Ter::TEM_BAD_CURRENCY),
            trading_fee: TRADING_FEE_THRESHOLD + 1,
        }),
        Ter::TEM_BAD_AMOUNT
    );

    assert_eq!(
        run_amm_create_preflight_facts(AMMCreatePreflightFacts {
            amount_asset: asset(1),
            amount_invalid: None,
            amount2_asset: asset(2),
            amount2_invalid: Some(Ter::TEM_BAD_CURRENCY),
            trading_fee: TRADING_FEE_THRESHOLD + 1,
        }),
        Ter::TEM_BAD_CURRENCY
    );

    assert_eq!(
        run_amm_create_preflight_facts(AMMCreatePreflightFacts {
            amount_asset: asset(1),
            amount_invalid: None,
            amount2_asset: asset(2),
            amount2_invalid: None,
            trading_fee: TRADING_FEE_THRESHOLD + 1,
        }),
        Ter::TEM_BAD_FEE
    );

    assert_eq!(
        run_amm_create_preflight_facts(AMMCreatePreflightFacts {
            amount_asset: asset(1),
            amount_invalid: None,
            amount2_asset: asset(2),
            amount2_invalid: None,
            trading_fee: TRADING_FEE_THRESHOLD,
        }),
        Ter::TES_SUCCESS
    );
}

#[test]
fn amm_create_preclaim_preserves_reference_ordering() {
    let mut facts = AMMCreatePreclaimFacts {
        amm_exists: true,
        amount_auth_result: Ter::TEC_NO_AUTH,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_DUPLICATE);

    facts = AMMCreatePreclaimFacts {
        amount_auth_result: Ter::TEC_NO_AUTH,
        amount2_auth_result: Ter::TEC_LOCKED,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_NO_AUTH);

    facts = AMMCreatePreclaimFacts {
        amount2_auth_result: Ter::TEC_NO_AUTH,
        amount_frozen_result: Ter::TEC_LOCKED,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_NO_AUTH);

    facts = AMMCreatePreclaimFacts {
        amount_frozen_result: Ter::TEC_LOCKED,
        amount2_frozen_result: Ter::TEC_NO_AUTH,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_LOCKED);

    facts = AMMCreatePreclaimFacts {
        amount2_frozen_result: Ter::TEC_LOCKED,
        amount_no_default_ripple: true,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_LOCKED);

    facts = AMMCreatePreclaimFacts {
        amount_no_default_ripple: true,
        xrp_reserve_positive: false,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TER_NO_RIPPLE);

    facts = AMMCreatePreclaimFacts {
        xrp_reserve_positive: false,
        amount_insufficient_balance: true,
        ..Default::default()
    };
    assert_eq!(
        run_amm_create_preclaim_facts(facts),
        Ter::TEC_INSUF_RESERVE_LINE
    );

    facts = AMMCreatePreclaimFacts {
        amount2_insufficient_balance: true,
        amount_is_lp_token: true,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_UNFUNDED_AMM);

    facts = AMMCreatePreclaimFacts {
        amount_is_lp_token: true,
        address_collision: true,
        ..Default::default()
    };
    assert_eq!(
        run_amm_create_preclaim_facts(facts),
        Ter::TEC_AMM_INVALID_TOKENS
    );

    facts = AMMCreatePreclaimFacts {
        address_collision: true,
        amount_mpt_trade_transfer_result: Ter::TEC_LOCKED,
        ..Default::default()
    };
    assert_eq!(
        run_amm_create_preclaim_facts(facts),
        Ter::TER_ADDRESS_COLLISION
    );

    facts = AMMCreatePreclaimFacts {
        amount_mpt_trade_transfer_result: Ter::TEC_LOCKED,
        amount2_mpt_trade_transfer_result: Ter::TEC_NO_AUTH,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_LOCKED);

    facts = AMMCreatePreclaimFacts {
        amount2_mpt_trade_transfer_result: Ter::TEC_NO_AUTH,
        amount_clawback_disabled_result: Ter::TEC_NO_PERMISSION,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_NO_AUTH);

    facts = AMMCreatePreclaimFacts {
        amount_clawback_disabled_result: Ter::TEC_NO_PERMISSION,
        amount2_clawback_disabled_result: Ter::TEC_LOCKED,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TEC_NO_PERMISSION);

    facts = AMMCreatePreclaimFacts {
        amm_clawback_enabled: true,
        amount_clawback_disabled_result: Ter::TEC_NO_PERMISSION,
        ..Default::default()
    };
    assert_eq!(run_amm_create_preclaim_facts(facts), Ter::TES_SUCCESS);

    assert_eq!(
        run_amm_create_preclaim_facts(AMMCreatePreclaimFacts::default()),
        Ter::TES_SUCCESS
    );
}
