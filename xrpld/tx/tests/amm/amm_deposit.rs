//! Tests for `AMMDeposit::preflight` parity helpers.

use protocol::{
    AMM_LIMIT_LP_TOKEN_FLAG, AMM_LP_TOKEN_FLAG, AMM_ONE_ASSET_LP_TOKEN_FLAG, AMM_SINGLE_ASSET_FLAG,
    AMM_TWO_ASSET_FLAG, AMM_TWO_ASSET_IF_EMPTY_FLAG, AccountID, Asset, Currency, IOUAmount, Issue,
    LedgerEntryType, Rules, STAmount, STLedgerEntry, TRADING_FEE_THRESHOLD, Ter,
    get_field_by_symbol,
};
use tx::{
    AMMDepositApplyFacts, AMMDepositApplyMathFacts, AMMDepositApplySink, AMMDepositPreclaimFacts,
    AMMDepositPreflightFacts, amm_deposit_check_extra_features, run_amm_deposit_apply_math_facts,
    run_amm_deposit_do_apply, run_amm_deposit_preclaim_facts, run_amm_deposit_preflight_facts,
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
fn amm_deposit_check_extra_features_matches_cpp_mpt_gate() {
    assert!(amm_deposit_check_extra_features(
        true, false, false, false, false, false
    ));
    assert!(amm_deposit_check_extra_features(
        true, true, true, false, false, true
    ));
    assert!(!amm_deposit_check_extra_features(
        true, false, true, false, false, false
    ));
    assert!(!amm_deposit_check_extra_features(
        true, false, false, true, false, false
    ));
    assert!(!amm_deposit_check_extra_features(
        true, false, false, false, true, false
    ));
    assert!(!amm_deposit_check_extra_features(
        true, false, false, false, false, true
    ));
    assert!(!amm_deposit_check_extra_features(
        false, true, false, false, false, false
    ));
    assert!(matches!(mpt_asset(7), Asset::MPTIssue(_)));
}

fn issue(fill: u8) -> Issue {
    Issue::new(Currency::from_array([fill; 20]), account(fill))
}

fn iou(value: i64, issue: Issue) -> STAmount {
    STAmount::from_iou_amount(
        get_field_by_symbol("sfAmount"),
        IOUAmount::from_parts(value, 0).expect("test IOU amount"),
        issue,
    )
}

fn lp(value: i64, issue: Issue) -> STAmount {
    STAmount::from_iou_amount(
        get_field_by_symbol("sfLPTokenBalance"),
        IOUAmount::from_parts(value, 0).expect("test LP amount"),
        issue,
    )
}

fn facts(flags: u32) -> AMMDepositPreflightFacts {
    AMMDepositPreflightFacts {
        flags,
        asset_pair_invalid: None,
        amount: None,
        amount_invalid: None,
        amount2: None,
        amount2_invalid: None,
        e_price: None,
        e_price_invalid: None,
        lp_token_out_signum: None,
        trading_fee: None,
    }
}

#[test]
fn amm_deposit_preflight_rejects_missing_or_multiple_sub_tx_flags() {
    assert_eq!(
        run_amm_deposit_preflight_facts(facts(0)),
        Ter::TEM_MALFORMED
    );
    assert_eq!(
        run_amm_deposit_preflight_facts(facts(AMM_SINGLE_ASSET_FLAG | AMM_TWO_ASSET_FLAG)),
        Ter::TEM_MALFORMED
    );
}

#[test]
fn amm_deposit_preflight_validates_mode_field_matrix() {
    let amount = asset(1);
    let amount2 = asset(2);

    let mut lp_token = facts(AMM_LP_TOKEN_FLAG);
    lp_token.lp_token_out_signum = Some(1);
    assert_eq!(run_amm_deposit_preflight_facts(lp_token), Ter::TES_SUCCESS);

    let mut lp_token_missing_pair_side = facts(AMM_LP_TOKEN_FLAG);
    lp_token_missing_pair_side.lp_token_out_signum = Some(1);
    lp_token_missing_pair_side.amount = Some(amount);
    assert_eq!(
        run_amm_deposit_preflight_facts(lp_token_missing_pair_side),
        Ter::TEM_MALFORMED
    );

    let mut single = facts(AMM_SINGLE_ASSET_FLAG);
    single.amount = Some(amount);
    assert_eq!(run_amm_deposit_preflight_facts(single), Ter::TES_SUCCESS);

    let mut single_with_amount2 = facts(AMM_SINGLE_ASSET_FLAG);
    single_with_amount2.amount = Some(amount);
    single_with_amount2.amount2 = Some(amount2);
    assert_eq!(
        run_amm_deposit_preflight_facts(single_with_amount2),
        Ter::TEM_MALFORMED
    );

    let mut two = facts(AMM_TWO_ASSET_FLAG);
    two.amount = Some(amount);
    two.amount2 = Some(amount2);
    assert_eq!(run_amm_deposit_preflight_facts(two), Ter::TES_SUCCESS);

    let mut one_asset_lp = facts(AMM_ONE_ASSET_LP_TOKEN_FLAG);
    one_asset_lp.amount = Some(amount);
    one_asset_lp.lp_token_out_signum = Some(1);
    assert_eq!(
        run_amm_deposit_preflight_facts(one_asset_lp),
        Ter::TES_SUCCESS
    );

    let mut limit_lp = facts(AMM_LIMIT_LP_TOKEN_FLAG);
    limit_lp.amount = Some(amount);
    limit_lp.e_price = Some(amount);
    assert_eq!(run_amm_deposit_preflight_facts(limit_lp), Ter::TES_SUCCESS);

    let mut empty = facts(AMM_TWO_ASSET_IF_EMPTY_FLAG);
    empty.amount = Some(amount);
    empty.amount2 = Some(amount2);
    empty.trading_fee = Some(TRADING_FEE_THRESHOLD);
    assert_eq!(run_amm_deposit_preflight_facts(empty), Ter::TES_SUCCESS);

    let mut empty_with_lp = facts(AMM_TWO_ASSET_IF_EMPTY_FLAG);
    empty_with_lp.amount = Some(amount);
    empty_with_lp.amount2 = Some(amount2);
    empty_with_lp.lp_token_out_signum = Some(1);
    assert_eq!(
        run_amm_deposit_preflight_facts(empty_with_lp),
        Ter::TEM_MALFORMED
    );
}

#[test]
fn amm_deposit_preflight_preserves_reference_error_ordering() {
    let amount = asset(1);
    let amount2 = asset(2);

    let mut invalid_pair = facts(AMM_TWO_ASSET_FLAG);
    invalid_pair.amount = Some(amount);
    invalid_pair.amount2 = Some(amount2);
    invalid_pair.asset_pair_invalid = Some(Ter::TEM_BAD_CURRENCY);
    invalid_pair.amount_invalid = Some(Ter::TEM_BAD_AMOUNT);
    assert_eq!(
        run_amm_deposit_preflight_facts(invalid_pair),
        Ter::TEM_BAD_CURRENCY
    );

    let mut same_amount_asset = facts(AMM_TWO_ASSET_FLAG);
    same_amount_asset.amount = Some(amount);
    same_amount_asset.amount2 = Some(amount);
    assert_eq!(
        run_amm_deposit_preflight_facts(same_amount_asset),
        Ter::TEM_BAD_AMM_TOKENS
    );

    let mut bad_lp = facts(AMM_LP_TOKEN_FLAG);
    bad_lp.lp_token_out_signum = Some(0);
    assert_eq!(
        run_amm_deposit_preflight_facts(bad_lp),
        Ter::TEM_BAD_AMM_TOKENS
    );

    let mut bad_amount = facts(AMM_SINGLE_ASSET_FLAG);
    bad_amount.amount = Some(amount);
    bad_amount.amount_invalid = Some(Ter::TEM_BAD_AMOUNT);
    assert_eq!(
        run_amm_deposit_preflight_facts(bad_amount),
        Ter::TEM_BAD_AMOUNT
    );

    let mut bad_eprice = facts(AMM_LIMIT_LP_TOKEN_FLAG);
    bad_eprice.amount = Some(amount);
    bad_eprice.e_price = Some(amount);
    bad_eprice.e_price_invalid = Some(Ter::TEM_BAD_ISSUER);
    assert_eq!(
        run_amm_deposit_preflight_facts(bad_eprice),
        Ter::TEM_BAD_ISSUER
    );

    let mut bad_fee = facts(AMM_TWO_ASSET_IF_EMPTY_FLAG);
    bad_fee.amount = Some(amount);
    bad_fee.amount2 = Some(amount2);
    bad_fee.trading_fee = Some(TRADING_FEE_THRESHOLD + 1);
    assert_eq!(run_amm_deposit_preflight_facts(bad_fee), Ter::TEM_BAD_FEE);
}

#[test]
fn amm_deposit_preclaim_preserves_reference_pool_state_ordering() {
    let mut facts = AMMDepositPreclaimFacts {
        amm_exists: false,
        amm_holds_result: Ter::TEC_INTERNAL,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TER_NO_AMM);

    facts = AMMDepositPreclaimFacts {
        amm_holds_result: Ter::TEC_INTERNAL,
        two_asset_if_empty: true,
        lp_token_balance_signum: 1,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_INTERNAL);

    facts = AMMDepositPreclaimFacts {
        two_asset_if_empty: true,
        lp_token_balance_signum: 1,
        ..Default::default()
    };
    assert_eq!(
        run_amm_deposit_preclaim_facts(facts),
        Ter::TEC_AMM_NOT_EMPTY
    );

    facts = AMMDepositPreclaimFacts {
        two_asset_if_empty: true,
        lp_token_balance_signum: 0,
        amount_balance_signum: 1,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_INTERNAL);

    facts = AMMDepositPreclaimFacts {
        lp_token_balance_signum: 0,
        amount_balance_signum: 0,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_AMM_EMPTY);

    facts = AMMDepositPreclaimFacts {
        lp_token_balance_signum: 1,
        amount_balance_signum: 0,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_INTERNAL);
}

#[test]
fn amm_deposit_preclaim_preserves_reference_check_ordering() {
    let mut facts = AMMDepositPreclaimFacts {
        amm_clawback_enabled: true,
        asset_auth_result: Ter::TEC_NO_AUTH,
        asset_frozen_result: Ter::TEC_LOCKED,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_NO_AUTH);

    facts = AMMDepositPreclaimFacts {
        amm_clawback_enabled: true,
        asset_frozen_result: Ter::TEC_LOCKED,
        asset2_auth_result: Ter::TEC_NO_AUTH,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_LOCKED);

    facts = AMMDepositPreclaimFacts {
        amm_clawback_enabled: true,
        asset2_auth_result: Ter::TEC_NO_AUTH,
        amount_check_result: Ter::TEC_UNFUNDED_AMM,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_NO_AUTH);

    facts = AMMDepositPreclaimFacts {
        amount_check_result: Ter::TEC_UNFUNDED_AMM,
        amount2_check_result: Ter::TEC_NO_AUTH,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_UNFUNDED_AMM);

    facts = AMMDepositPreclaimFacts {
        lp_token_mode: true,
        amount_check_result: Ter::TEC_UNFUNDED_AMM,
        pool_amount_check_result: Ter::TEC_LOCKED,
        pool_amount2_check_result: Ter::TEC_NO_AUTH,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_LOCKED);

    facts = AMMDepositPreclaimFacts {
        lp_token_out_asset_matches_lpt: Some(false),
        account_lp_holds_signum: 0,
        xrp_reserve_positive: false,
        ..Default::default()
    };
    assert_eq!(
        run_amm_deposit_preclaim_facts(facts),
        Ter::TEM_BAD_AMM_TOKENS
    );

    facts = AMMDepositPreclaimFacts {
        account_lp_holds_signum: 0,
        xrp_reserve_positive: false,
        asset_mpt_trade_transfer_result: Ter::TEC_LOCKED,
        ..Default::default()
    };
    assert_eq!(
        run_amm_deposit_preclaim_facts(facts),
        Ter::TEC_INSUF_RESERVE_LINE
    );

    facts = AMMDepositPreclaimFacts {
        asset_mpt_trade_transfer_result: Ter::TEC_LOCKED,
        asset2_mpt_trade_transfer_result: Ter::TEC_NO_AUTH,
        ..Default::default()
    };
    assert_eq!(run_amm_deposit_preclaim_facts(facts), Ter::TEC_LOCKED);

    assert_eq!(
        run_amm_deposit_preclaim_facts(AMMDepositPreclaimFacts::default()),
        Ter::TES_SUCCESS
    );
}

#[test]
fn amm_deposit_apply_math_initial_two_asset_deposit_mints_geometric_mean() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);
    let amount1 = iou(100, asset1);
    let amount2 = iou(400, asset2);

    let result = run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: Some(amount1.clone()),
        amount2: Some(amount2.clone()),
        e_price: None,
        lp_token_out: None,
        pool_amount1: iou(0, asset1),
        pool_amount2: iou(0, asset2),
        lp_token_balance: lp(0, lpt),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: AMM_TWO_ASSET_IF_EMPTY_FLAG,
    })
    .expect("initial deposit should calculate");

    assert_eq!(result.amount1, Some(amount1));
    assert_eq!(result.amount2, Some(amount2));
    assert_eq!(result.lp_tokens, lp(200, lpt));
    assert_eq!(result.new_lp_token_balance, lp(200, lpt));
    assert!(result.empty_pool_reinit);
}

