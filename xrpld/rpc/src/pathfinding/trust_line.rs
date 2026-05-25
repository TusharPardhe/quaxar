//! Trust line wrappers ported from `xrpld/rpc/detail/TrustLine.h/the reference source`.
//!
//! Wraps a RippleState SLE from one account's perspective, exposing balance,
//! limits, flags (freeze, auth, noRipple) relative to the viewing account.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{AccountID, Currency, Issue, JsonValue, STAmount};

// --- Ledger entry flags (from reference LedgerFormats) ---
const LSF_LOW_AUTH: u32 = 0x0001_0000;
const LSF_HIGH_AUTH: u32 = 0x0002_0000;
const LSF_LOW_NO_RIPPLE: u32 = 0x0010_0000;
const LSF_HIGH_NO_RIPPLE: u32 = 0x0020_0000;
const LSF_LOW_FREEZE: u32 = 0x0040_0000;
const LSF_HIGH_FREEZE: u32 = 0x0080_0000;
const LSF_LOW_DEEP_FREEZE: u32 = 0x0100_0000;
const LSF_HIGH_DEEP_FREEZE: u32 = 0x0200_0000;

/// Describes how an account was found in a path.
///
/// `Outgoing` = source account or found via a trust line with rippling enabled.
/// `Incoming` = found via a trust line with rippling disabled on the account's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineDirection {
    Incoming = 0,
    Outgoing = 1,
}

/// Minimal SLE-like trait for trust line construction.
/// In the real integration this would be the actual SLE type.
pub trait TrustLineSle {
    fn key(&self) -> Uint256;
    fn sle_type(&self) -> u16;
    fn get_field_amount(&self, field: &str) -> STAmount;
    fn get_field_u32(&self, field: &str) -> u32;
}

/// Base trust line wrapper. Presents a RippleState SLE from one account's
/// perspective. The reference class has `lowLimit_`, `highLimit_`, `balance_`,
/// `flags_`, and `viewLowest_`.
#[derive(Debug, Clone)]
pub struct TrustLineBase {
    pub key: Uint256,
    pub low_limit: STAmount,
    pub high_limit: STAmount,
    pub balance: STAmount,
    pub flags: u32,
    pub view_lowest: bool,
}

/// Ledger entry type for RippleState.
pub const LT_RIPPLE_STATE: u16 = 0x0072;

impl TrustLineBase {
    /// Construct from an SLE and the viewing account.
    /// Mirrors the reference `TrustLineBase(sle, viewAccount)` constructor.
    pub fn new(sle: &dyn TrustLineSle, view_account: &AccountID) -> Self {
        let key = sle.key();
        let low_limit = sle.get_field_amount("LowLimit");
        let high_limit = sle.get_field_amount("HighLimit");
        let mut balance = sle.get_field_amount("Balance");
        let flags = sle.get_field_u32("Flags");
        let view_lowest = low_limit.issue().issuer() == *view_account;

        if !view_lowest {
            balance.negate();
        }

        Self {
            key,
            low_limit,
            high_limit,
            balance,
            flags,
            view_lowest,
        }
    }

    pub fn get_account_id(&self) -> AccountID {
        if self.view_lowest {
            self.low_limit.issue().issuer()
        } else {
            self.high_limit.issue().issuer()
        }
    }

    pub fn get_account_id_peer(&self) -> AccountID {
        if !self.view_lowest {
            self.low_limit.issue().issuer()
        } else {
            self.high_limit.issue().issuer()
        }
    }

    pub fn get_auth(&self) -> bool {
        let flag = if self.view_lowest {
            LSF_LOW_AUTH
        } else {
            LSF_HIGH_AUTH
        };
        (self.flags & flag) != 0
    }

    pub fn get_auth_peer(&self) -> bool {
        let flag = if !self.view_lowest {
            LSF_LOW_AUTH
        } else {
            LSF_HIGH_AUTH
        };
        (self.flags & flag) != 0
    }

    pub fn get_no_ripple(&self) -> bool {
        let flag = if self.view_lowest {
            LSF_LOW_NO_RIPPLE
        } else {
            LSF_HIGH_NO_RIPPLE
        };
        (self.flags & flag) != 0
    }

    pub fn get_no_ripple_peer(&self) -> bool {
        let flag = if !self.view_lowest {
            LSF_LOW_NO_RIPPLE
        } else {
            LSF_HIGH_NO_RIPPLE
        };
        (self.flags & flag) != 0
    }

    pub fn get_direction(&self) -> LineDirection {
        if self.get_no_ripple() {
            LineDirection::Incoming
        } else {
            LineDirection::Outgoing
        }
    }

    pub fn get_direction_peer(&self) -> LineDirection {
        if self.get_no_ripple_peer() {
            LineDirection::Incoming
        } else {
            LineDirection::Outgoing
        }
    }

