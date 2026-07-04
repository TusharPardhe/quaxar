//! Full OfferCreate transactor — reference the reference implementation parity.
//!
//! Handles:
//! - Offer cancellation (sfOfferSequence)
//! - Expiration check
//! - Tick size rounding
//! - DEX crossing via flow engine (flowCross)
//! - Residual offer placement with book directory
//! - FillOrKill / ImmediateOrCancel
//! - Reserve check before placement
//! - Sell flag (accept more than specified)
//! - Owner count adjustment

use basics::math::base_uint::Uint160;
use protocol::{
    AccountID, Amounts, Quality, STAmount, STLedgerEntry, STTx, Ter, XRPAmount,
    get_field_by_symbol, is_tes_success,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

/// C++ parity: isGlobalFrozen for IOU assets. Checks if the issuer has lsfGlobalFreeze set.
fn is_global_frozen_iou<V: ledger::ApplyView>(view: &mut V, issuer: &AccountID) -> bool {
    let keylet = protocol::account_keylet(Uint160::from_void(issuer.data()));
    view.peek(keylet)
        .ok()
        .flatten()
        .is_some_and(|sle| sle.is_flag(protocol::lsfGlobalFreeze))
}

fn check_mpt_offer_global_and_trade_allowed<V: ledger::ApplyView>(
    view: &V,
    asset: protocol::Asset,
) -> Ter {
    let protocol::Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };

    if ledger::mptoken_helpers::is_global_frozen_mpt(view, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }

    ledger::mptoken_helpers::can_trade(view, &asset).unwrap_or(Ter::TEF_INTERNAL)
}

fn check_mpt_offer_accept_asset_allowed<V: ledger::ApplyView>(
    view: &V,
    account: &AccountID,
    asset: protocol::Asset,
) -> Ter {
    let protocol::Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };

    let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
        .unwrap_or(Ter::TEF_INTERNAL);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }
    if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }
    Ter::TES_SUCCESS
}

const TF_PASSIVE: u32 = 0x0001_0000;
const TF_IMMEDIATE_OR_CANCEL: u32 = 0x0002_0000;
const TF_FILL_OR_KILL: u32 = 0x0004_0000;
const TF_SELL: u32 = 0x0008_0000;

