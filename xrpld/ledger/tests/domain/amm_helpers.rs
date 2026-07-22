use basics::base_uint::Uint192;
use basics::number::{NumberParts as RuntimeNumber, current_number_one};
use ledger::{
    IsDeposit, adjust_amounts_by_lp_tokens, adjust_asset_in_by_tokens, adjust_asset_out_by_tokens,
    adjust_frac_by_tokens, adjust_lp_tokens, amm_asset_out, amm_lp_tokens, get_rounded_asset,
    get_rounded_lp_tokens, solve_quadratic_eq_smallest, within_relative_distance_amount,
    within_relative_distance_quality,
};
use protocol::{
    Asset, CurrentTransactionRulesGuard, IOUAmount, Issue, MPTAmount, MPTIssue, Quality, Rules,
    STAmount, XRPAmount, currency_from_string, feature_amm, feature_universal_number, fix_ammv1_1,
    fix_ammv1_3, sf_generic,
};

fn sample_issue(currency: &str, fill: u8) -> Issue {
    Issue::new(
        currency_from_string(currency),
        protocol::AccountID::from_array([fill; 20]),
    )
}

fn large_rules(enable_fix_ammv1_1: bool, enable_fix_ammv1_3: bool) -> Rules {
    let mut features = vec![feature_amm(), feature_universal_number()];
    if enable_fix_ammv1_1 {
        features.push(fix_ammv1_1());
    }
    if enable_fix_ammv1_3 {
        features.push(fix_ammv1_3());
    }
    Rules::new(features)
}

#[test]
fn solve_quadratic_eq_smallest_returns_none_for_negative_discriminant() {
    let one = current_number_one();
    assert_eq!(solve_quadratic_eq_smallest(one, one, one), None);
}

#[test]
fn amm_asset_out_ammtokens_vector() {
    let lpt_issue = sample_issue("LPT", 0x61);
    let asset_balance = STAmount::from_xrp_amount(XRPAmount::from_drops(10_000_000_000));
    let lpt_total = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(10_000_000, 0).expect("lpt total"),
        lpt_issue,
    );
    let lp_burn = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(5_000_000, 0).expect("lpt burn"),
        lpt_issue,
    );

    let pre_fix_guard = CurrentTransactionRulesGuard::new(large_rules(false, false));
    let pre_fix = amm_asset_out(&asset_balance, &lpt_total, &lp_burn, 0);
    drop(pre_fix_guard);

    let post_fix_guard = CurrentTransactionRulesGuard::new(large_rules(false, true));
    let post_fix = amm_asset_out(&asset_balance, &lpt_total, &lp_burn, 0);
    drop(post_fix_guard);

    assert_eq!(pre_fix.xrp().drops(), 7_500_000_000);
    assert_eq!(post_fix.xrp().drops(), 7_500_000_000);
}

#[test]
fn within_relative_distance_helpers_match_cpp_tolerance_shape() {
    let issue = sample_issue("USD", 0x62);
    let calc = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(1_000_000, 0).expect("calc"),
        issue,
    );
    let req = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(1_000_001, 0).expect("req"),
        issue,
    );
    let dist = RuntimeNumber::try_from_external_parts(1, -5, basics::number::get_mantissa_scale())
        .expect("distance");
    assert!(within_relative_distance_amount(calc, req, dist));

    let q1 = Quality::from_amounts(&protocol::Amounts::new(
        STAmount::from_xrp_amount(XRPAmount::from_drops(1_000_000_000)),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(2_000_000_000, 0).expect("out"),
            issue,
        ),
    ));
    let q2 = Quality::from_amounts(&protocol::Amounts::new(
        STAmount::from_xrp_amount(XRPAmount::from_drops(1_000_000_000)),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(2_000_000_100, 0).expect("out"),
            issue,
        ),
    ));
    let qdist = RuntimeNumber::try_from_external_parts(1, -6, basics::number::get_mantissa_scale())
        .expect("quality distance");
    assert!(within_relative_distance_quality(q1, q2, qdist));
}

#[test]
fn amm_lp_tokens_keeps_cpp_square_root_invariant() {
    let issue1 = sample_issue("USD", 0x63);
    let issue2 = sample_issue("EUR", 0x64);
    let lpt_issue = sample_issue("LPT", 0x65);
    let asset1 = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(10_000, 0).expect("asset1"),
        issue1,
    );
    let asset2 = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(10_000, 0).expect("asset2"),
        issue2,
    );

    let guard = CurrentTransactionRulesGuard::new(large_rules(false, true));
    let tokens = amm_lp_tokens(&asset1, &asset2, lpt_issue);
    drop(guard);

    assert_eq!(
        tokens.iou(),
        IOUAmount::from_parts(10_000, 0).expect("expected sqrt")
    );
}

