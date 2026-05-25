//! PathFindMPT ported from `xrpld/rpc/detail/MPT.h`.
//!
//! Lightweight struct for MPT pathfinding — tracks MPTID, zero-balance state,
//! and whether the token is maxed out.

#![allow(dead_code)]

use protocol::MPTID;

/// Pathfinding-specific MPT entry. Tracks whether the holder has zero balance
/// and whether the issuance has reached its maximum outstanding amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathFindMPT {
    mpt_id: MPTID,
    /// If true then holder's balance is 0 (always false for issuer).
    zero_balance: bool,
    /// OutstandingAmount == MaximumAmount.
    maxed_out: bool,
}

impl PathFindMPT {
    pub fn new(mpt_id: MPTID, zero_balance: bool, maxed_out: bool) -> Self {
        Self {
            mpt_id,
            zero_balance,
            maxed_out,
        }
    }

    /// Construct with default zero_balance=false, maxed_out=false.
    pub fn from_id(mpt_id: MPTID) -> Self {
        Self {
            mpt_id,
            zero_balance: false,
            maxed_out: false,
        }
    }

    pub fn mpt_id(&self) -> &MPTID {
        &self.mpt_id
    }

    pub fn is_zero_balance(&self) -> bool {
        self.zero_balance
    }

    pub fn is_maxed_out(&self) -> bool {
        self.maxed_out
    }
}

impl AsRef<MPTID> for PathFindMPT {
    fn as_ref(&self) -> &MPTID {
        &self.mpt_id
    }
}
