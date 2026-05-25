//! `PathAsset` port from `xrpl/protocol/PathAsset.h`.

use crate::{Asset, Currency, MPTID, currency_to_string, is_xrp_currency};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathAsset {
    Currency(Currency),
    MPTID(MPTID),
}

impl Default for PathAsset {
    fn default() -> Self {
        Self::Currency(Currency::zero())
    }
}

impl PathAsset {
    pub fn holds_currency(&self) -> bool {
        matches!(self, Self::Currency(_))
    }

    pub fn holds_mpt(&self) -> bool {
        matches!(self, Self::MPTID(_))
    }

    pub fn is_xrp(&self) -> bool {
        matches!(self, Self::Currency(currency) if is_xrp_currency(*currency))
    }

    pub fn currency(&self) -> Currency {
        match self {
            Self::Currency(currency) => *currency,
            Self::MPTID(_) => panic!("PathAsset does not hold a Currency"),
        }
    }

    pub fn mpt_id(&self) -> MPTID {
        match self {
            Self::Currency(_) => panic!("PathAsset does not hold an MPTID"),
            Self::MPTID(mpt_id) => *mpt_id,
        }
    }

    pub fn visit<R, FC, FM>(self, on_currency: FC, on_mpt: FM) -> R
    where
        FC: FnOnce(Currency) -> R,
        FM: FnOnce(MPTID) -> R,
    {
        match self {
            Self::Currency(currency) => on_currency(currency),
            Self::MPTID(mpt_id) => on_mpt(mpt_id),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Self::Currency(currency) => currency_to_string(*currency),
            Self::MPTID(mpt_id) => mpt_id.to_string(),
        }
    }
}

impl From<Currency> for PathAsset {
    fn from(value: Currency) -> Self {
        Self::Currency(value)
    }
}

impl From<MPTID> for PathAsset {
    fn from(value: MPTID) -> Self {
        Self::MPTID(value)
    }
}

impl From<Asset> for PathAsset {
    fn from(value: Asset) -> Self {
        match value {
            Asset::Issue(issue) => Self::Currency(issue.currency),
            Asset::MPTIssue(issue) => Self::MPTID(issue.mpt_id()),
        }
    }
}