    pub fn get_freeze(&self) -> bool {
        let flag = if self.view_lowest {
            LSF_LOW_FREEZE
        } else {
            LSF_HIGH_FREEZE
        };
        (self.flags & flag) != 0
    }

    pub fn get_deep_freeze(&self) -> bool {
        let flag = if self.view_lowest {
            LSF_LOW_DEEP_FREEZE
        } else {
            LSF_HIGH_DEEP_FREEZE
        };
        (self.flags & flag) != 0
    }

    pub fn get_freeze_peer(&self) -> bool {
        let flag = if !self.view_lowest {
            LSF_LOW_FREEZE
        } else {
            LSF_HIGH_FREEZE
        };
        (self.flags & flag) != 0
    }

    pub fn get_deep_freeze_peer(&self) -> bool {
        let flag = if !self.view_lowest {
            LSF_LOW_DEEP_FREEZE
        } else {
            LSF_HIGH_DEEP_FREEZE
        };
        (self.flags & flag) != 0
    }

    pub fn get_balance(&self) -> &STAmount {
        &self.balance
    }

    pub fn get_limit(&self) -> &STAmount {
        if self.view_lowest {
            &self.low_limit
        } else {
            &self.high_limit
        }
    }

    pub fn get_limit_peer(&self) -> &STAmount {
        if !self.view_lowest {
            &self.low_limit
        } else {
            &self.high_limit
        }
    }

    pub fn get_json(&self) -> JsonValue {
        let mut map = BTreeMap::new();
        map.insert(
            "low_id".to_owned(),
            JsonValue::String(self.low_limit.issue().issuer().to_string()),
        );
        map.insert(
            "high_id".to_owned(),
            JsonValue::String(self.high_limit.issue().issuer().to_string()),
        );
        JsonValue::Object(map)
    }
}

/// PathFindTrustLine — used by the pathfinder. Lightweight wrapper.
#[derive(Debug, Clone)]
pub struct PathFindTrustLine {
    pub base: TrustLineBase,
}

impl PathFindTrustLine {
    /// Construct from an SLE. Returns None if the SLE is not a RippleState.
    pub fn make_item(account_id: &AccountID, sle: &dyn TrustLineSle) -> Option<Self> {
        if sle.sle_type() != LT_RIPPLE_STATE {
            return None;
        }
        Some(Self {
            base: TrustLineBase::new(sle, account_id),
        })
    }

    // Delegate accessors to base
    pub fn get_balance(&self) -> &STAmount {
        self.base.get_balance()
    }
    pub fn get_limit(&self) -> &STAmount {
        self.base.get_limit()
    }
    pub fn get_limit_peer(&self) -> &STAmount {
        self.base.get_limit_peer()
    }
    pub fn get_no_ripple(&self) -> bool {
        self.base.get_no_ripple()
    }
    pub fn get_direction(&self) -> LineDirection {
        self.base.get_direction()
    }
}

/// Rate type (quality in/out) — mirrors `xrpl/protocol/Rate.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate(pub u32);

impl Rate {
    pub const QUALITY_ONE: u32 = 1_000_000_000;

    pub fn new(value: u32) -> Self {
        Self(value)
    }
}

/// RPCTrustLine — used by the `account_lines` RPC command.
/// Includes quality in/out values.
#[derive(Debug, Clone)]
pub struct RPCTrustLine {
    pub base: TrustLineBase,
    pub low_quality_in: Rate,
    pub low_quality_out: Rate,
    pub high_quality_in: Rate,
    pub high_quality_out: Rate,
}

impl RPCTrustLine {
    pub fn new(sle: &dyn TrustLineSle, view_account: &AccountID) -> Self {
        let base = TrustLineBase::new(sle, view_account);
        Self {
            base,
            low_quality_in: Rate::new(sle.get_field_u32("LowQualityIn")),
            low_quality_out: Rate::new(sle.get_field_u32("LowQualityOut")),
            high_quality_in: Rate::new(sle.get_field_u32("HighQualityIn")),
            high_quality_out: Rate::new(sle.get_field_u32("HighQualityOut")),
        }
    }

    pub fn make_item(account_id: &AccountID, sle: &dyn TrustLineSle) -> Option<Self> {
        if sle.sle_type() != LT_RIPPLE_STATE {
            return None;
        }
        Some(Self::new(sle, account_id))
    }

    pub fn get_quality_in(&self) -> &Rate {
        if self.base.view_lowest {
            &self.low_quality_in
        } else {
            &self.high_quality_in
        }
    }

    pub fn get_quality_out(&self) -> &Rate {
        if self.base.view_lowest {
            &self.low_quality_out
        } else {
            &self.high_quality_out
        }
    }
}