#[test]
fn adjust_lp_tokens_and_frac_follow_cpp_rounding_gates() {
    let lpt_issue = sample_issue("LPT", 0x66);
    let lpt_balance = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(9_999_999_999_999_999, 80).expect("lpt balance"),
        lpt_issue,
    );
    let lp_tokens = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(1, 0).expect("lp token"),
        lpt_issue,
    );

    let adjusted_deposit = adjust_lp_tokens(&lpt_balance, &lp_tokens, IsDeposit::Yes);
    let adjusted_withdraw = adjust_lp_tokens(&lpt_balance, &lp_tokens, IsDeposit::No);
    assert_eq!(adjusted_deposit.signum(), 0);
    assert_eq!(adjusted_withdraw.signum(), 0);

    let frac = RuntimeNumber::try_from_external_parts(3, -1, basics::number::get_mantissa_scale())
        .expect("frac");
    let pre_fix = adjust_frac_by_tokens(&large_rules(false, false), &lpt_balance, &lp_tokens, frac);
    let post_fix = adjust_frac_by_tokens(&large_rules(false, true), &lpt_balance, &lp_tokens, frac);
    assert_eq!(pre_fix, frac);
    assert!(post_fix < frac);
}

#[test]
fn rounded_asset_and_lp_tokens_match_cpp_fixammv13_gate() {
    let lpt_issue = sample_issue("LPT", 0x68);
    let balance = STAmount::from_xrp_amount(XRPAmount::from_drops(101));
    let lpt_balance = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(101, 0).expect("lpt balance"),
        lpt_issue,
    );
    let frac = RuntimeNumber::try_from_external_parts(1, -1, basics::number::get_mantissa_scale())
        .expect("frac");

    let pre_fix_asset =
        get_rounded_asset(&large_rules(false, false), &balance, frac, IsDeposit::Yes);
    let post_fix_asset =
        get_rounded_asset(&large_rules(false, true), &balance, frac, IsDeposit::Yes);
    assert_eq!(pre_fix_asset.xrp().drops(), 10);
    assert_eq!(post_fix_asset.xrp().drops(), 11);

    let pre_fix_tokens = get_rounded_lp_tokens(
        &large_rules(false, false),
        &lpt_balance,
        frac,
        IsDeposit::Yes,
    );
    let post_fix_tokens = get_rounded_lp_tokens(
        &large_rules(false, true),
        &lpt_balance,
        frac,
        IsDeposit::Yes,
    );
    assert_eq!(
        pre_fix_tokens.iou(),
        IOUAmount::from_parts(101, -1).expect("pre tokens")
    );
    assert_eq!(
        post_fix_tokens.iou(),
        IOUAmount::from_parts(101, -1).expect("post tokens")
    );
}

#[test]
fn rounded_asset_preserves_mpt_issue_in_every_ammv13_branch() {
    let issue = MPTIssue::new(Uint192::from_array([0x6B; 24]));
    let balance = STAmount::from_mpt_amount(sf_generic(), MPTAmount::from_value(101), issue);
    let frac = RuntimeNumber::try_from_external_parts(1, 0, basics::number::get_mantissa_scale())
        .expect("unit fraction");

    for rules in [large_rules(false, false), large_rules(false, true)] {
        let rounded = get_rounded_asset(&rules, &balance, frac, IsDeposit::No);
        assert_eq!(rounded.asset(), Asset::MPTIssue(issue));
        assert_eq!(rounded.mpt().value(), 101);
    }
}

#[test]
fn token_adjustment_helpers_preserve_cpp_clamping_shape() {
    let issue = sample_issue("USD", 0x69);
    let lpt_issue = sample_issue("LPT", 0x6A);
    let balance = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(10_000, 0).expect("balance"),
        issue,
    );
    let amount = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(5_000, 0).expect("amount"),
        issue,
    );
    let amount2 = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(5_000, 0).expect("amount2"),
        issue,
    );
    let lpt_balance = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(9_999_999_999_999_999, 80).expect("lpt balance"),
        lpt_issue,
    );
    let lp_tokens = STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(1, 0).expect("lp token"),
        lpt_issue,
    );

    let no_fix_guard = CurrentTransactionRulesGuard::new(large_rules(false, false));
    let (adj_amount, adj_amount2, adj_tokens) = adjust_amounts_by_lp_tokens(
        &balance,
        &amount,
        Some(&amount2),
        &lpt_balance,
        &lp_tokens,
        0,
        IsDeposit::Yes,
    );
    drop(no_fix_guard);
    assert_eq!(adj_amount.signum(), 0);
    assert_eq!(adj_amount2.expect("second amount").signum(), 0);
    assert_eq!(adj_tokens.signum(), 0);

    let fix_guard = CurrentTransactionRulesGuard::new(large_rules(true, true));
    let (in_tokens, in_amount) = adjust_asset_in_by_tokens(
        &large_rules(true, true),
        &balance,
        &amount,
        &lpt_balance,
        &lp_tokens,
        0,
    );
    let (out_tokens, out_amount) = adjust_asset_out_by_tokens(
        &large_rules(true, true),
        &balance,
        &amount,
        &lpt_balance,
        &lp_tokens,
        0,
    );
    drop(fix_guard);
    assert!(in_tokens <= lp_tokens);
    assert!(out_tokens <= lp_tokens);
    assert!(in_amount <= amount);
    assert!(out_amount <= amount);
}
