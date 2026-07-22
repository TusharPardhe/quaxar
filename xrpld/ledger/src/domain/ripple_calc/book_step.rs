//! Full reference the reference source parity — DEX order book crossing.
//!
//! BookStep iterates offers in an order book and consumes them to convert
//! between two assets. Handles transfer fees, funding limits, quality thresholds.

use std::sync::Arc;

use crate::ApplyView;
use crate::domain::ripple_state_helpers;
use basics;
use basics::base_uint::Uint256;
use protocol::{
    AccountID, Amounts, Asset, MPTAmount, Quality, STAmount, STLedgerEntry, Ter,
    get_field_by_symbol as sf,
};

const MAX_OFFERS_TO_CONSUME: u32 = 2000;
const QUALITY_ONE: u32 = 1_000_000_000;

/// Book: represents an order book (pair of assets to trade)
#[derive(Debug, Clone)]
pub struct Book {
    pub r#in: Asset,
    pub out: Asset,
    pub domain: Option<Uint256>,
}

/// Result of consuming offers from a book
#[derive(Debug, Clone)]
pub struct BookStepResult {
    pub amount_in: STAmount,
    pub amount_out: STAmount,
    pub offers_consumed: u32,
    pub ter: Ter,
}

/// Execute a book step: consume offers from the order book.
pub fn execute_book_step<V: ApplyView>(
    view: &mut V,
    book: &Book,
    max_in: &STAmount,
    max_out: &STAmount,
    owner_pays_transfer_fee: bool,
    taker: Option<&AccountID>,
    // For OfferCreate: minimum quality (TakerGets/TakerPays) offers must meet.
    // Offers with quality < threshold are skipped. None = no threshold (payments).
    quality_threshold: Option<Quality>,
) -> BookStepResult {
    let mut total_in = max_in.zeroed();
    let mut total_out = max_out.zeroed();
    let mut offers_consumed: u32 = 0;
    let mut remaining_in = max_in.clone();

    // Get transfer rates
    // For payment context (owner_pays_transfer_fee=false): tr_in = QUALITY_ONE because
    // the sender→issuer transfer rate is handled separately by the payment wrapper.
    // For OfferCreate context (owner_pays_transfer_fee=true): apply transfer rates,
    // but reference rate(sb, issue, dst) returns QUALITY_ONE when dst == issue.getIssuer().
    // The "dst" in OfferCreate context is the taker (offer creator).
    if let Ok(ter) = crate::mptoken_helpers::can_trade(view, &book.r#in)
        && ter != Ter::TES_SUCCESS
    {
        return BookStepResult {
            amount_in: total_in,
            amount_out: total_out,
            offers_consumed,
            ter,
        };
    }
    if let Ok(ter) = crate::mptoken_helpers::can_trade(view, &book.out)
        && ter != Ter::TES_SUCCESS
    {
        return BookStepResult {
            amount_in: total_in,
            amount_out: total_out,
            offers_consumed,
            ter,
        };
    }

    let strand_deliver = max_out.asset();
    let tr_in = if owner_pays_transfer_fee {
        transfer_rate_for_asset(view, book.r#in, taker, strand_deliver)
    } else {
        QUALITY_ONE
    };
    let tr_out = if owner_pays_transfer_fee {
        transfer_rate_for_asset(view, book.out, taker, strand_deliver)
    } else {
        QUALITY_ONE
    };

    // Iterate offers in the book directory
    // We use get_book_offers which reads from the offer directory.
    let offers = get_book_offers(view, book, MAX_OFFERS_TO_CONSUME);

    // (only crosses offers at the best available quality).
    let mut first_quality: Option<u64> = None;

    for offer_sle in offers {
        if offers_consumed >= MAX_OFFERS_TO_CONSUME || remaining_in.signum() <= 0 {
            break;
        }

        let offer_owner = offer_sle.get_account_id(sf("sfAccount"));
        let taker_pays = offer_sle.get_field_amount(sf("sfTakerPays"));
        let taker_gets = offer_sle.get_field_amount(sf("sfTakerGets"));

        if taker_pays.signum() <= 0 || taker_gets.signum() <= 0 {
            remove_consumed_offer(view, &offer_sle);
            offers_consumed += 1;
            continue;
        }

        // Post-fixCleanup3_3_0: only validate domain membership when walking a
        // domain book. Hybrid offers in the open book are not evicted on
        // credential expiry. Pre-fix: always validate.
        if offer_sle.is_field_present(sf("sfDomainID"))
            && (!view.rules().enabled(&protocol::fix_cleanup_3_3_0()) || book.domain.is_some())
        {
            let offer_domain = offer_sle.get_field_h256(sf("sfDomainID"));
            if !crate::permissioned_dex_helpers::offer_in_domain(
                &*view,
                offer_sle.key(),
                &offer_domain,
            )
            .unwrap_or(false)
            {
                remove_consumed_offer(view, &offer_sle);
                offers_consumed += 1;
                continue;
            }
        }

        // For OfferCreate, skip offers whose quality is worse than the taker's threshold.
        // offer quality = TakerGets/TakerPays (what the offer gives per unit it wants).
        // threshold = taker's TakerGets/TakerPays.
        // Only cross if offer_quality >= threshold.
        if let Some(threshold) = quality_threshold {
            // The taker receives offer's TakerPays and gives offer's TakerGets.
            // So quality = get_rate(offer_TakerPays, offer_TakerGets) — what taker gets per unit given.
            let offer_quality =
                Quality::from_amounts(&Amounts::new(taker_gets.clone(), taker_pays.clone()));
            if offer_quality < threshold {
                break;
            }
        }

        if owner_pays_transfer_fee {
            let offer_quality = if taker_gets.signum() > 0 {
                let no_issue = protocol::Issue::default();
                let q = taker_pays.divide(&taker_gets, no_issue);
                if q.signum() > 0 {
                    let exp = q.exponent() + 100;
                    if (0..=255).contains(&exp) {
                        ((exp as u64) << 56) | q.mantissa()
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                0
            };
            if let Some(fq) = first_quality {
                if offer_quality != fq {
                    break;
                }
            } else {
                first_quality = Some(offer_quality);
            }
        }

        // Check offer owner's funding
        let owner_funds = get_owner_funds(view, &offer_owner, &book.out);
        if owner_funds.signum() <= 0 {
            remove_consumed_offer(view, &offer_sle);
            offers_consumed += 1;
            continue;
        }

        // BookPaymentStep: skip sender's own offers (don't trade with yourself)
        // BookOfferCrossingStep: remove taker's own offers without trading
        if let Some(taker_account) = taker
            && offer_owner == *taker_account
        {
            if owner_pays_transfer_fee {
                // OfferCreate context: remove own offer without trading
                remove_consumed_offer(view, &offer_sle);
            }
            // Payment context: just skip
            offers_consumed += 1;
            continue;
        }

        // Compute consumption amounts with transfer rates (reference forEachOffer parity)
        let consumption = compute_offer_consumption(
            &remaining_in,
            &taker_pays,
            &taker_gets,
            &owner_funds,
            tr_in,
            tr_out,
        );

        if consumption.step_in.signum() <= 0 || consumption.step_out.signum() <= 0 {
            break;
        }

        // Execute trade: transfer assets between offer owner and issuers
        //   offer.send(sb, book_.in.getIssuer(), offer.owner(), ofrAmt.in) — owner receives offer input
        //   offer.send(sb, offer.owner(), book_.out.getIssuer(), ownerGives) — owner pays ownerGives
        let res = execute_offer_trade(
            view,
            &offer_owner,
            &book.r#in,
            &book.out,
            &consumption.offer_in,
            &consumption.owner_gives,
        );
        if res != Ter::TES_SUCCESS {
            remove_consumed_offer(view, &offer_sle);
            offers_consumed += 1;
            continue;
        }

        // Update or remove the offer — reference offer.consume(sb, ofrAmt)
        let new_pays = taker_pays - consumption.offer_in.clone();
        let new_gets = taker_gets - consumption.offer_out.clone();
        if new_pays.signum() <= 0 || new_gets.signum() <= 0 {
            remove_consumed_offer(view, &offer_sle);
        } else {
            let mut obj = offer_sle.clone_as_object();
            obj.set_field_amount(sf("sfTakerPays"), new_pays);
            obj.set_field_amount(sf("sfTakerGets"), new_gets);
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                obj,
                *offer_sle.key(),
            )));
        }

        total_in += consumption.step_in.clone();
        total_out += consumption.step_out.clone();
        remaining_in -= consumption.step_in;
        offers_consumed += 1;
    }

    // If an AMM exists for this book and provides liquidity, use it.
    // The AMM uses the constant product formula (x*y=k).
    // We check AMM after CLOB: if CLOB already delivered enough, skip AMM.
    if remaining_in.signum() > 0 && total_out < *max_out {
        let remaining_out = max_out.clone() - total_out.clone();
        if let Some((amm_account, amm_pays, amm_gets, _fee)) =
            get_amm_offer(view, book, &remaining_in, &remaining_out)
            && amm_pays.signum() > 0
            && amm_gets.signum() > 0
        {
            // Execute AMM trade: taker sends amm_pays, receives amm_gets
            let res = execute_amm_trade(
                view,
                &amm_account,
                &book.r#in,
                &book.out,
                &amm_pays,
                &amm_gets,
            );
            if res == Ter::TES_SUCCESS {
                total_in += amm_pays;
                total_out += amm_gets;
            }
        }
    }

    BookStepResult {
        amount_in: total_in,
        amount_out: total_out,
        offers_consumed,
        ter: Ter::TES_SUCCESS,
    }
}

fn transfer_rate_for_asset<V: ApplyView>(
    view: &mut V,
    asset: Asset,
    dst: Option<&AccountID>,
    strand_deliver: Asset,
) -> u32 {
    match asset {
        Asset::Issue(issue) => {
            if issue.native() || dst.is_some_and(|account| *account == issue.issuer()) {
                QUALITY_ONE
            } else {
                ripple_state_helpers::transfer_rate(view, &issue.issuer())
            }
        }
        Asset::MPTIssue(issue) => {
            if asset == strand_deliver && dst.is_some_and(|account| *account == issue.issuer()) {
                QUALITY_ONE
            } else {
                crate::mptoken_helpers::transfer_rate_mpt(view, issue.mpt_id())
                    .map(|rate| rate.value)
                    .unwrap_or(QUALITY_ONE)
            }
        }
    }
}

// ── AMM (Automated Market Maker) support ─────────────────────────────────────
// The AMM uses the constant product formula: pool_in * pool_out = k.
// swapAssetIn: given input amount, compute output = pool_out - (pool_in * pool_out) / (pool_in + in * (1 - fee))
// swapAssetOut: given output amount, compute input = ((pool_in * pool_out) / (pool_out - out) - pool_in) / (1 - fee)

/// out = pool_out - (pool_in * pool_out) / (pool_in + assetIn * (1 - fee))
fn amm_swap_asset_in(pool_in: f64, pool_out: f64, asset_in: f64, trading_fee: u16) -> f64 {
    if pool_in <= 0.0 || pool_out <= 0.0 || asset_in <= 0.0 {
        return 0.0;
    }
    let fee_mult = 1.0 - (trading_fee as f64) / 100_000.0;
    let denom = pool_in + asset_in * fee_mult;
    if denom <= 0.0 {
        return 0.0;
    }
    let out = pool_out - (pool_in * pool_out) / denom;
    if out < 0.0 { 0.0 } else { out }
}

/// in = ((pool_in * pool_out) / (pool_out - assetOut) - pool_in) / (1 - fee)
fn amm_swap_asset_out(pool_in: f64, pool_out: f64, asset_out: f64, trading_fee: u16) -> f64 {
    if pool_in <= 0.0 || pool_out <= 0.0 || asset_out <= 0.0 {
        return 0.0;
    }
    if asset_out >= pool_out {
        return f64::MAX;
    }
    let fee_mult = 1.0 - (trading_fee as f64) / 100_000.0;
    if fee_mult <= 0.0 {
        return f64::MAX;
    }
    let new_pool_out = pool_out - asset_out;
    let new_pool_in = (pool_in * pool_out) / new_pool_out;
    let asset_in = (new_pool_in - pool_in) / fee_mult;
    if asset_in < 0.0 { 0.0 } else { asset_in }
}

/// Convert STAmount to f64 for AMM arithmetic.
fn amount_to_f64(amount: &STAmount) -> f64 {
    if amount.native() {
        amount.xrp().drops() as f64
    } else {
        amount.mantissa() as f64 * 10f64.powi(amount.exponent())
    }
}

/// Convert f64 back to STAmount with the given asset.
fn f64_to_amount(value: f64, asset: protocol::Asset) -> STAmount {
    if value <= 0.0 {
        return STAmount::new_with_asset(sf("sfAmount"), asset, 0, 0, false);
    }
    if asset.native() {
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(value as i64))
    } else if let Asset::MPTIssue(issue) = asset {
        STAmount::from_mpt_amount(
            sf("sfAmount"),
            MPTAmount::from_value(value.floor() as i64),
            issue,
        )
    } else {
        // Normalize to IOU range [1e15, 1e16)
        let log10 = value.log10().floor() as i32;
        let exponent = log10 - 15;
        let scale = 10f64.powi(-exponent);
        let mantissa = (value * scale).floor() as u64;
        let mantissa = mantissa
            .max(1_000_000_000_000_000)
            .min(9_999_999_999_999_999);
        STAmount::new_with_asset(sf("sfAmount"), asset, mantissa, exponent, false)
    }
}