/// Full reference OfferCreate::doApply parity.
pub fn do_offer_create<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let account = sttx.get_account_id(sf("sfAccount"));
    let tx_flags = sttx.get_field_u32(sf("sfFlags"));
    let mut taker_pays = sttx.get_field_amount(sf("sfTakerPays"));
    let mut taker_gets = sttx.get_field_amount(sf("sfTakerGets"));

    // (XRP-for-XRP or same IOU issuer+currency)
    if taker_pays.native() && taker_gets.native() {
        return Ter::TEM_BAD_OFFER;
    }
    if !taker_pays.native() && !taker_gets.native() && taker_pays.asset() == taker_gets.asset() {
        return Ter::TEM_BAD_OFFER;
    }

    if taker_pays.signum() <= 0 || taker_gets.signum() <= 0 {
        return Ter::TEM_BAD_OFFER;
    }

    let mpt_allowed = check_mpt_offer_global_and_trade_allowed(view, taker_pays.asset());
    if mpt_allowed != Ter::TES_SUCCESS {
        return mpt_allowed;
    }
    let mpt_allowed = check_mpt_offer_global_and_trade_allowed(view, taker_gets.asset());
    if mpt_allowed != Ter::TES_SUCCESS {
        return mpt_allowed;
    }
    let mpt_allowed = check_mpt_offer_accept_asset_allowed(view, &account, taker_pays.asset());
    if mpt_allowed != Ter::TES_SUCCESS {
        return mpt_allowed;
    }

    // C++ parity: checkGlobalFrozen for IOU assets (preclaim check)
    if let protocol::Asset::Issue(issue) = taker_pays.asset() {
        if !issue.native() && is_global_frozen_iou(view, &issue.account) {
            return Ter::TEC_FROZEN;
        }
    }
    if let protocol::Asset::Issue(issue) = taker_gets.asset() {
        if !issue.native() && is_global_frozen_iou(view, &issue.account) {
            return Ter::TEC_FROZEN;
        }
    }

    let is_passive = (tx_flags & TF_PASSIVE) != 0;
    let is_ioc = (tx_flags & TF_IMMEDIATE_OR_CANCEL) != 0;
    let is_fok = (tx_flags & TF_FILL_OR_KILL) != 0;
    let is_sell = (tx_flags & TF_SELL) != 0;
    let is_hybrid = (tx_flags & protocol::tfHybrid) != 0;
    let domain_id = sttx
        .is_field_present(sf("sfDomainID"))
        .then(|| sttx.get_field_h256(sf("sfDomainID")));

    if is_hybrid && domain_id.is_none() {
        return Ter::TEM_INVALID_FLAG;
    }
    if domain_id.is_some()
        && !view
            .rules()
            .enabled(&protocol::feature_id("PermissionedDEX"))
    {
        return Ter::TEM_DISABLED;
    }

    // Get offer sequence (for the new offer's key)
    let offer_sequence = sttx.get_seq_value();

    let mut result = Ter::TES_SUCCESS;
    let mut freed_taker_gets: Option<STAmount> = None;

    // --- Cancel existing offer if OfferSequence present ---
    if sttx.is_field_present(sf("sfOfferSequence")) {
        let cancel_seq = sttx.get_field_u32(sf("sfOfferSequence"));
        let cancel_keylet = protocol::offer_keylet(Uint160::from_void(account.data()), cancel_seq);
        if let Ok(Some(old_offer)) = view.peek(cancel_keylet) {
            let released_gets = old_offer.get_field_amount(sf("sfTakerGets"));
            result = offer_delete(view, &account, old_offer);
            if is_tes_success(result) {
                freed_taker_gets = Some(released_gets);
            }
        }
    }

    // --- Expiration check ---
    if sttx.is_field_present(sf("sfExpiration")) {
        let expiration = sttx.get_field_u32(sf("sfExpiration"));
        let close_time = view.header().close_time;
        if close_time >= expiration {
            return Ter::TEC_EXPIRED;
        }
    }

    if !is_tes_success(result) {
        return result;
    }

    // --- Tick size rounding ---
    // reference: round offer to tick size of the issuer accounts
    let tick_size = get_tick_size(view, &taker_pays, &taker_gets);
    if tick_size < 15 {
        // reference: auto const rate = Quality{saTakerGets, saTakerPays}.round(uTickSize).rate();
        // Quality is stored as (exponent << 56) | mantissa = getRate(taker_gets, taker_pays)
        let quality = get_rate(&taker_gets, &taker_pays);
        let rounded_quality = round_quality(quality, tick_size);
        // Convert rounded quality back to a rate STAmount for multiply/divide
        let rate_amount = quality_to_rate_amount(rounded_quality, &taker_pays, &taker_gets);

        if is_sell {
            // reference: saTakerPays = multiply(saTakerGets, rate, saTakerPays.asset())
            if let Some(ref rate_amt) = rate_amount {
                taker_pays = taker_gets.multiply(rate_amt, taker_pays.asset());
            }
        } else {
            // reference: saTakerGets = divide(saTakerPays, rate, saTakerGets.asset())
            if let Some(ref rate_amt) = rate_amount {
                taker_gets = taker_pays.divide(rate_amt, taker_gets.asset());
            }
        }
        if taker_pays.signum() <= 0 || taker_gets.signum() <= 0 {
            return Ter::TES_SUCCESS; // Rounded to zero
        }
    }

    // --- reference preclaim: tecUNFUNDED_OFFER check ---
    {
        let account_funds = get_account_funds_for_offer(
            view,
            &account,
            &taker_pays,
            &taker_gets,
            freed_taker_gets.as_ref(),
        );
        static UNFUNDED_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if account_funds.signum() <= 0 {
            if UNFUNDED_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
                tracing::debug!(target: "tx",
                    "[offer_debug] UNFUNDED_OFFER: funds_signum={} taker_gets_native={} freed={:?}",
                    account_funds.signum(),
                    taker_gets.native(),
                    freed_taker_gets.as_ref().map(|f| f.signum())
                );
            }
            return Ter::TEC_UNFUNDED_OFFER;
        }
    }

    // It does NOT prevent crossing. FOK+Passive and IOC+Passive proceed to crossing
    // and apply FOK/IOC rules after. Only kill early if passive AND no crossing is
    // possible at all (i.e., the offer would not cross any existing offers).
    // We do NOT early-kill here — let the crossing loop run and apply FOK/IOC after.

    // --- DEX crossing via flow engine (reference flowCross calls flow()) ---
    // For passive offers, reference increments threshold so only strictly better offers are crossed.
    let mut quality_threshold =
        Quality::from_amounts(&Amounts::new(taker_pays.clone(), taker_gets.clone()));
    if is_passive {
        // offers do not cross. Quality::increment preserves stored XRPL
        // quality ordering instead of approximating with floating point.
        quality_threshold.increment();
    }

    let mut crossed = false;
    let (remaining_gets, remaining_pays) = if !is_passive {
        // a payment from taker to themselves through the order book.
        // takerAmount.in = TakerGets (what taker gives), takerAmount.out = TakerPays (what taker receives)
        // The taker pays TakerPays (what they give) and receives TakerGets (what they get).
        // deliver = TakerGets (what the taker wants to receive)
        // sendMax = TakerPays (what the taker is willing to pay)
        let deliver_asset = taker_gets.asset(); // deliver = takerAmount.out
        let send_max_asset = taker_pays.asset(); // sendMax = takerAmount.in

        let mut cross_paths = protocol::STPathSet::new(sf("sfPaths"));
        if !taker_gets.native() && !taker_pays.native() {
            let mut xrp_path = protocol::STPath::new();
            xrp_path.push_back(protocol::STPathElement::inferred(
                protocol::AccountID::default(),
                protocol::xrp_currency(),
                protocol::AccountID::default(),
                false,
            ));
            cross_paths.push_back(xrp_path);
        }

        // Build strands for crossing (reference toStrands with offerCrossing=true)
        let (_, strands) = ledger::flow_engine::strand_builder::to_strands(
            &account,
            &account, // src == dst for offer crossing
            &deliver_asset,
            Some(&send_max_asset),
            &cross_paths,
            true, // default paths allowed
            true, // owner pays transfer fee
            true, // offer crossing
        );

        let cross_book = Some((taker_gets.asset(), taker_pays.asset()));

        // Execute strands
        // reference: flow(deliver=TakerGets, sendMax=TakerPays)
        let mut used_direct_book_fallback = false;
        let mut flow_result = if !strands.is_empty() {
            ledger::flow_engine::strand_flow::execute_strands(
                view,
                &strands,
                &taker_gets, // deliver = TakerGets
                (tx_flags & TF_FILL_OR_KILL) == 0,
                Some(&taker_pays), // sendMax = TakerPays
                &account,
                Some(quality_threshold),
            )
        } else if let Some((book_in, book_out)) = cross_book {
            let cross_book = ledger::ripple_calc::book_step::Book {
                r#in: book_in,
                out: book_out,
                domain: None,
            };
            // Fallback to direct book step if strand building fails
            let result = ledger::ripple_calc::book_step::execute_book_step(
                view,
                &cross_book,
                &taker_gets,
                &taker_pays,
                true,
                Some(&account),
                Some(quality_threshold),
            );
            ledger::flow_engine::FlowResult {
                ter: result.ter,
                actual_in: result.amount_in,
                actual_out: result.amount_out,
            }
        } else {
            ledger::flow_engine::FlowResult {
                ter: Ter::TEC_PATH_DRY,
                actual_in: taker_pays.zeroed(),
                actual_out: taker_gets.zeroed(),
            }
        };

        if !strands.is_empty()
            && flow_result.actual_in.signum() == 0
            && flow_result.actual_out.signum() == 0
            && let Some((book_in, book_out)) = cross_book
        {
            let cross_book = ledger::ripple_calc::book_step::Book {
                r#in: book_in,
                out: book_out,
                domain: None,
            };
            let result = ledger::ripple_calc::book_step::execute_book_step(
                view,
                &cross_book,
                &taker_gets,
                &taker_pays,
                true,
                Some(&account),
                Some(quality_threshold),
            );
            if result.amount_in.signum() > 0 || result.amount_out.signum() > 0 {
                used_direct_book_fallback = true;
                flow_result = ledger::flow_engine::FlowResult {
                    ter: result.ter,
                    actual_in: result.amount_in,
                    actual_out: result.amount_out,
                };
            }
        }

        let actual_in = flow_result.actual_in;
        let actual_out = flow_result.actual_out;

        // even after fee deduction), propagate that directly — do not override with tecKILLED.
        // This matches reference the reference source:359 where flowCross returns {tecUNFUNDED_OFFER, takerAmount}.
        if flow_result.ter == Ter::TEC_UNFUNDED_OFFER
            && actual_in.signum() == 0
            && actual_out.signum() == 0
        {
            return Ter::TEC_UNFUNDED_OFFER;
        }

        if actual_in.signum() > 0 || actual_out.signum() > 0 {
            crossed = true;

            // Manual taker transfers are only needed for the FALLBACK path
            // (direct execute_book_step). When the flow engine succeeds via
            // strands, the strand execution (DirectStep/XRPEndpointStep) already
            // handles the taker's asset movement.
            if strands.is_empty() || used_direct_book_fallback {
                // Fallback path: book step only handled offer owners' side.
                // Transfer assets to/from the taker:
                if actual_in.signum() > 0 {
                    if actual_in.native() {
                        let acct_k = protocol::account_keylet(Uint160::from_void(account.data()));
                        if let Ok(Some(sle)) = view.peek(acct_k) {
                            let bal = sle.get_field_amount(sf("sfBalance")).xrp().drops();
                            let mut obj = sle.clone_as_object();
                            obj.set_field_amount(
                                sf("sfBalance"),
                                STAmount::from_xrp_amount(XRPAmount::from_drops(
                                    bal - actual_in.xrp().drops(),
                                )),
                            );
                            let _ = view
                                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
                        }
                    } else {
                        let issue = actual_in.issue();
                        ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(
                            view,
                            &account,
                            &issue.account,
                            &actual_in,
                        );
                    }
                }
                if actual_out.signum() > 0 {
                    if actual_out.native() {
                        let acct_k = protocol::account_keylet(Uint160::from_void(account.data()));
                        if let Ok(Some(sle)) = view.peek(acct_k) {
                            let bal = sle.get_field_amount(sf("sfBalance")).xrp().drops();
                            let mut obj = sle.clone_as_object();
                            obj.set_field_amount(
                                sf("sfBalance"),
                                STAmount::from_xrp_amount(XRPAmount::from_drops(
                                    bal + actual_out.xrp().drops(),
                                )),
                            );
                            let _ = view
                                .update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
                        }
                    } else {
                        let issue = actual_out.issue();
                        ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(
                            view,
                            &issue.account,
                            &account,
                            &actual_out,
                        );
                    }
                }
            }
        }

        // Compute remaining offer after crossing — reference flowCross afterCross computation.
        // After swap: actual_in = TakerPays consumed (IOU), actual_out = TakerGets consumed (XRP)
        let (rem_gets, rem_pays) = if is_sell {
            // tfSell: remaining from INPUT side (TakerPays/IOU)
            let gateway_rate = if !taker_pays.native() && account != taker_pays.issue().account {
                ledger::ripple_state_helpers::transfer_rate(view, &taker_pays.issue().account)
            } else {
                1_000_000_000u32
            };
            let non_gateway_in = if gateway_rate != 1_000_000_000 {
                actual_in.divide(
                    &STAmount::new_with_asset(
                        sf("sfAmount"),
                        protocol::Asset::Issue(protocol::Issue::default()),
                        gateway_rate as u64,
                        -9,
                        false,
                    ),
                    taker_pays.asset(),
                )
            } else {
                actual_in
            };
            let mut rem_pays = taker_pays.clone() - non_gateway_in;
            if rem_pays.signum() < 0 {
                rem_pays.clear();
            }
            let rem_gets = if rem_pays.signum() <= 0 {
                taker_gets.zeroed()
            } else {
                rem_pays
                    .multiply(&taker_gets, taker_gets.asset())
                    .divide(&taker_pays, taker_gets.asset())
            };
            (rem_gets, rem_pays)
        } else {
            // Non-sell: remaining from OUTPUT side (TakerGets/XRP)
            let mut rem_gets = taker_gets.clone() - actual_out;
            if rem_gets.signum() < 0 {
                rem_gets.clear();
            }
            let rem_pays = if rem_gets.signum() <= 0 {
                taker_pays.zeroed()
            } else {
                rem_gets
                    .multiply(&taker_pays, taker_pays.asset())
                    .divide(&taker_gets, taker_pays.asset())
            };
            (rem_gets, rem_pays)
        };
        (rem_gets, rem_pays)
    } else {
        // Passive offer — no crossing
        (taker_gets.clone(), taker_pays.clone())
    };

    // --- Fully crossed check ---
    if remaining_gets.signum() <= 0 || remaining_pays.signum() <= 0 {
        return Ter::TES_SUCCESS; // Fully crossed
    }

    // --- FillOrKill check ---
    if is_fok {
        return Ter::TEC_KILLED;
    }

    // --- ImmediateOrCancel check ---
    if is_ioc {
        if !crossed {
            return Ter::TEC_KILLED;
        }
        return Ter::TES_SUCCESS;
    }

    // --- Reserve check before placing ---
    let acct_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let Some(acct_sle) = view.peek(acct_keylet).ok().flatten() else {
        return Ter::TEF_INTERNAL;
    };
    let owner_count = acct_sle.get_field_u32(sf("sfOwnerCount"));
    let reserve = view.fees().account_reserve(owner_count as usize + 1);
    let balance = pre_fee_balance_drops
        .unwrap_or_else(|| acct_sle.get_field_amount(sf("sfBalance")).xrp().drops());
    if balance < reserve as i64 {
        if !crossed {
            return Ter::TEC_INSUF_RESERVE_OFFER;
        }
        // If crossed, allow it (reference behavior)
        return Ter::TES_SUCCESS;
    }

    // --- Place the remaining offer ---
    let offer_keylet = protocol::offer_keylet(Uint160::from_void(account.data()), offer_sequence);

    // Add to owner directory. reference the reference source: owner directory uses dirInsert,
    // while the book directory below uses dirAppend.
    let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
    let owner_node = match ledger::dir_insert(view, &owner_dir, offer_keylet.key, &|_| {}) {
        Ok(Some(page)) => page,
        other => {
            static DIR_FULL_LOG: std::sync::atomic::AtomicU32 =
                std::sync::atomic::AtomicU32::new(0);
            if DIR_FULL_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
                tracing::debug!(target: "tx",
                    "[offer_debug] DIR_FULL owner_dir: dir_key={:02x}{:02x}{:02x}{:02x} result={:?} crossed={} remaining_gets_signum={} remaining_pays_signum={}",
                    owner_dir.key.data()[0],
                    owner_dir.key.data()[1],
                    owner_dir.key.data()[2],
                    owner_dir.key.data()[3],
                    other.as_ref().map(|_| "ok").unwrap_or("err"),
                    crossed,
                    remaining_gets.signum(),
                    remaining_pays.signum()
                );
            }
            return Ter::TEC_DIR_FULL;
        }
    };

    // Adjust owner count
    let _ = ledger::adjust_owner_count(view, &acct_sle, 1);

    // Add to book directory
    let book = protocol::Book {
        r#in: taker_pays.asset(),
        out: taker_gets.asset(),
        domain: domain_id,
    };
    let book_base = protocol::book_keylet(book);
    let rate = get_rate(&taker_gets, &taker_pays);
    let quality_dir = protocol::quality_keylet(book_base, rate);

    let book_node = match ledger::dir_append(view, &quality_dir, offer_keylet.key, &|sle| {
        set_book_directory_fields(sle, &taker_pays, &taker_gets, rate, domain_id);
    }) {
        Ok(Some(page)) => page,
        other => {
            static BOOK_DIR_FULL_LOG: std::sync::atomic::AtomicU32 =
                std::sync::atomic::AtomicU32::new(0);
            if BOOK_DIR_FULL_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 20 {
                tracing::debug!(target: "tx",
                    "[offer_debug] DIR_FULL book_dir: dir_key={:02x}{:02x}{:02x}{:02x} result={:?} crossed={} remaining_gets_signum={} remaining_pays_signum={}",
                    quality_dir.key.data()[0],
                    quality_dir.key.data()[1],
                    quality_dir.key.data()[2],
                    quality_dir.key.data()[3],
                    other.as_ref().map(|_| "ok").unwrap_or("err"),
                    crossed,
                    remaining_gets.signum(),
                    remaining_pays.signum()
                );
            }
            return Ter::TEC_DIR_FULL;
        }
    };

    // Create the offer SLE
    let mut offer_obj = protocol::STObject::new(sf("sfLedgerEntry"));
    offer_obj.set_field_u16(sf("sfLedgerEntryType"), 0x006F); // ltOFFER
    offer_obj.set_account_id(sf("sfAccount"), account);
    offer_obj.set_field_u32(sf("sfSequence"), offer_sequence);
    offer_obj.set_field_h256(sf("sfBookDirectory"), quality_dir.key);
    offer_obj.set_field_amount(sf("sfTakerPays"), remaining_pays);
    offer_obj.set_field_amount(sf("sfTakerGets"), remaining_gets);
    offer_obj.set_field_u64(sf("sfOwnerNode"), owner_node);
    offer_obj.set_field_u64(sf("sfBookNode"), book_node);
    if let Some(domain_id) = domain_id {
        offer_obj.set_field_h256(sf("sfDomainID"), domain_id);
    }

    if sttx.is_field_present(sf("sfExpiration")) {
        offer_obj.set_field_u32(sf("sfExpiration"), sttx.get_field_u32(sf("sfExpiration")));
    }

    let mut offer_flags = 0u32;
    if is_passive {
        offer_flags |= 0x0001_0000; // lsfPassive
    }
    if is_sell {
        offer_flags |= 0x0002_0000; // lsfSell
    }
    if is_hybrid {
        offer_flags |= protocol::lsfHybrid;
    }
    offer_obj.set_field_u32(sf("sfFlags"), offer_flags);

    if is_hybrid {
        let open_rate = if view
            .rules()
            .enabled(&protocol::feature_id(protocol::FIX_CLEANUP_3_2_0_NAME))
        {
            rate
        } else {
            get_rate(
                &offer_obj.get_field_amount(sf("sfTakerGets")),
                &offer_obj.get_field_amount(sf("sfTakerPays")),
            )
        };
        let open_book = protocol::Book {
            r#in: taker_pays.asset(),
            out: taker_gets.asset(),
            domain: None,
        };
        let open_quality_dir =
            protocol::quality_keylet(protocol::book_keylet(open_book), open_rate);
        let open_book_node =
            match ledger::dir_append(view, &open_quality_dir, offer_keylet.key, &|sle| {
                // The legacy open-book key may use post-crossing quality, but
                // C++ still records the original placement rate in metadata.
                set_book_directory_fields(sle, &taker_pays, &taker_gets, rate, None);
            }) {
                Ok(Some(page)) => page,
                _ => return Ter::TEC_DIR_FULL,
            };

        let mut additional_books = protocol::STArray::new(sf("sfAdditionalBooks"));
        let mut book_info = protocol::STObject::make_inner_object(sf("sfBook"));
        book_info.set_field_h256(sf("sfBookDirectory"), open_quality_dir.key);
        book_info.set_field_u64(sf("sfBookNode"), open_book_node);
        additional_books.push_back(book_info);
        offer_obj.set_field_array(sf("sfAdditionalBooks"), additional_books);
    }

    let offer_sle = STLedgerEntry::from_stobject(offer_obj, offer_keylet.key);
    let _ = view.insert(Arc::new(offer_sle));

    Ter::TES_SUCCESS
}

