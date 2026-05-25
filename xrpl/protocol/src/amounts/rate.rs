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