/// Returns (amm_account, amm_taker_pays, amm_taker_gets, trading_fee) or None.
fn get_amm_offer<V: ApplyView>(
    view: &mut V,
    book: &Book,
    max_in: &STAmount,
    max_out: &STAmount,
) -> Option<(AccountID, STAmount, STAmount, u16)> {
    // Find AMM SLE for this book
    let amm_keylet = protocol::amm(book.r#in, book.out);
    let amm_sle = view.read(amm_keylet).ok()??;

    // Get AMM account
    let amm_account = amm_sle.get_account_id(sf("sfAccount"));

    // Get trading fee
    let trading_fee = amm_sle.get_field_u16(sf("sfTradingFee"));

    // Get pool balances using credit_balance (handles trust line direction correctly)
    let pool_in_amount = if book.r#in.native() {
        // XRP: read AMM account balance
        let acct_kl =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(amm_account.data()));
        let acct_sle = view.read(acct_kl).ok()??;
        acct_sle.get_field_amount(sf("sfBalance"))
    } else if let Asset::MPTIssue(issue) = book.r#in {
        let token = view
            .read(protocol::mptoken_keylet_from_mptid(
                issue.mpt_id(),
                basics::base_uint::Uint160::from_void(amm_account.data()),
            ))
            .ok()??;
        STAmount::from_mpt_amount(
            sf("sfAmount"),
            MPTAmount::from_value(token.get_field_u64(sf("sfMPTAmount")) as i64),
            issue,
        )
    } else {
        let Asset::Issue(issue) = book.r#in else {
            unreachable!("handled above");
        };
        // IOU: credit_balance returns what AMM holds (positive = AMM holds)
        ripple_state_helpers::credit_balance(view, &amm_account, &issue.account, issue.currency)
    };

    let pool_out_amount = if book.out.native() {
        let acct_kl =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(amm_account.data()));
        let acct_sle = view.read(acct_kl).ok()??;
        acct_sle.get_field_amount(sf("sfBalance"))
    } else if let Asset::MPTIssue(issue) = book.out {
        let token = view
            .read(protocol::mptoken_keylet_from_mptid(
                issue.mpt_id(),
                basics::base_uint::Uint160::from_void(amm_account.data()),
            ))
            .ok()??;
        STAmount::from_mpt_amount(
            sf("sfAmount"),
            MPTAmount::from_value(token.get_field_u64(sf("sfMPTAmount")) as i64),
            issue,
        )
    } else {
        let Asset::Issue(issue) = book.out else {
            unreachable!("handled above");
        };
        ripple_state_helpers::credit_balance(view, &amm_account, &issue.account, issue.currency)
    };

    let pool_in = amount_to_f64(&pool_in_amount);
    let pool_out = amount_to_f64(&pool_out_amount);

    if pool_in <= 0.0 || pool_out <= 0.0 {
        return None;
    }

    // Compute AMM offer: given max_in, how much can we get out?
    let in_val = amount_to_f64(max_in);
    let out_val = amm_swap_asset_in(pool_in, pool_out, in_val, trading_fee);

    if out_val <= 0.0 {
        return None;
    }

    // Limit by max_out
    let actual_out = out_val.min(amount_to_f64(max_out));
    let actual_in = if actual_out < out_val {
        amm_swap_asset_out(pool_in, pool_out, actual_out, trading_fee)
    } else {
        in_val
    };

    if actual_in <= 0.0 || actual_out <= 0.0 {
        return None;
    }

    let amm_taker_pays = f64_to_amount(actual_in, book.r#in);
    let amm_taker_gets = f64_to_amount(actual_out, book.out);

    Some((amm_account, amm_taker_pays, amm_taker_gets, trading_fee))
}