#[test]
fn amm_deposit_apply_math_two_asset_deposit_uses_limiting_side() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);

    let result = run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: Some(iou(20, asset1)),
        amount2: Some(iou(50, asset2)),
        e_price: None,
        lp_token_out: None,
        pool_amount1: iou(100, asset1),
        pool_amount2: iou(400, asset2),
        lp_token_balance: lp(200, lpt),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: AMM_TWO_ASSET_FLAG,
    })
    .expect("balanced deposit should calculate");
    let expected_amount1 = iou(100, asset1)
        .multiply(&lp(25, lpt), Asset::from(asset1))
        .divide(&lp(200, lpt), Asset::from(asset1));
    let expected_amount2 = iou(400, asset2)
        .multiply(&lp(25, lpt), Asset::from(asset2))
        .divide(&lp(200, lpt), Asset::from(asset2));

    assert_eq!(result.lp_tokens, lp(25, lpt));
    assert_eq!(result.amount1, Some(expected_amount1));
    assert_eq!(result.amount2, Some(expected_amount2));
    assert_eq!(result.new_lp_token_balance, lp(225, lpt));
    assert!(!result.empty_pool_reinit);
}

#[test]
fn amm_deposit_apply_math_single_asset_deposit_uses_amm_formula() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);
    let pool = iou(1_000, asset1);
    let deposit = iou(100, asset1);
    let lp_balance = lp(500, lpt);

    let result = run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: Some(deposit.clone()),
        amount2: None,
        e_price: None,
        lp_token_out: None,
        pool_amount1: pool.clone(),
        pool_amount2: iou(2_000, asset2),
        lp_token_balance: lp_balance.clone(),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: AMM_SINGLE_ASSET_FLAG,
    })
    .expect("single asset deposit should calculate");
    let expected_lp = ledger::amm_helpers::adjust_lp_tokens(
        &lp_balance,
        &ledger::amm_helpers::lp_tokens_out(&pool, &deposit, &lp_balance, 0),
        ledger::amm_helpers::IsDeposit::Yes,
    );

    assert_eq!(result.amount1, Some(deposit));
    assert_eq!(result.amount2, None);
    assert_eq!(result.lp_tokens, expected_lp.clone());
    assert_eq!(result.new_lp_token_balance, lp_balance + expected_lp);
    assert!(!result.empty_pool_reinit);
}

