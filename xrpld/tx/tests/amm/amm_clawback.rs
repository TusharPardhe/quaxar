//! Tests for `AMMClawback` parity helpers.

use protocol::Ter;
use tx::{
    AMMClawbackPreclaimFacts, AMMClawbackPreflightFacts, run_amm_clawback_check_extra_features,
    run_amm_clawback_preclaim_facts, run_amm_clawback_preflight_facts,
};

fn preflight_facts() -> AMMClawbackPreflightFacts {
    AMMClawbackPreflightFacts {
        issuer_equals_holder: false,
        asset_is_xrp: false,
        claw_two_assets: false,
        asset_issuer_matches_asset2_issuer: true,
        asset_issuer_matches_account: true,
        claw_amount_asset_matches_asset: None,
        claw_amount_signum: None,
    }
}

fn preclaim_facts() -> AMMClawbackPreclaimFacts {
    AMMClawbackPreclaimFacts {
        issuer_exists: true,
        holder_exists: true,
        amm_exists: true,
        mptokens_v2_enabled: false,
        issuer_allows_trustline_clawback: true,
        issuer_no_freeze: false,
        asset_claw_allowed: true,
        claw_two_assets: false,
        asset2_claw_allowed: true,
    }
}

#[test]
fn amm_clawback_check_extra_features_matches_reference_gate() {
    assert!(!run_amm_clawback_check_extra_features(
        false, true, false, false, false
    ));
    assert!(run_amm_clawback_check_extra_features(
        true, false, false, false, false
    ));
    assert!(!run_amm_clawback_check_extra_features(
        true, false, true, false, false
    ));
    assert!(!run_amm_clawback_check_extra_features(
        true, false, false, true, false
    ));
    assert!(!run_amm_clawback_check_extra_features(
        true, false, false, false, true
    ));
    assert!(run_amm_clawback_check_extra_features(
        true, true, true, true, true
    ));
}

#[test]
fn amm_clawback_preflight_preserves_reference_ordering() {
    let mut facts = preflight_facts();
    facts.issuer_equals_holder = true;
    facts.asset_is_xrp = true;
    assert_eq!(run_amm_clawback_preflight_facts(facts), Ter::TEM_MALFORMED);

    let mut facts = preflight_facts();
    facts.asset_is_xrp = true;
    facts.claw_two_assets = true;
    facts.asset_issuer_matches_asset2_issuer = false;
    assert_eq!(run_amm_clawback_preflight_facts(facts), Ter::TEM_MALFORMED);

    let mut facts = preflight_facts();
    facts.claw_two_assets = true;
    facts.asset_issuer_matches_asset2_issuer = false;
    facts.asset_issuer_matches_account = false;
    assert_eq!(
        run_amm_clawback_preflight_facts(facts),
        Ter::TEM_INVALID_FLAG
    );

    let mut facts = preflight_facts();
    facts.asset_issuer_matches_account = false;
    facts.claw_amount_asset_matches_asset = Some(false);
    assert_eq!(run_amm_clawback_preflight_facts(facts), Ter::TEM_MALFORMED);

    let mut facts = preflight_facts();
    facts.claw_amount_asset_matches_asset = Some(false);
    facts.claw_amount_signum = Some(0);
    assert_eq!(run_amm_clawback_preflight_facts(facts), Ter::TEM_BAD_AMOUNT);

    let mut facts = preflight_facts();
    facts.claw_amount_asset_matches_asset = Some(true);
    facts.claw_amount_signum = Some(0);
    assert_eq!(run_amm_clawback_preflight_facts(facts), Ter::TEM_BAD_AMOUNT);

    let mut facts = preflight_facts();
    facts.claw_amount_asset_matches_asset = Some(true);
    facts.claw_amount_signum = Some(1);
    assert_eq!(run_amm_clawback_preflight_facts(facts), Ter::TES_SUCCESS);
}

#[test]
fn amm_clawback_preclaim_preserves_reference_ordering() {
    let mut facts = preclaim_facts();
    facts.issuer_exists = false;
    facts.holder_exists = false;
    assert_eq!(run_amm_clawback_preclaim_facts(facts), Ter::TER_NO_ACCOUNT);

    let mut facts = preclaim_facts();
    facts.holder_exists = false;
    facts.amm_exists = false;
    assert_eq!(run_amm_clawback_preclaim_facts(facts), Ter::TER_NO_ACCOUNT);

    let mut facts = preclaim_facts();
    facts.amm_exists = false;
    facts.issuer_allows_trustline_clawback = false;
    assert_eq!(run_amm_clawback_preclaim_facts(facts), Ter::TER_NO_AMM);

    let mut facts = preclaim_facts();
    facts.issuer_allows_trustline_clawback = false;
    facts.asset_claw_allowed = true;
    assert_eq!(
        run_amm_clawback_preclaim_facts(facts),
        Ter::TEC_NO_PERMISSION
    );

    let mut facts = preclaim_facts();
    facts.issuer_no_freeze = true;
    assert_eq!(
        run_amm_clawback_preclaim_facts(facts),
        Ter::TEC_NO_PERMISSION
    );

    let mut facts = preclaim_facts();
    facts.mptokens_v2_enabled = true;
    facts.issuer_allows_trustline_clawback = false;
    facts.asset_claw_allowed = false;
    assert_eq!(
        run_amm_clawback_preclaim_facts(facts),
        Ter::TEC_NO_PERMISSION
    );

    let mut facts = preclaim_facts();
    facts.claw_two_assets = true;
    facts.asset2_claw_allowed = false;
    assert_eq!(
        run_amm_clawback_preclaim_facts(facts),
        Ter::TEC_NO_PERMISSION
    );

    assert_eq!(
        run_amm_clawback_preclaim_facts(preclaim_facts()),
        Ter::TES_SUCCESS
    );
}