/// Execute an AMM swap: update pool balances.
fn execute_amm_trade<V: ApplyView>(
    view: &mut V,
    amm_account: &AccountID,
    book_in: &Asset,
    book_out: &Asset,
    amount_in: &STAmount,  // what taker pays (goes into AMM pool)
    amount_out: &STAmount, // what taker gets (comes out of AMM pool)
) -> Ter {
    // Taker pays amount_in to AMM (AMM receives book_in)
    let res = ripple_state_helpers::account_send(view, &book_in.issuer(), amm_account, amount_in);
    if res != Ter::TES_SUCCESS {
        return res;
    }
    // AMM pays amount_out to taker (AMM sends book_out)
    ripple_state_helpers::account_send(view, amm_account, &book_out.issuer(), amount_out)
}

/// Remove a consumed offer — reference offerDelete parity.
/// Removes from owner directory, book directory, adjusts owner count, erases SLE.
fn remove_consumed_offer<V: ApplyView>(view: &mut V, offer_sle: &STLedgerEntry) {
    let offer_owner = offer_sle.get_account_id(sf("sfAccount"));

    // Remove from owner directory
    let owner_node = offer_sle.get_field_u64(sf("sfOwnerNode"));
    let owner_dir = protocol::owner_dir_keylet(basics::math::base_uint::Uint160::from_void(
        offer_owner.data(),
    ));
    let _ = crate::dir_remove(
        view as &mut dyn ApplyView,
        &owner_dir,
        owner_node,
        *offer_sle.key(),
        false,
    );

    // Remove from book directory
    let book_node = offer_sle.get_field_u64(sf("sfBookNode"));
    let book_dir_key = offer_sle.get_field_h256(sf("sfBookDirectory"));
    if !book_dir_key.is_zero() {
        let book_dir =
            protocol::Keylet::new(protocol::LedgerEntryType::DirectoryNode, book_dir_key);
        let _ = crate::dir_remove(
            view as &mut dyn ApplyView,
            &book_dir,
            book_node,
            *offer_sle.key(),
            true,
        );
    }

    // Adjust owner count
    let acct_keylet = protocol::account_keylet(basics::math::base_uint::Uint160::from_void(
        offer_owner.data(),
    ));
    if let Some(acct_sle) = view
        .peek(acct_keylet)
        .ok()
        .flatten()
        .or_else(|| view.read(acct_keylet).ok().flatten())
    {
        let _ = crate::adjust_owner_count(view as &mut dyn ApplyView, &acct_sle, -1);
    }

    // Erase the offer
    let _ = view.erase(Arc::new(offer_sle.clone()));
}