fn set_book_directory_fields(
    sle: &mut protocol::STObject,
    taker_pays: &STAmount,
    taker_gets: &STAmount,
    rate: u64,
    domain_id: Option<protocol::Domain>,
) {
    match taker_pays.asset() {
        protocol::Asset::Issue(issue) if !issue.native() => {
            sle.set_field_h160(
                sf("sfTakerPaysCurrency"),
                Uint160::from_void(issue.currency.data()),
            );
            sle.set_field_h160(
                sf("sfTakerPaysIssuer"),
                Uint160::from_void(issue.account.data()),
            );
        }
        protocol::Asset::MPTIssue(issue) => {
            sle.set_field_h192(sf("sfTakerPaysMPT"), issue.mpt_id());
        }
        _ => {}
    }
    match taker_gets.asset() {
        protocol::Asset::Issue(issue) if !issue.native() => {
            sle.set_field_h160(
                sf("sfTakerGetsCurrency"),
                Uint160::from_void(issue.currency.data()),
            );
            sle.set_field_h160(
                sf("sfTakerGetsIssuer"),
                Uint160::from_void(issue.account.data()),
            );
        }
        protocol::Asset::MPTIssue(issue) => {
            sle.set_field_h192(sf("sfTakerGetsMPT"), issue.mpt_id());
        }
        _ => {}
    }
    sle.set_field_u64(sf("sfExchangeRate"), rate);
    if let Some(domain_id) = domain_id {
        sle.set_field_h256(sf("sfDomainID"), domain_id);
    }
}

