//! Core DEX matching loop (flowCross).

use crate::dex::book_step::BookStep;
use crate::dex::mpt_dex;
use protocol::{Asset, STAmount, Ter, get_field_by_symbol, is_tes_success};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

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
/// MPT awareness: when the incoming asset (what the offer owner receives)
/// is an MPT, we create an MPToken for the offer owner if needed and verify
/// authorization. Unauthorized offers are removed without crossing.
pub fn flow_cross<S: BookStep, V: ledger::ApplyView>(
    view: &mut V,
    book_step: &mut S,
    taker_pays_limit: STAmount,
    taker_gets_limit: STAmount,
) -> Result<FlowCrossResult, ledger::ViewError> {
    let mut total_taker_pays = taker_pays_limit.zeroed();
    let mut total_taker_gets = taker_gets_limit.zeroed();

    // Determine if assets are MPT for crossing checks.
    // In the book, offers are stored from the maker's perspective:
    //   offer.TakerGets = what the taker receives (what the maker sells)
    //   offer.TakerPays = what the taker pays (what the maker buys/receives)
    // When crossing, the "incoming" asset to the offer owner is TakerPays
    // (what the taker sends to the maker). This corresponds to taker_gets_limit.asset()
    // in our reversed book perspective.
    let asset_in_to_maker = taker_gets_limit.asset();
    let is_asset_in_mpt = matches!(asset_in_to_maker, Asset::MPTIssue(_));

    while total_taker_pays < taker_pays_limit && total_taker_gets < taker_gets_limit {
        let Some(offer_sle) = book_step.next_offer(view)? else {
            break;
        };

        let offer_taker_gets = offer_sle.get_field_amount(sf("sfTakerGets"));
        let offer_taker_pays = offer_sle.get_field_amount(sf("sfTakerPays"));

        if offer_taker_gets.signum() <= 0 || offer_taker_pays.signum() <= 0 {
            let _ = view.erase(offer_sle);
            continue;
        }

        // MPT crossing check: if the asset flowing into the offer owner is MPT,
        // ensure the owner has an MPToken and is authorized.
        if is_asset_in_mpt {
            let issue = match &asset_in_to_maker {
                Asset::MPTIssue(i) => i,
                _ => unreachable!(),
            };
            let owner = offer_sle.get_account_id(sf("sfAccount"));
            let ter = mpt_dex::check_create_mpt(view, issue, &owner);
            if !is_tes_success(ter) {
                let _ = view.erase(offer_sle);
                continue;
            }
            let auth = mpt_dex::require_mpt_auth(view, issue, &owner);
            if !is_tes_success(auth) {
                let _ = view.erase(offer_sle);
                continue;
            }
        }

        // How much more the taker wants to get
        let taker_still_gets = taker_gets_limit.clone() - total_taker_gets.clone();
        let taker_still_pays = taker_pays_limit.clone() - total_taker_pays.clone();

        // Calculate how much of this offer we consume
        let actual_gets = if taker_still_gets < offer_taker_gets {
            taker_still_gets
        } else {
            offer_taker_gets.clone()
        };

        // actual_pays = actual_gets * offer_taker_pays / offer_taker_gets
        let actual_pays = actual_gets
            .multiply(&offer_taker_pays, taker_pays_limit.asset())
            .divide(&offer_taker_gets, taker_pays_limit.asset());

        // Don't exceed what taker is willing to pay
        let (final_gets, final_pays) = if actual_pays > taker_still_pays {
            let scaled_gets = taker_still_pays
                .multiply(&offer_taker_gets, taker_gets_limit.asset())
                .divide(&offer_taker_pays, taker_gets_limit.asset());
            (scaled_gets, taker_still_pays)
        } else {
            (actual_gets, actual_pays)
        };

        total_taker_pays += final_pays.clone();
        total_taker_gets += final_gets.clone();

        // Consume the offer
        if final_gets >= offer_taker_gets {
            let _ = view.erase(offer_sle);
        } else {
            let remaining_gets = offer_taker_gets - final_gets;
            let remaining_pays = offer_taker_pays - final_pays;
            let mut obj = offer_sle.clone_as_object();
            obj.set_field_amount(sf("sfTakerGets"), remaining_gets);
            obj.set_field_amount(sf("sfTakerPays"), remaining_pays);
            let _ = view.update(Arc::new(protocol::STLedgerEntry::from_stobject(
                obj,
                *offer_sle.key(),
            )));
            break;
        }
    }

    Ok(FlowCrossResult {
        taker_pays: total_taker_pays,
        taker_gets: total_taker_gets,
        ter: Ter::TES_SUCCESS,
    })
}