/// Get the best offer quality in a book (TakerGets/TakerPays = output/input).
/// Returns None if the book is empty.
pub fn get_book_best_quality<V: crate::ReadView>(view: &V, book: &Book) -> Option<f64> {
    let proto_book = protocol::Book {
        r#in: book.r#in,
        out: book.out,
        domain: book.domain,
    };
    let book_base = protocol::get_book_base(proto_book);
    let book_end = protocol::get_quality_next(book_base);
    let current_key = book_base;

    // Find the first directory page in the book range
    let next_page = view.succ(current_key, Some(book_end)).ok()??;
    let page_keylet = protocol::Keylet::new(protocol::LedgerEntryType::DirectoryNode, next_page);
    let dir = view.read(page_keylet).ok()??;

    // Get the first offer from this page
    let indexes = dir.get_field_v256(protocol::get_field_by_symbol("sfIndexes"));
    let offer_key = indexes.value().first()?;
    let offer_keylet = protocol::Keylet::new(protocol::LedgerEntryType::Offer, *offer_key);
    let offer = view.read(offer_keylet).ok()??;

    let sf = protocol::get_field_by_symbol;
    let taker_pays = offer.get_field_amount(sf("sfTakerPays"));
    let taker_gets = offer.get_field_amount(sf("sfTakerGets"));

    if taker_pays.signum() <= 0 {
        return None;
    }

    let tp = if taker_pays.native() {
        taker_pays.xrp().drops() as f64
    } else {
        taker_pays.mantissa() as f64 * 10f64.powi(taker_pays.exponent())
    };
    let tg = if taker_gets.native() {
        taker_gets.xrp().drops() as f64
    } else {
        taker_gets.mantissa() as f64 * 10f64.powi(taker_gets.exponent())
    };

    if tp <= 0.0 {
        return None;
    }
    Some(tg / tp) // TakerGets/TakerPays = output/input quality
}

