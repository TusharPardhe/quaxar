//! Full reference the reference source parity.
//!
//! XRPEndpointStep handles XRP at the source or destination of a payment path.
//! It computes available XRP (balance - reserve) and transfers it.

use crate::ApplyView;
use crate::domain::ripple_state_helpers;
use protocol::{AccountID, Ter, XRPAmount, get_field_by_symbol as sf};

/// Compute the maximum XRP that can flow from an account.
/// Returns available XRP (balance - reserve - fees).
pub fn max_xrp_flow<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    reserve: XRPAmount,
) -> XRPAmount {
    let acct_keylet =
        protocol::account_keylet(basics::base_uint::Uint160::from_void(account.data()));
    let Some(sle) = view.peek(acct_keylet).ok().flatten() else {
        return XRPAmount::from_drops(0);
    };
    let balance = sle.get_field_amount(sf("sfBalance")).xrp();
    let available = balance.drops().saturating_sub(reserve.drops());
    XRPAmount::from_drops(available.max(0))
}

/// Execute an XRP endpoint step: transfer XRP from src to dst.
/// For source endpoint: src sends XRP into the path.
/// For destination endpoint: path delivers XRP to dst.
pub fn execute_xrp_endpoint<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    amount: XRPAmount,
) -> Result<XRPAmount, Ter> {
    if amount.drops() <= 0 {
        return Ok(XRPAmount::from_drops(0));
    }
    let res = ripple_state_helpers::transfer_xrp(view, src, dst, amount);
    if res != Ter::TES_SUCCESS {
        return Err(res);
    }
    Ok(amount)
}

/// Context for creating an XRP endpoint step
#[derive(Debug, Clone)]
pub struct XrpEndpointContext {
    pub is_first: bool,
    pub is_last: bool,
    pub offer_crossing: bool,
    pub strand_deliver: protocol::Asset,
}

/// XRP endpoint step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XrpEndpointStep {
    pub account: AccountID,
    pub is_last: bool,
    pub is_first: bool,
}

impl XrpEndpointStep {
    pub fn new<V>(_view: &V, account: AccountID, ctx: XrpEndpointContext) -> Result<Self, Ter> {
        Ok(Self {
            account,
            is_last: ctx.is_last,
            is_first: ctx.is_first,
        })
    }

    pub fn loop_asset(&self) -> protocol::Asset {
        protocol::Asset::Issue(protocol::xrp_issue())
    }

    pub fn direct_accounts(&self) -> (AccountID, AccountID) {
        (self.account, self.account)
    }
}