#[test]
fn amm_deposit_apply_math_lp_token_mode_deposits_proportional_pool_assets() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);

    let result = run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: None,
        amount2: None,
        e_price: None,
        lp_token_out: Some(lp(25, lpt)),
        pool_amount1: iou(100, asset1),
        pool_amount2: iou(400, asset2),
        lp_token_balance: lp(200, lpt),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: AMM_LP_TOKEN_FLAG,
    })
    .expect("LP token mode should calculate proportional deposit");
    let frac = ledger::amm_helpers::stamount_as_number(&lp(25, lpt))
        / ledger::amm_helpers::stamount_as_number(&lp(200, lpt));

    assert_eq!(
        result.amount1,
        Some(ledger::amm_helpers::get_rounded_asset(
            &Rules::new(std::iter::empty()),
            &iou(100, asset1),
            frac,
            ledger::amm_helpers::IsDeposit::Yes,
        ))
    );
    assert_eq!(
        result.amount2,
        Some(ledger::amm_helpers::get_rounded_asset(
            &Rules::new(std::iter::empty()),
            &iou(400, asset2),
            frac,
            ledger::amm_helpers::IsDeposit::Yes,
        ))
    );
    assert_eq!(result.lp_tokens, lp(25, lpt));
    assert_eq!(result.new_lp_token_balance, lp(225, lpt));
}