fn get_book_offers<V: ApplyView>(view: &mut V, book: &Book, max: u32) -> Vec<STLedgerEntry> {
    let mut offers = Vec::new();

    let proto_book = protocol::Book {
        r#in: book.r#in,
        out: book.out,
        domain: book.domain,
    };
    let book_base = protocol::get_book_base(proto_book);
    let book_end = protocol::get_quality_next(book_base);

    let mut current_key = book_base;

    // Walk directory pages in quality order using succ
    while offers.len() < max as usize {
        // Find next directory page in the book range
        let next_page = match view.succ(current_key, Some(book_end)) {
            Ok(Some(key)) => key,
            _ => break,
        };

        // Read the directory page — use read fallback for NuDB-backed pages
        // not yet in the sandbox cache (fixes tecDIR_FULL for multi-page dirs).
        let page_keylet =
            protocol::Keylet::new(protocol::LedgerEntryType::DirectoryNode, next_page);
        let dir = view
            .peek(page_keylet)
            .ok()
            .flatten()
            .or_else(|| view.read(page_keylet).ok().flatten());
        let Some(dir) = dir else {
            // Advance past this page
            current_key = next_page;
            continue;
        };

        // Read offers from this page's sfIndexes
        if dir.is_field_present(sf("sfIndexes")) {
            let indexes = dir.get_field_v256(sf("sfIndexes"));
            for &offer_key in indexes.value() {
                if offers.len() >= max as usize {
                    break;
                }
                let offer_keylet =
                    protocol::Keylet::new(protocol::LedgerEntryType::Offer, offer_key);
                // Use read fallback for offers not yet in sandbox cache.
                let offer_sle = view
                    .peek(offer_keylet)
                    .ok()
                    .flatten()
                    .or_else(|| view.read(offer_keylet).ok().flatten());
                if let Some(offer_sle) = offer_sle {
                    offers.push(offer_sle.as_ref().clone());
                }
            }
        }

        // Move past this page for next iteration
        current_key = next_page;
    }

    offers
}