/// Delete an offer — remove from owner dir, book dir, and erase SLE.
/// Returns zero if frozen, unauthorized, or no balance.
fn get_account_funds_for_offer<V: ledger::ApplyView>(
    view: &mut V,
    account: &AccountID,
    _taker_pays: &STAmount,
    taker_gets: &STAmount,
    freed_taker_gets: Option<&STAmount>,
) -> STAmount {
    let mut funds = if taker_gets.native() {
        let acct_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
        if let Ok(Some(sle)) = view.peek(acct_keylet) {
            let balance = sle.get_field_amount(sf("sfBalance")).xrp().drops();
            let owner_count = sle.get_field_u32(sf("sfOwnerCount"));
            let reserve = view.fees().account_reserve(owner_count as usize) as i64;
            let available = balance - reserve;
            if available > 0 {
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(available))
            } else {
                STAmount::default()
            }
        } else {
            STAmount::default()
        }
    } else {
        match taker_gets.asset() {
            protocol::Asset::Issue(issue) => {
                if *account == issue.account {
                    taker_gets.clone()
                } else {
                    // return zero if the trust line or issuer is frozen.
                    if ledger::ripple_state_helpers::is_frozen(view, account, &issue) {
                        taker_gets.zeroed()
                    } else {
                        ledger::ripple_state_helpers::credit_balance(
                            view,
                            account,
                            &issue.account,
                            issue.currency,
                        )
                    }
                }
            }
            protocol::Asset::MPTIssue(issue) => {
                if issue.issuer() == *account {
                    view.read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                        .ok()
                        .flatten()
                        .map(|issuance| {
                            STAmount::from_mpt_amount(
                                sf("sfAmount"),
                                protocol::MPTAmount::from_value(
                                    ledger::mptoken_helpers::available_mpt_amount(&issuance),
                                ),
                                issue,
                            )
                        })
                        .unwrap_or_else(|| taker_gets.zeroed())
                } else if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue)
                    .unwrap_or(true)
                    || ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
                        .unwrap_or(Ter::TEF_INTERNAL)
                        != Ter::TES_SUCCESS
                {
                    taker_gets.zeroed()
                } else {
                    view.read(protocol::mptoken_keylet_from_mptid(
                        issue.mpt_id(),
                        Uint160::from_void(account.data()),
                    ))
                    .ok()
                    .flatten()
                    .map(|token| {
                        STAmount::from_mpt_amount(
                            sf("sfAmount"),
                            protocol::MPTAmount::from_value(
                                token.get_field_u64(sf("sfMPTAmount")) as i64
                            ),
                            issue,
                        )
                    })
                    .unwrap_or_else(|| taker_gets.zeroed())
                }
            }
        }
    };

    if let Some(released) = freed_taker_gets
        && released.asset() == taker_gets.asset()
        && released.signum() > 0
    {
        funds += released.clone();
    }

    static FUNDS_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if FUNDS_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 50 {
        tracing::debug!(target: "tx",
            "[offer_funds] funds_signum={} taker_gets_native={} freed_signum={:?} acct={:02x}{:02x}{:02x}{:02x}",
            funds.signum(),
            taker_gets.native(),
            freed_taker_gets.map(|f| f.signum()),
            account.data()[0],
            account.data()[1],
            account.data()[2],
            account.data()[3],
        );
    }

    funds
}