#[test]
fn amm_deposit_apply_math_one_asset_lp_token_mode_respects_max_amount() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);
    let pool = iou(1_000, asset1);
    let lp_balance = lp(500, lpt);
    let requested_lp = lp(10, lpt);
    let required_amount = ledger::amm_helpers::amm_asset_in(&pool, &lp_balance, &requested_lp, 0);

    let result = run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: Some(required_amount.clone()),
        amount2: None,
        e_price: None,
        lp_token_out: Some(requested_lp.clone()),
        pool_amount1: pool.clone(),
        pool_amount2: iou(2_000, asset2),
        lp_token_balance: lp_balance.clone(),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: AMM_ONE_ASSET_LP_TOKEN_FLAG,
    })
    .expect("one-asset LP token mode should calculate");

    assert_eq!(result.amount1, Some(required_amount));
    assert_eq!(result.amount2, None);
    assert_eq!(result.lp_tokens, requested_lp.clone());
    assert_eq!(result.new_lp_token_balance, lp_balance + requested_lp);

    let failed = run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: Some(iou(1, asset1)),
        amount2: None,
        e_price: None,
        lp_token_out: Some(lp(10, lpt)),
        pool_amount1: pool,
        pool_amount2: iou(2_000, asset2),
        lp_token_balance: lp(500, lpt),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: AMM_ONE_ASSET_LP_TOKEN_FLAG,
    })
    .unwrap_err();
    assert_eq!(failed, Ter::TEC_AMM_FAILED);
}