/// Get the funds available for an offer owner to deliver.
fn get_owner_funds<V: ApplyView>(view: &mut V, owner: &AccountID, asset: &Asset) -> STAmount {
    if *owner == asset.issuer() {
        // Owner is issuer — unlimited funds
        return STAmount::new_with_asset(sf("sfAmount"), *asset, u64::MAX / 2, 0, false);
    }
    if asset.native() {
        // XRP: balance minus reserve
        let acct_keylet =
            protocol::account_keylet(basics::base_uint::Uint160::from_void(owner.data()));
        if let Some(sle) = view
            .peek(acct_keylet)
            .ok()
            .flatten()
            .or_else(|| view.read(acct_keylet).ok().flatten())
        {
            let balance = sle.get_field_amount(sf("sfBalance")).xrp().drops();
            let owner_count = sle.get_field_u32(sf("sfOwnerCount"));
            let reserve = view.fees().account_reserve(owner_count as usize) as i64;
            let available = balance - reserve;
            if available <= 0 {
                return STAmount::default();
            }
            return STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(available));
        }
        return STAmount::default();
    }
    if let Asset::MPTIssue(issue) = *asset {
        let token_keylet = protocol::mptoken_keylet_from_mptid(
            issue.mpt_id(),
            basics::base_uint::Uint160::from_void(owner.data()),
        );
        let Some(token) = view
            .peek(token_keylet)
            .ok()
            .flatten()
            .or_else(|| view.read(token_keylet).ok().flatten())
        else {
            return STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::new(), issue);
        };
        if token.is_flag(protocol::lsfMPTLocked)
            || crate::mptoken_helpers::is_global_frozen_mpt(view, &issue).unwrap_or(true)
        {
            return STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::new(), issue);
        }
        return STAmount::from_mpt_amount(
            sf("sfAmount"),
            MPTAmount::from_value(token.get_field_u64(sf("sfMPTAmount")) as i64),
            issue,
        );
    }
    let Asset::Issue(issue) = *asset else {
        unreachable!("handled above");
    };
    // IOU: check freeze status first (reference FreezeHandling::ZeroIfFrozen)
    if ripple_state_helpers::is_frozen(view, owner, &issue) {
        return STAmount::default();
    }
    ripple_state_helpers::credit_balance(view, owner, &issue.account, issue.currency)
}

/// Result of offer consumption computation.
/// and offer amounts (what the offer owner receives/gives).
struct OfferConsumption {
    /// What the taker pays (includes input transfer rate)
    step_in: STAmount,
    /// What the taker receives (= ofrAmt.out, used for step output)
    step_out: STAmount,
    /// What the offer owner receives (= ofrAmt.in, no rate)
    offer_in: STAmount,
    /// What the offer owner gives (includes output transfer rate)
    owner_gives: STAmount,
    /// The actual offer output consumed (= ofrAmt.out, for updating offer SLE)
    offer_out: STAmount,
}

/// Compute how much of an offer to consume, applying transfer rates.
///   stpAmt.in = mulRatio(ofrAmt.in, ofrInRate, QUALITY_ONE, true)
///   ownerGives = mulRatio(ofrAmt.out, ofrOutRate, QUALITY_ONE, false)
///   If funds < ownerGives: recompute from available funds
///   If remaining_in < stpAmt.in: recompute from remaining input
fn compute_offer_consumption(
    remaining_in: &STAmount,
    taker_pays: &STAmount,
    taker_gets: &STAmount,
    owner_funds: &STAmount,
    transfer_rate_in: u32,
    transfer_rate_out: u32,
) -> OfferConsumption {
    let ofr_in = taker_pays.clone();
    let ofr_out = taker_gets.clone();

    // reference: stpAmt.in = mulRatio(ofrAmt.in, ofrInRate, QUALITY_ONE, true)
    let mut stp_in = mul_ratio_amount(&ofr_in, transfer_rate_in, QUALITY_ONE, true);
    let mut stp_out = ofr_out.clone();
    let mut owner_gives = mul_ratio_amount(&ofr_out, transfer_rate_out, QUALITY_ONE, false);
    let mut actual_ofr_in = ofr_in;
    let mut actual_ofr_out = ofr_out;

    // reference: if (funds < ownerGives) — limit by owner funding
    if *owner_funds < owner_gives {
        owner_gives = owner_funds.clone();
        stp_out = mul_ratio_amount(&owner_gives, QUALITY_ONE, transfer_rate_out, false);
        if taker_gets.signum() > 0 {
            actual_ofr_out = stp_out.clone();
            // quality.rate() = taker_pays / taker_gets
            // Use cross_type_mul_div to avoid intermediate overflow.
            actual_ofr_in = cross_type_scale(
                &actual_ofr_out,
                taker_pays,
                taker_gets,
                taker_pays.asset(),
                true,
            );
        }
        stp_in = mul_ratio_amount(&actual_ofr_in, transfer_rate_in, QUALITY_ONE, true);
    }

    // reference: limitStepIn if remaining_in < stpAmt.in
    if *remaining_in < stp_in {
        stp_in = remaining_in.clone();
        let in_lmt = mul_ratio_amount(&stp_in, QUALITY_ONE, transfer_rate_in, false);
        if taker_pays.signum() > 0 {
            actual_ofr_in = in_lmt;
            // quality.rate() = taker_pays / taker_gets
            // Use cross_type_mul_div to avoid intermediate overflow.
            actual_ofr_out = cross_type_scale(
                &actual_ofr_in,
                taker_gets,
                taker_pays,
                taker_gets.asset(),
                false,
            );
            static SCALE_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if SCALE_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                tracing::debug!(target: "ledger",                    "[cross_type_scale] in={:?} gets={:?} pays={:?} out={:?}",
                    actual_ofr_in.xrp().drops(),
                    taker_gets.mantissa(),
                    taker_pays.xrp().drops(),
                    actual_ofr_out.mantissa()
                );
            }
        }
        stp_out = actual_ofr_out.clone();
        owner_gives = mul_ratio_amount(&stp_out, transfer_rate_out, QUALITY_ONE, false);
    }

    OfferConsumption {
        step_in: stp_in,
        step_out: stp_out.clone(),
        offer_in: actual_ofr_in,
        owner_gives,
        offer_out: actual_ofr_out,
    }
}

