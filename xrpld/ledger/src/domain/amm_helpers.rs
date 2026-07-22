//! `xrpl/ledger/helpers/AMMHelpers.*` compatibility-safe math helpers.

use basics::number::{
    NumberParts as RuntimeNumber, NumberRoundModeGuard, RoundingMode, SaveNumberRoundMode,
    get_rounding_mode, root2, set_rounding_mode,
};
use protocol::{
    IOUAmount, Issue, Quality, STAmount, XRPAmount, fix_ammv1_1, fix_ammv1_3,
    get_current_transaction_rules, get_fee, is_feature_enabled,
};

fn number_from_i64(value: i64) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(value, 0, basics::number::get_mantissa_scale())
        .expect("small integer should stay representable in Number")
}

pub fn stamount_as_number(amount: &STAmount) -> RuntimeNumber {
    if amount.native() {
        RuntimeNumber::from(amount.xrp())
    } else if amount.holds_mpt_issue() {
        RuntimeNumber::from(amount.mpt())
    } else {
        RuntimeNumber::from(amount.iou())
    }
}

pub fn to_st_amount(issue: Issue, amount: RuntimeNumber) -> STAmount {
    if issue.native() {
        STAmount::from_xrp_amount(
            XRPAmount::try_from(amount).expect("XRP amount should stay representable"),
        )
    } else {
        STAmount::from_iou_amount(
            protocol::sf_generic(),
            IOUAmount::try_from(amount).expect("IOU amount should stay representable"),
            issue,
        )
    }
}

fn is_amm_rounding_enabled() -> bool {
    get_current_transaction_rules().is_some_and(|rules| rules.enabled(&fix_ammv1_1()))
}

fn lp_token_rounding(is_deposit: IsDeposit) -> RoundingMode {
    match is_deposit {
        IsDeposit::Yes => RoundingMode::Downward,
        IsDeposit::No => RoundingMode::Upward,
    }
}

fn asset_rounding(is_deposit: IsDeposit) -> RoundingMode {
    match is_deposit {
        IsDeposit::Yes => RoundingMode::Upward,
        IsDeposit::No => RoundingMode::Downward,
    }
}

pub fn multiply(amount: &STAmount, frac: RuntimeNumber, mode: RoundingMode) -> STAmount {
    let _guard = NumberRoundModeGuard::new(mode);
    let product = stamount_as_number(amount) * frac;
    protocol::to_amount_from_number(amount.asset(), product, mode)
        .expect("rounded AMM amount should remain representable")
}