#[test]
fn amm_deposit_apply_math_limit_lp_token_accepts_effective_price() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);
    let pool = iou(1_000, asset1);
    let deposit = iou(100, asset1);
    let lp_balance = lp(500, lpt);
    let tokens = ledger::amm_helpers::adjust_lp_tokens(
        &lp_balance,
        &ledger::amm_helpers::lp_tokens_out(&pool, &deposit, &lp_balance, 0),
        ledger::amm_helpers::IsDeposit::Yes,
    );
    let e_price = deposit.divide(&tokens, Asset::from(asset1));

    let result = run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: Some(deposit.clone()),
        amount2: None,
        e_price: Some(e_price),
        lp_token_out: None,
        pool_amount1: pool,
        pool_amount2: iou(2_000, asset2),
        lp_token_balance: lp_balance.clone(),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: AMM_LIMIT_LP_TOKEN_FLAG,
    })
    .expect("limit LP token mode should accept effective price");

    assert_eq!(result.amount1, Some(deposit));
    assert_eq!(result.amount2, None);
    assert_eq!(result.lp_tokens, tokens.clone());
    assert_eq!(result.new_lp_token_balance, lp_balance + tokens);
}

#[test]
fn amm_deposit_apply_math_rejects_bad_runtime_modes() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);
    let base = AMMDepositApplyMathFacts {
        amount1: Some(iou(1, asset1)),
        amount2: Some(iou(1, asset2)),
        e_price: None,
        lp_token_out: None,
        pool_amount1: iou(100, asset1),
        pool_amount2: iou(100, asset2),
        lp_token_balance: lp(100, lpt),
        trading_fee: 0,
        rules: Rules::new(std::iter::empty()),
        flags: 0,
    };

    assert_eq!(
        run_amm_deposit_apply_math_facts(&base).unwrap_err(),
        Ter::TEM_MALFORMED
    );

    let mut non_empty_initial = base.clone();
    non_empty_initial.flags = AMM_TWO_ASSET_IF_EMPTY_FLAG;
    assert_eq!(
        run_amm_deposit_apply_math_facts(&non_empty_initial).unwrap_err(),
        Ter::TEC_AMM_FAILED
    );
}