pub fn offer_delete_pub<V: ledger::ApplyView>(
    view: &mut V,
    account: &AccountID,
    offer_sle: Arc<STLedgerEntry>,
) -> Ter {
    offer_delete(view, account, offer_sle)
}

fn offer_delete<V: ledger::ApplyView>(
    view: &mut V,
    account: &AccountID,
    offer_sle: Arc<STLedgerEntry>,
) -> Ter {
    let _ = account;
    ledger::offer_helpers::offer_delete(view, offer_sle).unwrap_or(Ter::TEF_BAD_LEDGER)
}

/// Returns the exchange rate encoded as u64: top 8 bits = exponent+100, lower 56 bits = mantissa.
/// reference: getRate(offerOut=taker_gets, offerIn=taker_pays) = divide(taker_pays, taker_gets) encoded.
fn get_rate(taker_gets: &STAmount, taker_pays: &STAmount) -> u64 {
    if taker_gets.signum() <= 0 {
        return 0;
    }
    // reference: STAmount r = divide(offerIn, offerOut, noIssue())
    // offerIn = taker_pays, offerOut = taker_gets
    let no_issue = protocol::Issue::default();
    let r = taker_pays.divide(taker_gets, no_issue);
    if r.signum() <= 0 {
        return 0;
    }
    // reference: (r.exponent() + 100) << 56 | r.mantissa()
    let exp = r.exponent() + 100;
    if !(0..=255).contains(&exp) {
        return 0;
    }
    ((exp as u64) << 56) | r.mantissa()
}