/// Scale `value` by `numerator / denominator`, producing a result with `result_asset`.
/// Computes: result = value * numerator / denominator
/// Handles cross-type (XRP drops * IOU / XRP drops) correctly.
/// Matches reference Quality::ceilIn / ceilOut which use mulRound/divRound with full
/// integer precision (not floating point).
fn cross_type_scale(
    value: &STAmount,
    numerator: &STAmount,
    denominator: &STAmount,
    result_asset: protocol::Asset,
    round_up: bool,
) -> STAmount {
    let sf_amt = protocol::get_field_by_symbol("sfAmount");
    let zero = STAmount::new_with_asset(sf_amt, result_asset, 0, 0, false);

    if value.signum() == 0 || numerator.signum() == 0 {
        return zero;
    }

    let (d_m, _d_e) = raw_mantissa_exp(denominator);
    if d_m == 0 {
        return zero;
    }

    // Use i128 arithmetic to preserve full precision for the multiplication.
    // Convert each amount to a scaled integer representation:
    //   real_value = mantissa * 10^exponent
    // We compute: result = (v_m * 10^v_e) * (n_m * 10^n_e) / (d_m * 10^d_e)
    //           = (v_m * n_m / d_m) * 10^(v_e + n_e - d_e)
    // But to avoid overflow and preserve precision, we keep the product in i128.

    let (v_m, v_e) = raw_mantissa_exp(value);
    let (n_m, n_e) = raw_mantissa_exp(numerator);
    let (d_m, d_e) = raw_mantissa_exp(denominator);

    // product_mantissa = v_m * n_m (fits in u128 since both are at most ~10^16)
    let product: u128 = v_m as u128 * n_m as u128;
    let combined_exp: i32 = v_e + n_e - d_e;

    // result_mantissa = product / d_m (with rounding)
    let (quotient, remainder) = (product / d_m as u128, product % d_m as u128);
    let mut result_mantissa = if round_up && remainder > 0 {
        quotient + 1
    } else {
        quotient
    };

    if result_mantissa == 0 {
        return zero;
    }

    let negative = (value.negative() != numerator.negative()) != denominator.negative();

    if result_asset.native() {
        // XRP: result = result_mantissa * 10^combined_exp (in drops)
        let drops: i64 = if combined_exp >= 0 {
            (result_mantissa as i128 * 10i128.pow(combined_exp as u32)) as i64
        } else {
            let divisor = 10u128.pow((-combined_exp) as u32);
            let rem = result_mantissa % divisor;
            let q = result_mantissa / divisor;
            if round_up && rem > 0 {
                (q + 1) as i64
            } else {
                q as i64
            }
        };
        return STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(if negative {
            -drops
        } else {
            drops
        }));
    }

    // IOU: normalize result_mantissa * 10^combined_exp into [1e15, 1e16) * 10^exp
    let mut exponent = combined_exp;

    // Normalize: shift mantissa into [1e15, 1e16)
    while result_mantissa > 0 && result_mantissa < 1_000_000_000_000_000 {
        result_mantissa *= 10;
        exponent -= 1;
    }
    while result_mantissa >= 10_000_000_000_000_000 {
        let rem = result_mantissa % 10;
        result_mantissa /= 10;
        if round_up && rem > 0 {
            result_mantissa += 1;
        }
        exponent += 1;
    }

    // Clamp to valid IOU range
    let mantissa = (result_mantissa as u64)
        .max(1_000_000_000_000_000)
        .min(9_999_999_999_999_999);

    STAmount::new_with_asset(
        protocol::get_field_by_symbol("sfAmount"),
        result_asset,
        mantissa,
        exponent,
        negative,
    )
}

/// Get raw (mantissa, exponent) for an STAmount without normalization.
/// XRP: mantissa = drops, exponent = 0.
/// IOU: stored mantissa and exponent.
fn raw_mantissa_exp(amount: &STAmount) -> (u64, i32) {
    if amount.native() {
        (amount.xrp().drops().unsigned_abs(), 0)
    } else {
        (amount.mantissa(), amount.exponent())
    }
}

/// When round_up=true, rounds away from zero. When false, rounds toward zero.
fn mul_ratio_amount(
    amount: &STAmount,
    numerator: u32,
    denominator: u32,
    round_up: bool,
) -> STAmount {
    if numerator == denominator {
        return amount.clone();
    }
    if amount.native() {
        let drops = amount.xrp().drops();
        let result = if round_up {
            (drops as i128 * numerator as i128 + denominator as i128 - 1) / denominator as i128
        } else {
            (drops as i128 * numerator as i128) / denominator as i128
        };
        STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(result as i64))
    } else {
        match amount.asset() {
            Asset::MPTIssue(issue) => {
                let value = amount.mpt().value();
                let result = if round_up {
                    (value as i128 * numerator as i128 + denominator as i128 - 1)
                        / denominator as i128
                } else {
                    (value as i128 * numerator as i128) / denominator as i128
                };
                STAmount::from_mpt_amount(
                    protocol::get_field_by_symbol("sfAmount"),
                    MPTAmount::from_value(result as i64),
                    issue,
                )
            }
            Asset::Issue(issue) => {
                let iou = amount.iou();
                let adjusted =
                    crate::domain::mul_ratio::mul_ratio(iou, numerator, denominator, round_up);
                STAmount::from_iou_amount(
                    protocol::get_field_by_symbol("sfAmount"),
                    adjusted,
                    issue,
                )
            }
        }
    }
}

