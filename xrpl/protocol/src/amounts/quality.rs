//! `Quality` and offer-rate math ported from `xrpl/protocol/Quality.*` and
//! the quality-related helpers in `xrpl/protocol/the reference source`.

use std::{
    cmp::Ordering,
    panic::{AssertUnwindSafe, catch_unwind},
};

use basics::number::{
    NumberParts as RuntimeNumber, NumberRoundModeGuard, RoundingMode, current_number_one,
    get_mantissa_scale,
};

use crate::{Asset, STAmount, no_issue, sf_generic};

pub const QUALITY_ONE: u32 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QualityFunctionAmmTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QualityFunctionClobLikeTag;

const TEN_TO_14: u64 = 100_000_000_000_000;
const TEN_TO_14_MINUS_1: u64 = TEN_TO_14 - 1;
const TEN_TO_17: u64 = 100_000_000_000_000_000;
const ST_AMOUNT_MIN_VALUE: u64 = 1_000_000_000_000_000;
const ST_AMOUNT_MAX_VALUE: u64 = 9_999_999_999_999_999;
const ST_AMOUNT_MIN_OFFSET: i32 = -96;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amounts {
    pub r#in: STAmount,
    pub out: STAmount,
}

impl Amounts {
    pub fn new(r#in: STAmount, out: STAmount) -> Self {
        Self { r#in, out }
    }

    pub fn empty(&self) -> bool {
        self.r#in.signum() <= 0 || self.out.signum() <= 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Quality {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityFunction {
    m: RuntimeNumber,
    b: RuntimeNumber,
    quality: Option<Quality>,
}

impl Quality {
    pub const MIN_TICK_SIZE: i32 = 3;
    pub const MAX_TICK_SIZE: i32 = 16;

    pub const fn from_value(value: u64) -> Self {
        Self { value }
    }

    pub fn from_amounts(amount: &Amounts) -> Self {
        Self {
            value: get_rate(&amount.out, &amount.r#in),
        }
    }

    pub const fn value(self) -> u64 {
        self.value
    }

    pub fn rate(self) -> STAmount {
        amount_from_quality(self.value)
    }

    pub fn increment(&mut self) {
        assert!(self.value > 0, "xrpl::Quality::increment() : minimum value");
        self.value -= 1;
    }

    pub fn decrement(&mut self) {
        assert!(
            self.value < u64::MAX,
            "xrpl::Quality::decrement() : maximum value"
        );
        self.value += 1;
    }

    pub fn ceil_in(&self, amount: &Amounts, limit: &STAmount) -> Amounts {
        ceil_in_impl(amount, limit, true, *self, div_round)
    }

    pub fn ceil_in_strict(&self, amount: &Amounts, limit: &STAmount, round_up: bool) -> Amounts {
        ceil_in_impl(amount, limit, round_up, *self, div_round_strict)
    }

    pub fn ceil_out(&self, amount: &Amounts, limit: &STAmount) -> Amounts {
        ceil_out_impl(amount, limit, true, *self, mul_round)
    }

    pub fn ceil_out_strict(&self, amount: &Amounts, limit: &STAmount, round_up: bool) -> Amounts {
        ceil_out_impl(amount, limit, round_up, *self, mul_round_strict)
    }

    pub fn round(self, digits: usize) -> Self {
        static MOD: [u64; 17] = [
            10_000_000_000_000_000,
            1_000_000_000_000_000,
            100_000_000_000_000,
            10_000_000_000_000,
            1_000_000_000_000,
            100_000_000_000,
            10_000_000_000,
            1_000_000_000,
            100_000_000,
            10_000_000,
            1_000_000,
            100_000,
            10_000,
            1_000,
            100,
            10,
            1,
        ];

        let exponent = self.value >> 56;
        let mut mantissa = self.value & 0x00ff_ffff_ffff_ffff;
        mantissa += MOD[digits] - 1;
        mantissa -= mantissa % MOD[digits];

        Self {
            value: (exponent << 56) | mantissa,
        }
    }
}

impl PartialOrd for Quality {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Quality {
    fn cmp(&self, other: &Self) -> Ordering {
        other.value.cmp(&self.value)
    }
}

impl QualityFunction {
    pub fn from_quality(quality: Quality, _: QualityFunctionClobLikeTag) -> Self {
        let rate = quality.rate();
        if rate.signum() <= 0 {
            panic!("QualityFunction quality rate is 0.");
        }

        Self {
            m: RuntimeNumber::zero(),
            b: current_number_one() / quality_rate_as_number(quality),
            quality: Some(quality),
        }
    }

    pub fn from_amm(amounts: &Amounts, tfee: u16, _: QualityFunctionAmmTag) -> Self {
        if amounts.r#in.signum() <= 0 || amounts.out.signum() <= 0 {
            panic!("QualityFunction amounts are 0.");
        }

        let cfee = fee_mult(tfee);
        let amount_in = stamount_as_number(&amounts.r#in);
        let amount_out = stamount_as_number(&amounts.out);

        Self {
            m: -cfee / amount_in,
            b: amount_out * cfee / amount_in,
            quality: None,
        }
    }

    pub fn combine(&mut self, qf: &Self) {
        self.m += self.b * qf.m;
        self.b *= qf.b;
        if self.m != RuntimeNumber::zero() {
            self.quality = None;
        }
    }

    pub fn out_from_avg_q(&self, quality: Quality) -> Option<RuntimeNumber> {
        if self.m != RuntimeNumber::zero() && quality.rate().signum() != 0 {
            let _guard = NumberRoundModeGuard::new(RoundingMode::Upward);
            let out = (current_number_one() / quality_rate_as_number(quality) - self.b) / self.m;
            if out <= RuntimeNumber::zero() {
                return None;
            }
            return Some(out);
        }

        None
    }

    pub fn is_const(&self) -> bool {
        self.quality.is_some()
    }

    pub fn quality(&self) -> Option<Quality> {
        self.quality
    }

    pub fn slope(&self) -> RuntimeNumber {
        self.m
    }

    pub fn intercept(&self) -> RuntimeNumber {
        self.b
    }
}

pub fn composed_quality(lhs: Quality, rhs: Quality) -> Quality {
    let lhs_rate = lhs.rate();
    assert!(
        lhs_rate.signum() != 0,
        "xrpl::composed_quality : nonzero left input"
    );

    let rhs_rate = rhs.rate();
    assert!(
        rhs_rate.signum() != 0,
        "xrpl::composed_quality : nonzero right input"
    );

    let rate = mul_round(&lhs_rate, &rhs_rate, lhs_rate.asset(), true);
    let stored_exponent = (rate.exponent() + 100) as u64;
    let stored_mantissa = rate.mantissa();

    assert!(
        (1..=255).contains(&stored_exponent),
        "xrpl::composed_quality : valid exponent"
    );

    Quality::from_value((stored_exponent << 56) | stored_mantissa)
}

pub fn amount_from_quality(rate: u64) -> STAmount {
    if rate == 0 {
        return STAmount::new_with_asset(sf_generic(), no_issue(), 0, 0, false);
    }

    let mantissa = rate & !(255u64 << 56);
    let exponent = ((rate >> 56) as i32) - 100;
    STAmount::new_with_asset(sf_generic(), no_issue(), mantissa, exponent, false)
}

pub fn get_rate(offer_out: &STAmount, offer_in: &STAmount) -> u64 {
    if offer_out.signum() == 0 {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| divide(offer_in, offer_out, no_issue()))).ok();
    let Some(rate) = result else {
        return 0;
    };

    if rate.signum() == 0 {
        return 0;
    }

    if !(-100..=155).contains(&rate.exponent()) {
        return 0;
    }

    let exponent = (rate.exponent() + 100) as u64;
    (exponent << 56) | rate.mantissa()
}

pub fn divide(num: &STAmount, den: &STAmount, asset: impl Into<Asset>) -> STAmount {
    let asset = asset.into();

    if den.signum() == 0 {
        panic!("division by zero");
    }

    if num.signum() == 0 {
        return STAmount::new_with_asset(sf_generic(), asset, 0, 0, false);
    }

    let mut num_val = num.mantissa();
    let mut den_val = den.mantissa();
    let mut num_offset = num.exponent();
    let mut den_offset = den.exponent();

    if is_native_or_mpt_amount(num) {
        while num_val < ST_AMOUNT_MIN_VALUE {
            num_val *= 10;
            num_offset -= 1;
        }
    }

    if is_native_or_mpt_amount(den) {
        while den_val < ST_AMOUNT_MIN_VALUE {
            den_val *= 10;
            den_offset -= 1;
        }
    }

    let amount = muldiv(num_val, TEN_TO_17, den_val)
        .expect("divide should preserve the reference implementation overflow behavior")
        + 5;

    STAmount::new_with_asset(
        sf_generic(),
        asset,
        amount,
        num_offset - den_offset - 17,
        num.negative() != den.negative(),
    )
}

pub fn multiply(v1: &STAmount, v2: &STAmount, asset: impl Into<Asset>) -> STAmount {
    let asset = asset.into();

    if v1.signum() == 0 || v2.signum() == 0 {
        return STAmount::new_with_asset(sf_generic(), asset, 0, 0, false);
    }

    if v1.native() && v2.native() && asset.native() {
        let min_v = signed_native_value(v1).min(signed_native_value(v2));
        let max_v = signed_native_value(v1).max(signed_native_value(v2));

        if min_v > 3_000_000_000 {
            panic!("Native value overflow");
        }

        if ((max_v >> 32) * min_v) > 2_095_475_792 {
            panic!("Native value overflow");
        }

        let product = min_v * max_v;
        return STAmount::new_with_asset(
            sf_generic(),
            asset,
            product.unsigned_abs(),
            0,
            product < 0,
        );
    }

    if v1.holds_mpt_issue() && v2.holds_mpt_issue() && matches!(asset, Asset::MPTIssue(_)) {
        let min_v = signed_mpt_value(v1).min(signed_mpt_value(v2));
        let max_v = signed_mpt_value(v1).max(signed_mpt_value(v2));

        if min_v > 3_037_000_499 {
            panic!("MPT value overflow");
        }

        if ((max_v >> 32) * min_v) > 2_147_483_648 {
            panic!("MPT value overflow");
        }

        let product = min_v * max_v;
        return STAmount::new_with_asset(
            sf_generic(),
            asset,
            product.unsigned_abs(),
            0,
            product < 0,
        );
    }

    let mut value1 = v1.mantissa();
    let mut value2 = v2.mantissa();
    let mut offset1 = v1.exponent();
    let mut offset2 = v2.exponent();

    if is_native_or_mpt_amount(v1) {
        while value1 < ST_AMOUNT_MIN_VALUE {
            value1 *= 10;
            offset1 -= 1;
        }
    }

    if is_native_or_mpt_amount(v2) {
        while value2 < ST_AMOUNT_MIN_VALUE {
            value2 *= 10;
            offset2 -= 1;
        }
    }

    let amount = muldiv(value1, value2, TEN_TO_14)
        .expect("multiply should preserve the reference implementation overflow behavior")
        + 7;

    STAmount::new_with_asset(
        sf_generic(),
        asset,
        amount,
        offset1 + offset2 + 14,
        v1.negative() != v2.negative(),
    )
}

pub fn mul_round(v1: &STAmount, v2: &STAmount, asset: Asset, round_up: bool) -> STAmount {
    mul_round_impl(v1, v2, asset, round_up, canonicalize_round)
}

pub fn mul_round_strict(v1: &STAmount, v2: &STAmount, asset: Asset, round_up: bool) -> STAmount {
    mul_round_impl(v1, v2, asset, round_up, canonicalize_round_strict)
}

pub fn div_round(num: &STAmount, den: &STAmount, asset: Asset, round_up: bool) -> STAmount {
    div_round_impl(num, den, asset, round_up)
}

pub fn div_round_strict(num: &STAmount, den: &STAmount, asset: Asset, round_up: bool) -> STAmount {
    div_round_impl(num, den, asset, round_up)
}

fn ceil_in_impl(
    amount: &Amounts,
    limit: &STAmount,
    round_up: bool,
    quality: Quality,
    div_round_fn: fn(&STAmount, &STAmount, Asset, bool) -> STAmount,
) -> Amounts {
    if amount.r#in > *limit {
        let mut result = Amounts::new(
            limit.clone(),
            div_round_fn(limit, &quality.rate(), amount.out.asset(), round_up),
        );
        if result.out > amount.out {
            result.out = amount.out.clone();
        }
        return result;
    }

    amount.clone()
}

fn ceil_out_impl(
    amount: &Amounts,
    limit: &STAmount,
    round_up: bool,
    quality: Quality,
    mul_round_fn: fn(&STAmount, &STAmount, Asset, bool) -> STAmount,
) -> Amounts {
    if amount.out > *limit {
        let mut result = Amounts::new(
            mul_round_fn(limit, &quality.rate(), amount.r#in.asset(), round_up),
            limit.clone(),
        );
        if result.r#in > amount.r#in {
            result.r#in = amount.r#in.clone();
        }
        return result;
    }

    amount.clone()
}

fn mul_round_impl(
    v1: &STAmount,
    v2: &STAmount,
    asset: Asset,
    round_up: bool,
    canonicalize_fn: fn(bool, &mut u64, &mut i32, bool),
) -> STAmount {
    if v1.signum() == 0 || v2.signum() == 0 {
        return STAmount::new_with_asset(sf_generic(), asset, 0, 0, false);
    }

    let xrp = asset.native();

    if v1.native() && v2.native() && xrp {
        let min_v = signed_native_value(v1).min(signed_native_value(v2));
        let max_v = signed_native_value(v1).max(signed_native_value(v2));

        if min_v > 3_000_000_000 {
            panic!("Native value overflow");
        }

        if ((max_v >> 32) * min_v) > 2_095_475_792 {
            panic!("Native value overflow");
        }

        let product = min_v * max_v;
        return STAmount::new_with_asset(
            sf_generic(),
            asset,
            product.unsigned_abs(),
            0,
            product < 0,
        );
    }

    if v1.holds_mpt_issue() && v2.holds_mpt_issue() && matches!(asset, Asset::MPTIssue(_)) {
        let min_v = signed_mpt_value(v1).min(signed_mpt_value(v2));
        let max_v = signed_mpt_value(v1).max(signed_mpt_value(v2));

        if min_v > 3_037_000_499 {
            panic!("MPT value overflow");
        }

        if ((max_v >> 32) * min_v) > 2_147_483_648 {
            panic!("MPT value overflow");
        }

        let product = min_v * max_v;
        return STAmount::new_with_asset(
            sf_generic(),
            asset,
            product.unsigned_abs(),
            0,
            product < 0,
        );
    }

    let mut value1 = v1.mantissa();
    let mut value2 = v2.mantissa();
    let mut offset1 = v1.exponent();
    let mut offset2 = v2.exponent();

    if is_native_or_mpt_amount(v1) {
        while value1 < ST_AMOUNT_MIN_VALUE {
            value1 *= 10;
            offset1 -= 1;
        }
    }

    if is_native_or_mpt_amount(v2) {
        while value2 < ST_AMOUNT_MIN_VALUE {
            value2 *= 10;
            offset2 -= 1;
        }
    }

    let result_negative = v1.negative() != v2.negative();
    let mut amount = muldiv_round(
        value1,
        value2,
        TEN_TO_14,
        if result_negative != round_up {
            TEN_TO_14_MINUS_1
        } else {
            0
        },
    )
    .expect("mulRound should preserve the reference implementation overflow behavior");

    let mut offset = offset1 + offset2 + 14;
    if result_negative != round_up {
        canonicalize_fn(xrp, &mut amount, &mut offset, round_up);
    }

    let result = STAmount::new_with_asset(sf_generic(), asset, amount, offset, result_negative);
    if round_up && !result_negative && result.signum() == 0 {
        if is_native_or_mpt_asset(asset) {
            return STAmount::new_with_asset(sf_generic(), asset, 1, 0, false);
        }
        return STAmount::new_with_asset(
            sf_generic(),
            asset,
            ST_AMOUNT_MIN_VALUE,
            ST_AMOUNT_MIN_OFFSET,
            false,
        );
    }

    result
}

fn div_round_impl(num: &STAmount, den: &STAmount, asset: Asset, round_up: bool) -> STAmount {
    if den.signum() == 0 {
        panic!("division by zero");
    }

    if num.signum() == 0 {
        return STAmount::new_with_asset(sf_generic(), asset, 0, 0, false);
    }

    let mut num_val = num.mantissa();
    let mut den_val = den.mantissa();
    let mut num_offset = num.exponent();
    let mut den_offset = den.exponent();

    if is_native_or_mpt_amount(num) {
        while num_val < ST_AMOUNT_MIN_VALUE {
            num_val *= 10;
            num_offset -= 1;
        }
    }

    if is_native_or_mpt_amount(den) {
        while den_val < ST_AMOUNT_MIN_VALUE {
            den_val *= 10;
            den_offset -= 1;
        }
    }

    let result_negative = num.negative() != den.negative();
    let mut amount = muldiv_round(
        num_val,
        TEN_TO_17,
        den_val,
        if result_negative != round_up {
            den_val - 1
        } else {
            0
        },
    )
    .expect("divRound should preserve the reference implementation overflow behavior");

    let mut offset = num_offset - den_offset - 17;
    if result_negative != round_up {
        canonicalize_round(
            is_native_or_mpt_asset(asset),
            &mut amount,
            &mut offset,
            round_up,
        );
    }

    let _guard = NumberRoundModeGuard::new(if round_up ^ result_negative {
        RoundingMode::Upward
    } else {
        RoundingMode::Downward
    });
    let result = STAmount::new_with_asset(sf_generic(), asset, amount, offset, result_negative);

    if round_up && !result_negative && result.signum() == 0 {
        if is_native_or_mpt_asset(asset) {
            return STAmount::new_with_asset(sf_generic(), asset, 1, 0, false);
        }
        return STAmount::new_with_asset(
            sf_generic(),
            asset,
            ST_AMOUNT_MIN_VALUE,
            ST_AMOUNT_MIN_OFFSET,
            false,
        );
    }

    result
}

fn is_native_or_mpt_amount(amount: &STAmount) -> bool {
    amount.native() || amount.holds_mpt_issue()
}

fn is_native_or_mpt_asset(asset: Asset) -> bool {
    asset.native() || matches!(asset, Asset::MPTIssue(_))
}

fn signed_native_value(amount: &STAmount) -> i64 {
    if !amount.native() {
        panic!("amount is not native!");
    }

    let mut value = i64::try_from(amount.mantissa()).expect("native mantissa should fit i64");
    if amount.negative() {
        value = -value;
    }
    value
}

fn signed_mpt_value(amount: &STAmount) -> i64 {
    if !amount.holds_mpt_issue() {
        panic!("amount is not MPT!");
    }

    let mut value = i64::try_from(amount.mantissa()).expect("MPT mantissa should fit i64");
    if amount.negative() {
        value = -value;
    }
    value
}

fn stamount_as_number(amount: &STAmount) -> RuntimeNumber {
    if amount.native() {
        RuntimeNumber::from(amount.xrp())
    } else if amount.holds_mpt_issue() {
        RuntimeNumber::from(amount.mpt())
    } else {
        RuntimeNumber::from(amount.iou())
    }
}

fn quality_rate_as_number(quality: Quality) -> RuntimeNumber {
    stamount_as_number(&quality.rate())
}

fn fee_mult(tfee: u16) -> RuntimeNumber {
    let scale = get_mantissa_scale();
    let fee = RuntimeNumber::try_from_external_parts(i64::from(tfee), 0, scale)
        .expect("trading fee should stay representable in Number")
        / RuntimeNumber::try_from_external_parts(100_000, 0, scale)
            .expect("fee scale factor should stay representable in Number");
    current_number_one() - fee
}

fn muldiv(multiplier: u64, multiplicand: u64, divisor: u64) -> Result<u64, String> {
    let ret = (u128::from(multiplier) * u128::from(multiplicand)) / u128::from(divisor);
    u64::try_from(ret).map_err(|_| {
        format!(
            "overflow: ({} * {}) / {}",
            multiplier, multiplicand, divisor
        )
    })
}

fn muldiv_round(
    multiplier: u64,
    multiplicand: u64,
    divisor: u64,
    rounding: u64,
) -> Result<u64, String> {
    let ret = ((u128::from(multiplier) * u128::from(multiplicand)) + u128::from(rounding))
        / u128::from(divisor);
    u64::try_from(ret).map_err(|_| {
        format!(
            "overflow: (({} * {}) + {}) / {}",
            multiplier, multiplicand, rounding, divisor
        )
    })
}

fn canonicalize_round(native: bool, value: &mut u64, offset: &mut i32, _round_up: bool) {
    if native {
        if *offset < 0 {
            let mut loops = 0;
            while *offset < -1 {
                *value /= 10;
                *offset += 1;
                loops += 1;
            }

            *value += if loops >= 2 { 9 } else { 10 };
            *value /= 10;
            *offset += 1;
        }
    } else if *value > ST_AMOUNT_MAX_VALUE {
        while *value > (10 * ST_AMOUNT_MAX_VALUE) {
            *value /= 10;
            *offset += 1;
        }

        *value += 9;
        *value /= 10;
        *offset += 1;
    }
}

fn canonicalize_round_strict(native: bool, value: &mut u64, offset: &mut i32, round_up: bool) {
    if native {
        if *offset < 0 {
            let mut had_remainder = false;
            while *offset < -1 {
                let new_value = *value / 10;
                had_remainder |= *value != (new_value * 10);
                *value = new_value;
                *offset += 1;
            }

            *value += if had_remainder && round_up { 10 } else { 9 };
            *value /= 10;
            *offset += 1;
        }
    } else if *value > ST_AMOUNT_MAX_VALUE {
        while *value > (10 * ST_AMOUNT_MAX_VALUE) {
            *value /= 10;
            *offset += 1;
        }

        *value += 9;
        *value /= 10;
        *offset += 1;
    }
}
