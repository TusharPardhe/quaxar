//! AccountAssets ported from `xrpld/rpc/detail/AccountAssets.h/the reference source`.
//!
//! Discovers all assets an account can send/receive via trust lines and MPTs.
//! Used by the pathfinder to enumerate available currencies.

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::Arc;

use protocol::{AccountID, Currency, MPTID};

use super::asset_cache::AssetCache;
use super::trust_line::LineDirection;

/// A path asset is either a Currency (from trust lines) or an MPTID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathAsset {
    Currency(Currency),
    Mpt(MPTID),
}

/// Find all assets an account can send (source assets).
///
/// For trust lines: includes currencies where the account has positive balance
/// OR where the peer extends credit and there is credit remaining.
/// For MPTs: includes tokens where balance > 0 and not maxed out.
///
/// Mirrors `accountSourceAssets()` from reference.
pub fn account_source_assets(
    account: &AccountID,
    cache: &AssetCache,
    include_xrp: bool,
) -> HashSet<PathAsset> {
    let mut assets = HashSet::new();

    if include_xrp {
        assets.insert(PathAsset::Currency(protocol::xrp_currency()));
    }

    if let Some(lines) = cache.get_ripple_lines(account, LineDirection::Outgoing) {
        for entry in lines.iter() {
            let balance = entry.get_balance();
            // Include if: positive balance (have IOUs to send)
            // OR peer extends credit and there's credit left
            if balance.signum() > 0
                || (entry.get_limit_peer().signum() > 0 && {
                    let mut n = balance.clone();
                    n.negate();
                    n
                } < *entry.get_limit_peer())
            {
                if let Some(currency) = Some(balance.issue().currency) {
                    assets.insert(PathAsset::Currency(currency.clone()));
                }
            }
        }
    }

    // Remove bad currency (currency code 0x0000...0001 in the reference)
    assets.remove(&PathAsset::Currency(Currency::default()));

    if let Some(mpts) = cache.get_mpts(account) {
        for entry in mpts.iter() {
            if !entry.is_zero_balance() && !entry.is_maxed_out() {
                assets.insert(PathAsset::Mpt(entry.mpt_id().clone()));
            }
        }
    }

    assets
}

/// Find all assets an account can receive (destination assets).
///
/// For trust lines: includes currencies where balance < limit (can take more).
/// For MPTs: includes tokens where balance == 0 and not maxed out.
///
/// Mirrors `accountDestAssets()` from reference.
pub fn account_dest_assets(
    account: &AccountID,
    cache: &AssetCache,
    include_xrp: bool,
) -> HashSet<PathAsset> {
    let mut assets = HashSet::new();

    if include_xrp {
        assets.insert(PathAsset::Currency(protocol::xrp_currency()));
    }

    if let Some(lines) = cache.get_ripple_lines(account, LineDirection::Outgoing) {
        for entry in lines.iter() {
            let balance = entry.get_balance();
            // Can take more if balance < limit
            if balance < entry.get_limit() {
                if let Some(currency) = Some(balance.issue().currency) {
                    assets.insert(PathAsset::Currency(currency.clone()));
                }
            }
        }
    }

    // Remove bad currency
    assets.remove(&PathAsset::Currency(Currency::default()));

    if let Some(mpts) = cache.get_mpts(account) {
        for entry in mpts.iter() {
            if entry.is_zero_balance() && !entry.is_maxed_out() {
                assets.insert(PathAsset::Mpt(entry.mpt_id().clone()));
            }
        }
    }

    assets
}