/// Execute the trade: transfer assets between path and offer owner.
fn execute_offer_trade<V: ApplyView>(
    view: &mut V,
    offer_owner: &AccountID,
    book_in: &Asset,
    book_out: &Asset,
    amount_in: &STAmount,
    amount_out: &STAmount,
) -> Ter {
    // Credit offer owner with amount_in (they receive what taker pays)
    let res = ripple_state_helpers::account_send(view, &book_in.issuer(), offer_owner, amount_in);
    if res != Ter::TES_SUCCESS {
        return res;
    }
    // Debit offer owner of amount_out (they give what taker gets)
    ripple_state_helpers::account_send(view, offer_owner, &book_out.issuer(), amount_out)
}

/// Result from estimate/execute that strand.rs expects
pub struct BookStepOutput {
    pub actual_amount_in: STAmount,
    pub actual_amount_out: STAmount,
    pub quality: protocol::Quality,
}

/// Estimate how much output a book step can produce for a given input.
pub fn estimate_explicit_book_step<V: crate::ReadView>(
    _view: &V,
    _source_asset: Asset,
    requested_out: &STAmount,
) -> Result<Option<BookStepOutput>, crate::ViewError> {
    let quality = protocol::Quality::from_amounts(&protocol::Amounts::new(
        requested_out.clone(),
        requested_out.clone(),
    ));
    Ok(Some(BookStepOutput {
        actual_amount_in: requested_out.clone(),
        actual_amount_out: requested_out.clone(),
        quality,
    }))
}

/// Execute a book step as part of a strand.
pub fn execute_explicit_book_step<V: ApplyView>(
    view: &mut V,
    _src_account: &AccountID,
    _dst_account: &AccountID,
    max_in: &STAmount,
    max_out: &STAmount,
    _domain: Option<()>,
) -> Result<Option<BookStepOutput>, crate::ViewError> {
    // Determine book from the amount issues
    let book = Book {
        r#in: max_in.asset(),
        out: max_out.asset(),
        domain: None,
    };
    let result = execute_book_step(view, &book, max_in, max_out, true, None, None);
    if result.ter == Ter::TES_SUCCESS && result.amount_out.signum() > 0 {
        let quality = protocol::Quality::from_amounts(&protocol::Amounts::new(
            result.amount_in.clone(),
            result.amount_out.clone(),
        ));
        Ok(Some(BookStepOutput {
            actual_amount_in: result.amount_in,
            actual_amount_out: result.amount_out,
            quality,
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mul_ratio_amount_xrp_round_up() {
        // 100 drops * 1002000000 / 1000000000 = 100.2 → rounds UP to 101
        let amount = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
        let result = mul_ratio_amount(&amount, 1_002_000_000, 1_000_000_000, true);
        // (100 * 1002000000 + 999999999) / 1000000000 = (100200000000 + 999999999) / 1000000000
        // = 101199999999 / 1000000000 = 101
        assert_eq!(result.xrp().drops(), 101);
    }

    #[test]
    fn test_mul_ratio_amount_xrp_round_down() {
        // 100 drops * 1002000000 / 1000000000 = 100.2 → rounds DOWN to 100
        let amount = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100));
        let result = mul_ratio_amount(&amount, 1_002_000_000, 1_000_000_000, false);
        // (100 * 1002000000) / 1000000000 = 100200000000 / 1000000000 = 100
        assert_eq!(result.xrp().drops(), 100);
    }

    #[test]
    fn test_mul_ratio_amount_identity() {
        // When numerator == denominator, return unchanged
        let amount = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(12345));
        let result = mul_ratio_amount(&amount, 1_000_000_000, 1_000_000_000, true);
        assert_eq!(result.xrp().drops(), 12345);
    }

    #[test]
    fn test_mul_ratio_amount_xrp_large() {
        // Large amount: 1 billion drops * 1.002 rate
        let amount = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1_000_000_000));
        let result_up = mul_ratio_amount(&amount, 1_002_000_000, 1_000_000_000, true);
        let result_down = mul_ratio_amount(&amount, 1_002_000_000, 1_000_000_000, false);
        // 1000000000 * 1002000000 / 1000000000 = 1002000000 (exact, no rounding needed)
        assert_eq!(result_up.xrp().drops(), 1_002_000_000);
        assert_eq!(result_down.xrp().drops(), 1_002_000_000);
    }
}
