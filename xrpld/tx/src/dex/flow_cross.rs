//! Core DEX matching loop (flowCross).

use crate::dex::book_step::BookStep;
use protocol::{STAmount, Ter, get_field_by_symbol};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

/// Transfer rate parity — no fee.
const QUALITY_ONE: u32 = 1_000_000_000;

pub struct FlowCrossResult {
    pub taker_pays: STAmount,
    pub taker_gets: STAmount,
    pub ter: Ter,
}

/// Core DEX matching loop matching reference flowCross.
///
/// Iterates through offers in the book, consuming them and tracking
/// total amounts exchanged. Fully consumed offers are erased.
/// Partially consumed offers have their TakerPays/TakerGets updated.
///
/// `transfer_rate_out` is the issuer's transfer rate on the output asset.
/// When non-parity, the taker receives less to account for the transfer fee
/// charged on the outbound leg. This matches the C++ "always charge peer on
/// strand" semantics (PR #7422).
pub fn flow_cross<S: BookStep, V: ledger::ApplyView>(
    view: &mut V,
    book_step: &mut S,
    taker_pays_limit: STAmount,
    taker_gets_limit: STAmount,
    transfer_rate_out: u32,
) -> Result<FlowCrossResult, ledger::ViewError> {
    let mut total_taker_pays = taker_pays_limit.zeroed();
    let mut total_taker_gets = taker_gets_limit.zeroed();

    while total_taker_pays < taker_pays_limit && total_taker_gets < taker_gets_limit {
        let Some(offer_sle) = book_step.next_offer(view)? else {
            break;
        };

        // From the offer maker's perspective:
        // offer TakerGets = what the taker receives (maker sells)
        // offer TakerPays = what the taker pays (maker buys)
        let offer_taker_gets = offer_sle.get_field_amount(sf("sfTakerGets"));
        let offer_taker_pays = offer_sle.get_field_amount(sf("sfTakerPays"));

        if offer_taker_gets.signum() <= 0 || offer_taker_pays.signum() <= 0 {
            // Invalid offer — remove it
            let _ = view.erase(offer_sle);
            continue;
        }

        // How much more the taker wants to get
        let taker_still_gets = taker_gets_limit.clone() - total_taker_gets.clone();
        let taker_still_pays = taker_pays_limit.clone() - total_taker_pays.clone();

        // Calculate how much of this offer we consume
        // actual_gets = min(taker_still_gets, offer_taker_gets)
        let actual_gets = if taker_still_gets < offer_taker_gets {
            taker_still_gets
        } else {
            offer_taker_gets.clone()
        };

        // actual_pays = actual_gets * offer_taker_pays / offer_taker_gets
        // (proportional: if we take X of what they sell, we pay X * their_price)
        let actual_pays = actual_gets
            .multiply(&offer_taker_pays, taker_pays_limit.asset())
            .divide(&offer_taker_gets, taker_pays_limit.asset());

        // Don't exceed what taker is willing to pay
        let (final_gets, final_pays) = if actual_pays > taker_still_pays {
            // Taker can't afford full proportional amount — scale down
            let scaled_gets = taker_still_pays
                .multiply(&offer_taker_gets, taker_gets_limit.asset())
                .divide(&offer_taker_pays, taker_gets_limit.asset());
            (scaled_gets, taker_still_pays)
        } else {
            (actual_gets, actual_pays)
        };

        // Apply transfer fee on the output leg.
        // The owner gives `final_gets` from their balance, but the taker
        // receives less when a transfer rate is set on the output asset issuer.
        // effective_gets = final_gets * QUALITY_ONE / transfer_rate_out
        // This rounds down (taker bears the fee).
        let effective_gets = apply_transfer_fee(&final_gets, transfer_rate_out);

        total_taker_pays += final_pays.clone();
        total_taker_gets += effective_gets.clone();

        // Consume the offer
        if final_gets >= offer_taker_gets {
            // Fully consumed — erase the offer
            let _ = view.erase(offer_sle);
        } else {
            // Partially consumed — update remaining amounts
            let remaining_gets = offer_taker_gets - final_gets;
            let remaining_pays = offer_taker_pays - final_pays;
            let mut obj = offer_sle.clone_as_object();
            obj.set_field_amount(sf("sfTakerGets"), remaining_gets);
            obj.set_field_amount(sf("sfTakerPays"), remaining_pays);
            let _ = view.update(Arc::new(protocol::STLedgerEntry::from_stobject(
                obj,
                *offer_sle.key(),
            )));
            break; // Partial consumption means we're done
        }
    }

    Ok(FlowCrossResult {
        taker_pays: total_taker_pays,
        taker_gets: total_taker_gets,
        ter: Ter::TES_SUCCESS,
    })
}

/// Apply transfer fee to an amount. Returns amount * QUALITY_ONE / rate,
/// rounding down so the taker bears the fee. When rate == QUALITY_ONE (no fee)
/// returns the amount unchanged.
fn apply_transfer_fee(amount: &STAmount, rate: u32) -> STAmount {
    if rate <= QUALITY_ONE {
        return amount.clone();
    }
    if amount.native() {
        let drops = amount.xrp().drops();
        let result = (drops as i128 * QUALITY_ONE as i128) / rate as i128;
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(result as i64))
    } else {
        // IOU/MPT: multiply by (QUALITY_ONE / rate)
        // Use the STAmount divide path for proper IOU mantissa handling.
        let one = STAmount::new_with_asset(
            sf("sfAmount"),
            amount.asset(),
            QUALITY_ONE as u64,
            -9,
            false,
        );
        let rate_amt =
            STAmount::new_with_asset(sf("sfAmount"), amount.asset(), rate as u64, -9, false);
        amount.multiply(&one, amount.asset()).divide(&rate_amt, amount.asset())
    }
}

/// Look up the transfer rate for an asset's issuer from the ledger.
/// Returns QUALITY_ONE for XRP (native) or if no rate is set.
pub fn get_transfer_rate_for_asset<V: ledger::ApplyView>(view: &mut V, asset: protocol::Asset) -> u32 {
    if asset.native() {
        return QUALITY_ONE;
    }
    let issuer = asset.issuer();
    ledger::ripple_state_helpers::transfer_rate(view, &issuer)
}