#[derive(Default)]
struct RecordingDepositSink {
    amm: Option<STLedgerEntry>,
    updated: Option<STLedgerEntry>,
    deposits: Vec<STAmount>,
    minted: Vec<STAmount>,
}

impl AMMDepositApplySink for RecordingDepositSink {
    fn get_amm_entry(&mut self, _: &Asset, _: &Asset) -> Option<STLedgerEntry> {
        self.amm.clone()
    }

    fn update_amm_entry(&mut self, sle: STLedgerEntry) {
        self.updated = Some(sle);
    }

    fn deposit_asset(&mut self, _: &AccountID, amount: &STAmount) -> Ter {
        self.deposits.push(amount.clone());
        Ter::TES_SUCCESS
    }

    fn mint_lp_tokens(&mut self, _: &AccountID, amount: &STAmount) -> Ter {
        self.minted.push(amount.clone());
        Ter::TES_SUCCESS
    }
}

#[test]
fn amm_deposit_do_apply_updates_lp_balance_and_uses_actual_deposits() {
    let asset1 = issue(1);
    let asset2 = issue(2);
    let lpt = issue(3);
    let mut amm = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AMM,
        protocol::keylet::amm(Asset::from(asset1), Asset::from(asset2)).key,
    );
    amm.set_field_amount(get_field_by_symbol("sfLPTokenBalance"), lp(200, lpt));
    let mut sink = RecordingDepositSink {
        amm: Some(amm),
        ..Default::default()
    };

    let ter = run_amm_deposit_do_apply(
        AMMDepositApplyFacts {
            account: account(9),
            asset1: Asset::from(asset1),
            asset2: Asset::from(asset2),
            amount1: Some(iou(20, asset1)),
            amount2: Some(iou(50, asset2)),
            e_price: None,
            lp_token_out: None,
            pool_amount1: iou(100, asset1),
            pool_amount2: iou(400, asset2),
            lp_token_balance: lp(200, lpt),
            trading_fee: 0,
            rules: Rules::new(std::iter::empty()),
            flags: AMM_TWO_ASSET_FLAG,
        },
        &mut sink,
    );

    assert_eq!(ter, Ter::TES_SUCCESS);
    let expected_amount1 = iou(100, asset1)
        .multiply(&lp(25, lpt), Asset::from(asset1))
        .divide(&lp(200, lpt), Asset::from(asset1));
    let expected_amount2 = iou(400, asset2)
        .multiply(&lp(25, lpt), Asset::from(asset2))
        .divide(&lp(200, lpt), Asset::from(asset2));
    assert_eq!(sink.deposits, vec![expected_amount1, expected_amount2]);
    assert_eq!(sink.minted, vec![lp(25, lpt)]);
    let updated = sink.updated.expect("AMM entry should be updated");
    assert_eq!(
        updated.get_field_amount(get_field_by_symbol("sfLPTokenBalance")),
        lp(225, lpt)
    );
}