/// Get tick size from issuer accounts.
fn get_tick_size<V: ledger::ApplyView>(
    view: &V,
    taker_pays: &STAmount,
    taker_gets: &STAmount,
) -> u8 {
    let mut tick_size: u8 = 15; // Quality::kMAX_TICK_SIZE

    // Check pays issuer
    if let protocol::Asset::Issue(issue) = taker_pays.asset()
        && !issue.native()
    {
        let issuer = issue.account;
        let issuer_keylet = protocol::account_keylet(Uint160::from_void(issuer.data()));
        if let Ok(Some(sle)) = view.read(issuer_keylet) {
            if sle.is_field_present(sf("sfTickSize")) {
                let ts = sle.get_field_u8(sf("sfTickSize"));
                if ts < tick_size {
                    tick_size = ts;
                }
            }
        }
    }

    // Check gets issuer
    if let protocol::Asset::Issue(issue) = taker_gets.asset()
        && !issue.native()
    {
        let issuer = issue.account;
        let issuer_keylet = protocol::account_keylet(Uint160::from_void(issuer.data()));
        if let Ok(Some(sle)) = view.read(issuer_keylet) {
            if sle.is_field_present(sf("sfTickSize")) {
                let ts = sle.get_field_u8(sf("sfTickSize"));
                if ts < tick_size {
                    tick_size = ts;
                }
            }
        }
    }

    tick_size
}

