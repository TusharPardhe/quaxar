//! Transfer-rate helpers from `xrpl/protocol/Rate.h` and `Rate2the reference source`.

use crate::{Asset, QUALITY_ONE, STAmount, div_round, divide, mul_round, multiply};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rate {
    pub value: u32,
}

impl Rate {
    pub const fn new(value: u32) -> Self {
        Self { value }
    }
}

impl std::fmt::Display for Rate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

pub const PARITY_RATE: Rate = Rate { value: QUALITY_ONE };

fn as_amount(rate: Rate) -> STAmount {
    STAmount::new_with_asset(
        crate::sf_generic(),
        crate::no_issue(),
        u64::from(rate.value),
        -9,
        false,
    )
}

pub fn multiply_rate(amount: &STAmount, rate: Rate) -> STAmount {
    assert!(rate.value != 0, "nonzero rate input");
    if rate == PARITY_RATE {
        return amount.clone();
    }
    multiply(amount, &as_amount(rate), amount.asset())
}

pub fn multiply_round(amount: &STAmount, rate: Rate, round_up: bool) -> STAmount {
    assert!(rate.value != 0, "nonzero rate input");
    if rate == PARITY_RATE {
        return amount.clone();
    }
    mul_round(amount, &as_amount(rate), amount.asset(), round_up)
}

pub fn multiply_round_with_asset(
    amount: &STAmount,
    rate: Rate,
    asset: Asset,
    round_up: bool,
) -> STAmount {
    assert!(rate.value != 0, "nonzero rate input");
    if rate == PARITY_RATE {
        return amount.clone();
    }
    mul_round(amount, &as_amount(rate), asset, round_up)
}

pub fn divide_rate(amount: &STAmount, rate: Rate) -> STAmount {
    assert!(rate.value != 0, "nonzero rate input");
    if rate == PARITY_RATE {
        return amount.clone();
    }
    divide(amount, &as_amount(rate), amount.asset())
}

pub fn divide_round(amount: &STAmount, rate: Rate, round_up: bool) -> STAmount {
    assert!(rate.value != 0, "nonzero rate input");
    if rate == PARITY_RATE {
        return amount.clone();
    }
    div_round(amount, &as_amount(rate), amount.asset(), round_up)
}

pub fn divide_round_with_asset(
    amount: &STAmount,
    rate: Rate,
    asset: Asset,
    round_up: bool,
) -> STAmount {
    assert!(rate.value != 0, "nonzero rate input");
    if rate == PARITY_RATE {
        return amount.clone();
    }
    div_round(amount, &as_amount(rate), asset, round_up)
}

pub mod nft {
    use super::Rate;

    pub fn transfer_fee_as_rate(fee: u16) -> Rate {
        Rate::new(u32::from(fee) * 10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::{PARITY_RATE, Rate, nft::transfer_fee_as_rate};

    #[test]
    fn transfer_fee_rate_scaling() {
        assert_eq!(transfer_fee_as_rate(1).value, 10_000);
        assert_eq!(PARITY_RATE.value, crate::QUALITY_ONE);
        assert_eq!(Rate::new(7).to_string(), "7");
    }
}

#[cfg(test)]
mod mpt_rate_tests {
    use super::*;
    use crate::{AccountID, MPTAmount, MPTIssue, make_mpt_id, sf_generic};

    /// Testnet ledger 21511422, tx 492FDB4D...: an MPT partial payment with
    /// TransferFee 3 and SendMax 122354741735175090. `divide` builds an
    /// MPT mantissa above i64::MAX at exponent -2; rippled's canonicalize
    /// truncates the last internal digit and the network delivered
    /// 122351071203038999. quaxar threw "MPT amount out of range"
    /// (tefEXCEPTION) and forked.
    #[test]
    fn mpt_partial_payment_rate_math_matches_testnet_21511422() {
        let issue = MPTIssue::new(make_mpt_id(21_511_363, AccountID::from_array([0x11; 20])));
        let amt = |v: i64| STAmount::from_mpt_amount(sf_generic(), MPTAmount::from(v), issue);
        let rate = Rate::new(1_000_000_000 + 3 * 10_000);
        // Testnet rules at that ledger select the Small Number mantissa.
        let _scale = basics::number::NumberMantissaScaleGuard::new(
            basics::number::MantissaScale::Small,
        );

        let delivered = divide_rate(&amt(122_354_741_735_175_090), rate);
        assert_eq!(delivered.mpt().value(), 122_351_071_203_038_999);
        // Sender debit including the fee, as recorded by the network.
        assert_eq!(multiply_rate(&delivered, rate).mpt().value(), 122_354_741_735_175_100);
    }
}
