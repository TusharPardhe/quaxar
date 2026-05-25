//! `xrpl/protocol/AMMCore.*` parity helpers.

use basics::number::{NumberParts as RuntimeNumber, current_number_one, get_mantissa_scale};
use sha2::{Digest, Sha512};

use crate::{
    AccountID, Currency, Issue, NotTec, Rules, STAmount, Ter, bad_currency, feature_amm,
    feature_universal_number, is_xrp_currency,
};

pub const TRADING_FEE_THRESHOLD: u16 = 1_000;
pub const TOTAL_TIME_SLOT_SECS: u32 = 24 * 3_600;
pub const AUCTION_SLOT_TIME_INTERVALS: u16 = 20;
pub const AUCTION_SLOT_MAX_AUTH_ACCOUNTS: u16 = 4;
pub const AUCTION_SLOT_FEE_SCALE_FACTOR: u32 = 100_000;
pub const AUCTION_SLOT_DISCOUNTED_FEE_FRACTION: u32 = 10;
pub const AUCTION_SLOT_MIN_FEE_FRACTION: u32 = 25;
pub const AUCTION_SLOT_INTERVAL_DURATION: u32 =
    TOTAL_TIME_SLOT_SECS / AUCTION_SLOT_TIME_INTERVALS as u32;
pub const VOTE_MAX_SLOTS: u16 = 8;
pub const VOTE_WEIGHT_SCALE_FACTOR: u32 = 100_000;

fn sha512_half(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    out
}

fn number_from_i64(value: i64) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(value, 0, get_mantissa_scale())
        .expect("small integer should stay representable in Number")
}

pub fn amm_lpt_currency(cur1: Currency, cur2: Currency) -> Currency {
    let (min_currency, max_currency) = if cur1 <= cur2 {
        (cur1, cur2)
    } else {
        (cur2, cur1)
    };
    let hash = sha512_half(&[min_currency.data(), max_currency.data()]);
    let mut bytes = [0u8; Currency::BYTES];
    bytes[0] = 0x03;
    bytes[1..].copy_from_slice(&hash[..Currency::BYTES - 1]);
    Currency::from_array(bytes)
}

pub fn amm_lpt_issue(cur1: Currency, cur2: Currency, amm_account_id: AccountID) -> Issue {
    Issue::new(amm_lpt_currency(cur1, cur2), amm_account_id)
}

pub fn invalid_amm_asset(issue: Issue, pair: Option<(Issue, Issue)>) -> NotTec {
    if bad_currency() == issue.currency {
        return Ter::TEM_BAD_CURRENCY;
    }
    if is_xrp_currency(issue.currency) && issue.account.is_non_zero() {
        return Ter::TEM_BAD_ISSUER;
    }
    if let Some((first, second)) = pair
        && issue != first
        && issue != second
    {
        return Ter::TEM_BAD_AMM_TOKENS;
    }
    Ter::TES_SUCCESS
}

pub fn invalid_amm_asset_pair(
    issue1: Issue,
    issue2: Issue,
    pair: Option<(Issue, Issue)>,
) -> NotTec {
    if issue1 == issue2 {
        return Ter::TEM_BAD_AMM_TOKENS;
    }
    let first = invalid_amm_asset(issue1, pair);
    if first != Ter::TES_SUCCESS {
        return first;
    }
    invalid_amm_asset(issue2, pair)
}

pub fn invalid_amm_amount(
    amount: &STAmount,
    pair: Option<(Issue, Issue)>,
    valid_zero: bool,
) -> NotTec {
    let asset = invalid_amm_asset(amount.issue(), pair);
    if asset != Ter::TES_SUCCESS {
        return asset;
    }
    if amount.signum() < 0 || (!valid_zero && amount.signum() == 0) {
        return Ter::TEM_BAD_AMOUNT;
    }
    Ter::TES_SUCCESS
}

pub fn amm_auction_time_slot(current: u64, auction_slot: &crate::STObject) -> Option<u8> {
    let expiration =
        u64::from(auction_slot.get_field_u32(crate::get_field_by_symbol("sfExpiration")));
    assert!(
        expiration >= u64::from(TOTAL_TIME_SLOT_SECS),
        "xrpl::ammAuctionTimeSlot : minimum expiration"
    );
    let start = expiration - u64::from(TOTAL_TIME_SLOT_SECS);
    if current >= start {
        let diff = current - start;
        if diff < u64::from(TOTAL_TIME_SLOT_SECS) {
            return Some((diff / u64::from(AUCTION_SLOT_INTERVAL_DURATION)) as u8);
        }
    }
    None
}

pub fn amm_enabled(rules: &Rules) -> bool {
    rules.enabled(&feature_amm()) && rules.enabled(&feature_universal_number())
}

pub fn get_fee(tfee: u16) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(i64::from(tfee), 0, get_mantissa_scale())
        .expect("trading fee should stay representable in Number")
        / RuntimeNumber::try_from_external_parts(
            i64::from(AUCTION_SLOT_FEE_SCALE_FACTOR),
            0,
            get_mantissa_scale(),
        )
        .expect("fee scale factor should stay representable in Number")
}

pub fn fee_mult(tfee: u16) -> RuntimeNumber {
    current_number_one() - get_fee(tfee)
}

pub fn fee_mult_half(tfee: u16) -> RuntimeNumber {
    current_number_one() - get_fee(tfee) / number_from_i64(2)
}