/// Quality is encoded as (exponent << 56) | mantissa.
/// Rounding is UP (adds kMOD[digits]-1 before truncating).
fn round_quality(quality: u64, digits: u8) -> u64 {
    if quality == 0 || digits >= 16 {
        return quality;
    }
    static K_MOD: [u64; 17] = [
        10000000000000000, // 0
        1000000000000000,  // 1
        100000000000000,   // 2
        10000000000000,    // 3
        1000000000000,     // 4
        100000000000,      // 5
        10000000000,       // 6
        1000000000,        // 7
        100000000,         // 8
        10000000,          // 9
        1000000,           // 10
        100000,            // 11
        10000,             // 12
        1000,              // 13
        100,               // 14
        10,                // 15
        1,                 // 16
    ];
    let exponent = quality >> 56;
    let mut mantissa = quality & 0x00ffffffffffffff;
    let modulus = K_MOD[digits as usize];
    mantissa += modulus - 1;
    mantissa -= mantissa % modulus;
    (exponent << 56) | mantissa
}

/// Convert a quality (encoded u64) back to an STAmount rate for multiply/divide.
fn quality_to_rate_amount(quality: u64, _pays: &STAmount, _gets: &STAmount) -> Option<STAmount> {
    if quality == 0 {
        return None;
    }
    let exponent = (quality >> 56) as i32 - 100;
    let mantissa = quality & 0x00ffffffffffffff;
    if mantissa == 0 {
        return None;
    }
    Some(STAmount::new_with_asset(
        sf("sfAmount"),
        protocol::Asset::Issue(protocol::Issue::default()),
        mantissa,
        exponent,
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_book_directory_metadata_can_keep_original_exchange_rate() {
        let issuer = protocol::AccountID::from_array([0x55; 20]);
        let currency = protocol::currency_from_string("USD");
        let taker_pays = STAmount::from_iou_amount(
            sf("sfTakerPays"),
            protocol::IOUAmount::from_parts(100, 0).expect("valid iou"),
            protocol::Issue::new(currency, issuer),
        );
        let taker_gets = STAmount::from_xrp_amount(XRPAmount::from_drops(250));
        let mut dir = protocol::STObject::new(sf("sfLedgerEntry"));

        set_book_directory_fields(&mut dir, &taker_pays, &taker_gets, 42, None);

        assert_eq!(dir.get_field_u64(sf("sfExchangeRate")), 42);
        assert!(!dir.is_field_present(sf("sfDomainID")));
        assert!(dir.is_field_present(sf("sfTakerPaysCurrency")));
        assert!(dir.is_field_present(sf("sfTakerPaysIssuer")));
    }
}