fn solve_quadratic_eq(a: RuntimeNumber, b: RuntimeNumber, c: RuntimeNumber) -> RuntimeNumber {
    (-b + root2(b * b - number_from_i64(4) * a * c).expect("discriminant should be nonnegative"))
        / (number_from_i64(2) * a)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsDeposit {
    No,
    Yes,
}

pub fn amm_lp_tokens(asset1: &STAmount, asset2: &STAmount, lpt_issue: Issue) -> STAmount {
    let rounding = if is_feature_enabled(&fix_ammv1_3()) {
        RoundingMode::Downward
    } else {
        get_rounding_mode()
    };
    let _guard = NumberRoundModeGuard::new(rounding);
    let tokens = root2(stamount_as_number(asset1) * stamount_as_number(asset2))
        .expect("square root should succeed for nonnegative pool balances");
    to_st_amount(lpt_issue, tokens)
}

pub fn lp_tokens_out(
    asset1_balance: &STAmount,
    asset1_deposit: &STAmount,
    lpt_amm_balance: &STAmount,
    tfee: u16,
) -> STAmount {
    let f1 = protocol::fee_mult(tfee);
    let f2 = protocol::fee_mult_half(tfee) / f1;
    let r = stamount_as_number(asset1_deposit) / stamount_as_number(asset1_balance);
    let c = root2(f2 * f2 + r / f1).expect("root2 should succeed") - f2;
    if !is_feature_enabled(&fix_ammv1_3()) {
        let t = stamount_as_number(lpt_amm_balance) * (r - c) / (number_from_i64(1) + c);
        return to_st_amount(lpt_amm_balance.issue(), t);
    }

    let frac = (r - c) / (number_from_i64(1) + c);
    multiply(lpt_amm_balance, frac, RoundingMode::Downward)
}

pub fn amm_asset_in(
    asset1_balance: &STAmount,
    lpt_amm_balance: &STAmount,
    lp_tokens: &STAmount,
    tfee: u16,
) -> STAmount {
    let f1 = protocol::fee_mult(tfee);
    let f2 = protocol::fee_mult_half(tfee) / f1;
    let t1 = stamount_as_number(lp_tokens) / stamount_as_number(lpt_amm_balance);
    let t2 = number_from_i64(1) + t1;
    let d = f2 - t1 / t2;
    let a = number_from_i64(1) / (t2 * t2);
    let b = number_from_i64(2) * d / t2 - number_from_i64(1) / f1;
    let c = d * d - f2 * f2;
    let frac = solve_quadratic_eq(a, b, c);
    if !is_feature_enabled(&fix_ammv1_3()) {
        return protocol::to_amount_from_number(
            asset1_balance.asset(),
            stamount_as_number(asset1_balance) * frac,
            get_rounding_mode(),
        )
        .expect("legacy AMM asset amount should remain representable");
    }

    multiply(asset1_balance, frac, RoundingMode::Upward)
}

pub fn lp_tokens_in(
    asset1_balance: &STAmount,
    asset1_withdraw: &STAmount,
    lpt_amm_balance: &STAmount,
    tfee: u16,
) -> STAmount {
    let fr = stamount_as_number(asset1_withdraw) / stamount_as_number(asset1_balance);
    let f1 = get_fee(tfee);
    let c = fr * f1 + number_from_i64(2) - f1;
    let frac = (c - root2(c * c - number_from_i64(4) * fr).expect("root2 should succeed"))
        / number_from_i64(2);
    if !is_feature_enabled(&fix_ammv1_3()) {
        return to_st_amount(
            lpt_amm_balance.issue(),
            stamount_as_number(lpt_amm_balance) * frac,
        );
    }

    multiply(lpt_amm_balance, frac, RoundingMode::Upward)
}

pub fn amm_asset_out(
    asset_balance: &STAmount,
    lpt_amm_balance: &STAmount,
    lp_tokens: &STAmount,
    tfee: u16,
) -> STAmount {
    let f = get_fee(tfee);
    let t1 = stamount_as_number(lp_tokens) / stamount_as_number(lpt_amm_balance);
    let frac = (t1 * t1 - t1 * (number_from_i64(2) - f)) / (t1 * f - number_from_i64(1));
    if !is_feature_enabled(&fix_ammv1_3()) {
        return protocol::to_amount_from_number(
            asset_balance.asset(),
            stamount_as_number(asset_balance) * frac,
            get_rounding_mode(),
        )
        .expect("legacy AMM asset amount should remain representable");
    }

    multiply(asset_balance, frac, RoundingMode::Downward)
}

pub fn within_relative_distance_quality(
    calc_quality: Quality,
    req_quality: Quality,
    dist: RuntimeNumber,
) -> bool {
    if calc_quality == req_quality {
        return true;
    }
    let (min_quality, max_quality) = if calc_quality <= req_quality {
        (calc_quality, req_quality)
    } else {
        (req_quality, calc_quality)
    };
    let min_rate = stamount_as_number(&min_quality.rate());
    let max_rate = stamount_as_number(&max_quality.rate());
    ((min_rate - max_rate) / min_rate) < dist
}

pub trait RelativeDistanceAmount: Clone + PartialEq + PartialOrd {
    fn as_number(&self) -> RuntimeNumber;
}

impl RelativeDistanceAmount for STAmount {
    fn as_number(&self) -> RuntimeNumber {
        stamount_as_number(self)
    }
}

impl RelativeDistanceAmount for IOUAmount {
    fn as_number(&self) -> RuntimeNumber {
        RuntimeNumber::from(*self)
    }
}

impl RelativeDistanceAmount for XRPAmount {
    fn as_number(&self) -> RuntimeNumber {
        RuntimeNumber::from(*self)
    }
}

impl RelativeDistanceAmount for RuntimeNumber {
    fn as_number(&self) -> RuntimeNumber {
        *self
    }
}

pub fn within_relative_distance_amount<T>(calc: T, req: T, dist: RuntimeNumber) -> bool
where
    T: RelativeDistanceAmount,
{
    if calc == req {
        return true;
    }
    let (min_amount, max_amount) = if calc < req { (calc, req) } else { (req, calc) };
    let min_number = min_amount.as_number();
    let max_number = max_amount.as_number();
    ((max_number - min_number) / max_number) < dist
}

pub fn solve_quadratic_eq_smallest(
    a: RuntimeNumber,
    b: RuntimeNumber,
    c: RuntimeNumber,
) -> Option<RuntimeNumber> {
    let d = b * b - number_from_i64(4) * a * c;
    if d < RuntimeNumber::zero() {
        return None;
    }
    let sqrt_d = root2(d).ok()?;
    if b > RuntimeNumber::zero() {
        Some((number_from_i64(2) * c) / (-b - sqrt_d))
    } else {
        Some((number_from_i64(2) * c) / (-b + sqrt_d))
    }
}

pub fn adjust_lp_tokens(
    lpt_amm_balance: &STAmount,
    lp_tokens: &STAmount,
    is_deposit: IsDeposit,
) -> STAmount {
    let _saved = SaveNumberRoundMode::new(set_rounding_mode(RoundingMode::Downward));
    match is_deposit {
        IsDeposit::Yes => lpt_amm_balance.clone() + lp_tokens.clone() - lpt_amm_balance.clone(),
        IsDeposit::No => lp_tokens.clone() - lpt_amm_balance.clone() + lpt_amm_balance.clone(),
    }
}

pub fn adjust_amounts_by_lp_tokens(
    amount_balance: &STAmount,
    amount: &STAmount,
    amount2: Option<&STAmount>,
    lpt_amm_balance: &STAmount,
    lp_tokens: &STAmount,
    tfee: u16,
    is_deposit: IsDeposit,
) -> (STAmount, Option<STAmount>, STAmount) {
    if is_feature_enabled(&fix_ammv1_3()) {
        return (amount.clone(), amount2.cloned(), lp_tokens.clone());
    }

    let lp_tokens_actual = adjust_lp_tokens(lpt_amm_balance, lp_tokens, is_deposit);
    if lp_tokens_actual.signum() == 0 {
        return (
            STAmount::default(),
            amount2.map(|_| STAmount::default()),
            lp_tokens_actual,
        );
    }

    if lp_tokens_actual < *lp_tokens {
        let amm_rounding_enabled = is_amm_rounding_enabled();
        if let Some(amount2) = amount2 {
            let fr = stamount_as_number(&lp_tokens_actual) / stamount_as_number(lp_tokens);
            let amount_actual = protocol::to_amount_from_number(
                amount.asset(),
                fr * stamount_as_number(amount),
                get_rounding_mode(),
            )
            .expect("legacy AMM asset amount should remain representable");
            let amount2_actual = protocol::to_amount_from_number(
                amount2.asset(),
                fr * stamount_as_number(amount2),
                get_rounding_mode(),
            )
            .expect("legacy AMM asset amount should remain representable");
            if !amm_rounding_enabled {
                return (
                    std::cmp::min(amount_actual, amount.clone()),
                    Some(std::cmp::min(amount2_actual, amount2.clone())),
                    lp_tokens_actual,
                );
            }
            return (amount_actual, Some(amount2_actual), lp_tokens_actual);
        }

        let amount_actual = match is_deposit {
            IsDeposit::Yes => {
                amm_asset_in(amount_balance, lpt_amm_balance, &lp_tokens_actual, tfee)
            }
            IsDeposit::No if !amm_rounding_enabled => {
                amm_asset_out(amount_balance, lpt_amm_balance, lp_tokens, tfee)
            }
            IsDeposit::No => {
                amm_asset_out(amount_balance, lpt_amm_balance, &lp_tokens_actual, tfee)
            }
        };
        if !amm_rounding_enabled {
            return (
                std::cmp::min(amount_actual, amount.clone()),
                None,
                lp_tokens_actual,
            );
        }
        return (amount_actual, None, lp_tokens_actual);
    }

    (amount.clone(), amount2.cloned(), lp_tokens_actual)
}

pub fn get_rounded_asset(
    rules: &protocol::Rules,
    balance: &STAmount,
    frac: RuntimeNumber,
    is_deposit: IsDeposit,
) -> STAmount {
    if !rules.enabled(&fix_ammv1_3()) {
        return protocol::to_amount_from_number(
            balance.asset(),
            stamount_as_number(balance) * frac,
            get_rounding_mode(),
        )
        .expect("legacy AMM asset amount should remain representable");
    }
    multiply(balance, frac, asset_rounding(is_deposit))
}

pub fn get_rounded_asset_with_product<F, G>(
    rules: &protocol::Rules,
    no_round_cb: F,
    balance: &STAmount,
    product_cb: G,
    is_deposit: IsDeposit,
) -> STAmount
where
    F: FnOnce() -> RuntimeNumber,
    G: FnOnce() -> RuntimeNumber,
{
    if !rules.enabled(&fix_ammv1_3()) {
        return protocol::to_amount_from_number(
            balance.asset(),
            no_round_cb(),
            get_rounding_mode(),
        )
        .expect("legacy AMM asset amount should remain representable");
    }

    let rm = asset_rounding(is_deposit);
    if matches!(is_deposit, IsDeposit::Yes) {
        return multiply(balance, product_cb(), rm);
    }

    let _guard = NumberRoundModeGuard::new(rm);
    protocol::to_amount_from_number(balance.asset(), product_cb(), rm)
        .expect("rounded AMM asset amount should remain representable")
}

pub fn get_rounded_lp_tokens(
    rules: &protocol::Rules,
    balance: &STAmount,
    frac: RuntimeNumber,
    is_deposit: IsDeposit,
) -> STAmount {
    if !rules.enabled(&fix_ammv1_3()) {
        return to_st_amount(balance.issue(), stamount_as_number(balance) * frac);
    }

    let tokens = multiply(balance, frac, lp_token_rounding(is_deposit));
    adjust_lp_tokens(balance, &tokens, is_deposit)
}

pub fn get_rounded_lp_tokens_with_product<F, G>(
    rules: &protocol::Rules,
    no_round_cb: F,
    lpt_amm_balance: &STAmount,
    product_cb: G,
    is_deposit: IsDeposit,
) -> STAmount
where
    F: FnOnce() -> RuntimeNumber,
    G: FnOnce() -> RuntimeNumber,
{
    if !rules.enabled(&fix_ammv1_3()) {
        return to_st_amount(lpt_amm_balance.issue(), no_round_cb());
    }

    let tokens = match is_deposit {
        IsDeposit::Yes => {
            let _guard = NumberRoundModeGuard::new(lp_token_rounding(is_deposit));
            to_st_amount(lpt_amm_balance.issue(), product_cb())
        }
        IsDeposit::No => multiply(lpt_amm_balance, product_cb(), lp_token_rounding(is_deposit)),
    };
    adjust_lp_tokens(lpt_amm_balance, &tokens, is_deposit)
}

pub fn adjust_asset_in_by_tokens(
    rules: &protocol::Rules,
    balance: &STAmount,
    amount: &STAmount,
    lpt_amm_balance: &STAmount,
    tokens: &STAmount,
    tfee: u16,
) -> (STAmount, STAmount) {
    if !rules.enabled(&fix_ammv1_3()) {
        return (tokens.clone(), amount.clone());
    }

    let mut asset_adj = amm_asset_in(balance, lpt_amm_balance, tokens, tfee);
    let mut tokens_adj = tokens.clone();
    if asset_adj > *amount {
        let adj_amount = protocol::to_amount_from_number(
            amount.asset(),
            number_from_i64(2) * stamount_as_number(amount) - stamount_as_number(&asset_adj),
            get_rounding_mode(),
        )
        .expect("AMM adjusted asset amount should remain representable");
        let t = lp_tokens_out(balance, &adj_amount, lpt_amm_balance, tfee);
        tokens_adj = adjust_lp_tokens(lpt_amm_balance, &t, IsDeposit::Yes);
        asset_adj = amm_asset_in(balance, lpt_amm_balance, &tokens_adj, tfee);
    }
    (tokens_adj, std::cmp::min(amount.clone(), asset_adj))
}

pub fn adjust_asset_out_by_tokens(
    rules: &protocol::Rules,
    balance: &STAmount,
    amount: &STAmount,
    lpt_amm_balance: &STAmount,
    tokens: &STAmount,
    tfee: u16,
) -> (STAmount, STAmount) {
    if !rules.enabled(&fix_ammv1_3()) {
        return (tokens.clone(), amount.clone());
    }

    let mut asset_adj = amm_asset_out(balance, lpt_amm_balance, tokens, tfee);
    let mut tokens_adj = tokens.clone();
    if asset_adj > *amount {
        let adj_amount = protocol::to_amount_from_number(
            amount.asset(),
            number_from_i64(2) * stamount_as_number(amount) - stamount_as_number(&asset_adj),
            get_rounding_mode(),
        )
        .expect("AMM adjusted asset amount should remain representable");
        let t = lp_tokens_in(balance, &adj_amount, lpt_amm_balance, tfee);
        tokens_adj = adjust_lp_tokens(lpt_amm_balance, &t, IsDeposit::No);
        asset_adj = amm_asset_out(balance, lpt_amm_balance, &tokens_adj, tfee);
    }
    (tokens_adj, std::cmp::min(amount.clone(), asset_adj))
}

pub fn adjust_frac_by_tokens(
    rules: &protocol::Rules,
    lpt_amm_balance: &STAmount,
    tokens: &STAmount,
    frac: RuntimeNumber,
) -> RuntimeNumber {
    if !rules.enabled(&fix_ammv1_3()) {
        return frac;
    }
    stamount_as_number(tokens) / stamount_as_number(lpt_amm_balance)
}
