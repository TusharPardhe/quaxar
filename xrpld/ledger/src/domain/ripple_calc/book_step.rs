//! Full reference the reference source parity — DEX order book crossing.
//!
//! BookStep iterates offers in an order book and consumes them to convert
//! between two assets. Handles transfer fees, funding limits, quality thresholds.

use std::sync::Arc;

use crate::ApplyView;
use crate::domain::flow_engine::SelfCrossCancellation;
use crate::domain::ripple_state_helpers;
use basics;
use basics::{
    base_uint::{Uint160, Uint256},
    number::{NumberParts as RuntimeNumber, NumberRoundModeGuard, RoundingMode},
};
use protocol::{
    AccountID, Amounts, Asset, MPTAmount, Quality, QualityFunction, QualityFunctionAmmTag,
    QualityFunctionClobLikeTag, STAmount, STLedgerEntry, Ter, get_field_by_symbol as sf,
};

/// `BookStep::kMaxOffersToConsume` in rippled.  Reaching the cap marks the
/// containing strand inactive; flow has a separate 1500-offer aggregate cap.
pub(crate) const MAX_OFFERS_TO_CONSUME: u32 = 1000;
const QUALITY_ONE: u32 = 1_000_000_000;

/// Book: represents an order book (pair of assets to trade)
#[derive(Debug, Clone)]
pub struct Book {
    pub r#in: Asset,
    pub out: Asset,
    pub domain: Option<Uint256>,
}

/// Quality uses XRPL's reversed stored-rate ordering; `>=` means an offer is
/// at least as favorable to the taker as the requested threshold.
pub(crate) fn quality_satisfies_threshold(
    offer_quality: Quality,
    threshold: Option<Quality>,
) -> bool {
    threshold.is_none_or(|minimum| offer_quality >= minimum)
}

fn rejects_step_quality(enforce: bool, offer_quality: Quality, threshold: Option<Quality>) -> bool {
    enforce && !quality_satisfies_threshold(offer_quality, threshold)
}

fn effective_strand_dst<'a>(
    strand_dst: Option<&'a AccountID>,
    taker: Option<&'a AccountID>,
) -> Option<&'a AccountID> {
    strand_dst.or(taker)
}

fn accepts_step_quality(first: &mut Option<Quality>, candidate: Quality) -> bool {
    match *first {
        Some(quality) => quality == candidate,
        None => {
            *first = Some(candidate);
            true
        }
    }
}

fn amm_target_quality(
    clob: Option<Quality>,
    threshold: Option<Quality>,
    fix_ammv1_1: bool,
    multi_path: bool,
) -> Option<Quality> {
    match (clob, threshold) {
        (Some(tip), Some(limit)) if fix_ammv1_1 && !multi_path && limit > tip => None,
        (tip, _) => tip,
    }
}

fn is_self_crossing_offer(
    remove_self_crossing: bool,
    strand_src: Option<&AccountID>,
    strand_dst: Option<&AccountID>,
    owner: &AccountID,
    offer_quality: Quality,
    quality_threshold: Option<Quality>,
) -> bool {
    remove_self_crossing
        && strand_src.is_some_and(|source| source == owner)
        && strand_dst.is_some_and(|destination| destination == owner)
        && quality_threshold
            .is_some_and(|threshold| quality_satisfies_threshold(offer_quality, Some(threshold)))
}

fn offer_owner_authorized<V: ApplyView>(view: &V, asset: &Asset, owner: &AccountID) -> bool {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.issuer() == *owner => true,
        Asset::Issue(issue) => {
            let issuer_id =
                Uint160::from_slice(issue.issuer().data()).expect("account width should match");
            let Ok(Some(issuer)) = view.read(protocol::account_keylet(issuer_id)) else {
                return false;
            };
            if !issuer.is_flag(protocol::lsfRequireAuth) {
                return true;
            }
            let Ok(Some(line)) = view.read(protocol::line(*owner, issue.issuer(), issue.currency))
            else {
                return false;
            };
            let flag = if *owner > issue.issuer() {
                protocol::lsfLowAuth
            } else {
                protocol::lsfHighAuth
            };
            line.is_flag(flag)
        }
        Asset::MPTIssue(issue) => crate::mptoken_helpers::require_auth_mpt(view, issue, owner)
            .is_ok_and(|ter| ter == Ter::TES_SUCCESS),
    }
}

/// Result of consuming offers from a book
#[derive(Debug, Clone)]
pub struct BookStepResult {
    pub amount_in: STAmount,
    pub amount_out: STAmount,
    pub offers_consumed: u32,
    pub ter: Ter,
}

/// Exact execution policy supplied by a flow `BookStep`.
#[derive(Debug, Clone)]
pub struct BookStepOptions<'a> {
    pub owner_pays_transfer_fee: bool,
    pub taker: Option<&'a AccountID>,
    pub quality_threshold: Option<Quality>,
    /// Only direct/default OfferCreate crossing may delete a self offer at
    /// the current executable tip. Other paths leave it untouched.
    pub remove_self_crossing: bool,
    /// Cancellation-only accumulator for eligible direct self-cross offers.
    /// It is intentionally separate from the value-flow sandbox.
    pub self_cross_cancellation: Option<SelfCrossCancellation>,
    pub amm_context: Option<crate::domain::flow_engine::AmmContext>,
    /// rippled's debt direction of the preceding strand step.
    pub previous_redeems: bool,
    /// Actual strand endpoint used by BookStep::rate. This is distinct from
    /// the transaction/taker account used for AMM auction fees and self-cross.
    pub strand_dst: Option<&'a AccountID>,
    pub strand_deliver: Option<Asset>,
    /// BookOfferCrossingStep checks its local threshold only on default path.
    pub enforce_quality_threshold: bool,
}

/// Execute a book step using the historical call signature. New flow code
/// should use `execute_book_step_with_options` so self-crossing policy is
/// explicit rather than inferred from a non-null taker.
pub fn execute_book_step<V: ApplyView>(
    view: &mut V,
    book: &Book,
    max_in: &STAmount,
    max_out: &STAmount,
    owner_pays_transfer_fee: bool,
    taker: Option<&AccountID>,
    quality_threshold: Option<Quality>,
) -> BookStepResult {
    execute_book_step_with_options(
        view,
        book,
        max_in,
        max_out,
        BookStepOptions {
            owner_pays_transfer_fee,
            taker,
            quality_threshold,
            remove_self_crossing: false,
            self_cross_cancellation: None,
            amm_context: None,
            previous_redeems: false,
            strand_dst: taker,
            strand_deliver: Some(max_out.asset()),
            enforce_quality_threshold: true,
        },
    )
}

/// Execute a book step: consume the best same-quality CLOB liquidity and an
/// AMM synthetic offer only when it satisfies the same threshold.
pub fn execute_book_step_with_options<V: ApplyView>(
    view: &mut V,
    book: &Book,
    max_in: &STAmount,
    max_out: &STAmount,
    options: BookStepOptions<'_>,
) -> BookStepResult {
    let owner_pays_transfer_fee = options.owner_pays_transfer_fee;
    let taker = options.taker;
    let quality_threshold = options.quality_threshold;
    let remove_self_crossing = options.remove_self_crossing;
    let self_cross_cancellation = options.self_cross_cancellation;
    let amm_context = options.amm_context.unwrap_or_else(|| {
        crate::domain::flow_engine::AmmContext::new(taker.copied().unwrap_or_default(), false)
    });
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

    let strand_dst = effective_strand_dst(options.strand_dst, taker);
    let strand_deliver = options.strand_deliver.unwrap_or_else(|| max_out.asset());
    let tr_in = if options.previous_redeems {
        transfer_rate_for_asset(view, book.r#in, strand_dst, strand_deliver)
    } else {
        QUALITY_ONE
    };
    let tr_out = if owner_pays_transfer_fee {
        transfer_rate_for_asset(view, book.out, strand_dst, strand_deliver)
    } else {
        QUALITY_ONE
    };

    // Iterate offers in the book directory
    // We use get_book_offers which reads from the offer directory.
    let raw_offers = get_book_offers(view, book, MAX_OFFERS_TO_CONSUME);

    // FlowOfferStream removes malformed, unauthorized-domain and unfunded
    // entries before exposing tip(). Do the same discovery cleanup before
    // asking AMMLiquidity to compete with that tip. Stop discovery at the
    // first valid offer: later entries must not be cleaned if AMM execution
    // ends this step before FlowOfferStream advances to them.
    let mut offers = Vec::with_capacity(raw_offers.len());
    let mut found_tip = false;
    for offer_sle in raw_offers {
        if found_tip {
            offers.push(offer_sle);
            continue;
        }
        let taker_pays = offer_sle.get_field_amount(sf("sfTakerPays"));
        let taker_gets = offer_sle.get_field_amount(sf("sfTakerGets"));
        if taker_pays.signum() <= 0 || taker_gets.signum() <= 0 {
            if let Some(removable) = &self_cross_cancellation {
                removable.record(*offer_sle.key());
            }
            remove_consumed_offer(view, &offer_sle);
            offers_consumed += 1;
            continue;
        }
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
                if let Some(removable) = &self_cross_cancellation {
                    removable.record(*offer_sle.key());
                }
                remove_consumed_offer(view, &offer_sle);
                offers_consumed += 1;
                continue;
            }
        }
        let offer_owner = offer_sle.get_account_id(sf("sfAccount"));
        if get_owner_funds(view, &offer_owner, &taker_gets).signum() <= 0 {
            if let Some(removable) = &self_cross_cancellation {
                removable.record(*offer_sle.key());
            }
            remove_consumed_offer(view, &offer_sle);
            offers_consumed += 1;
            continue;
        }
        found_tip = true;
        offers.push(offer_sle);
    }

    // A BookStep consumes one quality directory per call, as in rippled's
    // FlowOfferStream. This applies to payments as well as OfferCreate.
    let mut first_quality: Option<Quality> = None;
    let mut offer_attempted = false;

    // rippled tries AMM liquidity before the cleaned CLOB tip. The AMM offer
    // establishes the one-quality-per-step boundary just like a real offer.
    // Domain books never use AMM liquidity.
    let clob_tip = offers.first().map(|offer| {
        Quality::from_amounts(&Amounts::new(
            offer.get_field_amount(sf("sfTakerPays")),
            offer.get_field_amount(sf("sfTakerGets")),
        ))
    });
    let amm_generation_quality = amm_target_quality(
        clob_tip,
        quality_threshold,
        view.rules().enabled(&protocol::fix_ammv1_1()),
        amm_context.multi_path(),
    );
    let mut stop_before_clob = false;
    if book.domain.is_none()
        && remaining_in.signum() > 0
        && total_out < *max_out
        && let Some(amm_offer) = get_amm_offer(view, book, amm_generation_quality, &amm_context)
    {
        // `BookStep::execOffer` applies the same owner-authorization gate to
        // real offers and synthetic AMM offers.  An AMM trust line can exist
        // without the issuer having authorized it after `lsfRequireAuth` is
        // enabled; in that case the synthetic offer is skipped (there is no
        // ledger offer to erase) and the CLOB stream remains eligible.
        if !offer_owner_authorized(view, &book.r#in, &amm_offer.account) {
            first_quality = None;
        } else if rejects_step_quality(
            options.enforce_quality_threshold,
            amm_offer.quality(),
            quality_threshold,
        ) {
            first_quality = Some(amm_offer.quality());
            stop_before_clob = true;
        } else {
            first_quality = Some(amm_offer.quality());
            let remaining_out = max_out.clone() - total_out.clone();
            let raw_in_limit = mul_ratio_amount(&remaining_in, QUALITY_ONE, tr_in, false);
            if let Some((amm_pays, amm_gets)) = amm_offer.limit(&raw_in_limit, &remaining_out) {
                let step_in = mul_ratio_amount(&amm_pays, tr_in, QUALITY_ONE, true);
                if !amm_offer_invariant_holds(&amm_offer, &amm_pays, &amm_gets) {
                    tracing::warn!(
                        target: "ledger",
                        "[book_step] AMM pool product invariant failed"
                    );
                    if amm_invariant_failure_is_fatal(
                        false,
                        view.rules()
                            .enabled(&protocol::feature_id("fixAMMOverflowOffer")),
                    ) {
                        return BookStepResult {
                            amount_in: total_in,
                            amount_out: total_out,
                            offers_consumed,
                            ter: Ter::TEC_INVARIANT_FAILED,
                        };
                    }
                }
                let res = execute_amm_trade(
                    view,
                    &amm_offer.account,
                    &book.r#in,
                    &book.out,
                    &amm_pays,
                    &amm_gets,
                );
                if res == Ter::TES_SUCCESS {
                    amm_context.set_amm_used();
                    total_in += step_in.clone();
                    total_out += amm_gets;
                    remaining_in -= step_in;
                    offer_attempted = true;
                } else {
                    stop_before_clob = true;
                }
            } else {
                stop_before_clob = true;
            }
        }
    }

    if !stop_before_clob {
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

            // The first tip was funded during discovery; later tips are checked
            // only when FlowOfferStream-equivalent iteration reaches them.
            let owner_funds = get_owner_funds(view, &offer_owner, &taker_gets);
            if owner_funds.signum() <= 0 {
                remove_consumed_offer(view, &offer_sle);
                if !offer_attempted {
                    first_quality = None;
                }
                offers_consumed += 1;
                continue;
            }

            // The Book stores offer fields as TakerPays/TakerGets. In this
            // strand direction, `Quality::from_amounts` must receive that raw
            // pair (in=TakerPays, out=TakerGets) so its encoded comparison is
            // on the same scale as OfferCreate's `Quality{takerAmount.out,
            // sendMax}` threshold. Swapping them makes the reciprocal quality
            // and admits offers that are worse than the taker's limit.
            let offer_quality =
                Quality::from_amounts(&Amounts::new(taker_pays.clone(), taker_gets.clone()));

            // `forEachOffer` stops before invoking the derived callback when
            // the stream advances to a second quality after an offer attempt.
            if !accepts_step_quality(&mut first_quality, offer_quality) {
                break;
            }

            // BookOfferCrossingStep::limitSelfCrossQuality runs after stream
            // funding and same-quality selection but before authorization and
            // the general quality-threshold check. It requires both strand
            // endpoints to be the owner and the self offer to meet the limit.
            if is_self_crossing_offer(
                remove_self_crossing,
                taker,
                strand_dst,
                &offer_owner,
                offer_quality,
                quality_threshold,
            ) {
                // Do not remove through this value-flow sandbox. The caller
                // applies the recorded key with offer_helpers::offer_delete
                // even when this strand later proves dry.
                if let Some(cancellations) = &self_cross_cancellation {
                    cancellations.record(*offer_sle.key());
                }
                if !offer_attempted {
                    first_quality = None;
                }
                offers_consumed += 1;
                continue;
            }

            // Authorization follows the self-cross callback in rippled. An
            // invalid non-self tip is permanently cleaned and may expose the
            // next quality only when nothing has yet been attempted.
            if !offer_owner_authorized(view, &book.r#in, &offer_owner) {
                remove_consumed_offer(view, &offer_sle);
                if !offer_attempted {
                    first_quality = None;
                }
                offers_consumed += 1;
                continue;
            }
            if rejects_step_quality(
                options.enforce_quality_threshold,
                offer_quality,
                quality_threshold,
            ) {
                break;
            }

            offer_attempted = true;

            // Compute consumption amounts with transfer rates (reference forEachOffer parity).
            // A reverse BookStep must limit the final offer by the outstanding
            // requested output before it derives the required input. See
            // rippled BookStep.cpp `limitStepOut` and `revImp`.
            let remaining_out = max_out.clone() - total_out.clone();
            let consumption = compute_offer_consumption(
                &remaining_in,
                &remaining_out,
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
// The AMM uses the constant product formula: pool_in * pool_out = k. These
// swaps intentionally mirror rippled's AMMHelpers.h `swapAssetIn` and
// `swapAssetOut` Number arithmetic rather than converting ledger amounts to
// binary floating point. In particular, an output-limited trade must derive
// its input from the requested output, with every intermediate rounding step
// favoring the AMM.

fn number_to_amount(asset: Asset, amount: RuntimeNumber, mode: RoundingMode) -> Option<STAmount> {
    // `to_amount_from_number` installs this guard for native amounts, but IOU
    // conversion also needs it when reducing Number's runtime precision to
    // the 16-digit STAmount representation.
    let _rounding = NumberRoundModeGuard::new(mode);
    protocol::to_amount_from_number::<STAmount>(asset, amount, mode).ok()
}

/// Typed equivalent of rippled `swapAssetIn`.
///
/// `pool_out - (pool_in * pool_out) / (pool_in + asset_in * (1 - fee))`
fn amm_swap_asset_in(
    pool_in: &STAmount,
    pool_out: &STAmount,
    asset_in: &STAmount,
    trading_fee: u16,
    amm_rounding_enabled: bool,
) -> Option<STAmount> {
    if pool_in.signum() <= 0 || pool_out.signum() <= 0 || asset_in.signum() <= 0 {
        return None;
    }

    let pool_out_asset = pool_out.asset();
    let pool_in = crate::domain::amm_helpers::stamount_as_number(pool_in);
    let pool_out = crate::domain::amm_helpers::stamount_as_number(pool_out);
    let asset_in = crate::domain::amm_helpers::stamount_as_number(asset_in);

    let swap_out = if amm_rounding_enabled {
        // fixAMMv1_1: stage the calculation with the same directions as
        // rippled. The output is minimized, which preserves the pool product.
        let numerator = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
            pool_in * pool_out
        };
        let fee = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
            protocol::get_fee(trading_fee)
        };
        let denominator = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Downward);
            pool_in + asset_in * (RuntimeNumber::one(basics::number::get_mantissa_scale()) - fee)
        };
        if denominator <= RuntimeNumber::zero() {
            return None;
        }
        let ratio = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
            numerator / denominator
        };
        {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Downward);
            pool_out - ratio
        }
    } else {
        pool_out - (pool_in * pool_out) / (pool_in + asset_in * protocol::fee_mult(trading_fee))
    };

    if swap_out.signum() <= 0 {
        return None;
    }
    number_to_amount(pool_out_asset, swap_out, RoundingMode::Downward)
}

/// Typed equivalent of rippled `swapAssetOut`.
///
/// `((pool_in * pool_out) / (pool_out - asset_out) - pool_in) / (1 - fee)`
fn amm_swap_asset_out(
    pool_in_amount: &STAmount,
    pool_out_amount: &STAmount,
    asset_out_amount: &STAmount,
    trading_fee: u16,
    amm_rounding_enabled: bool,
) -> Option<STAmount> {
    if pool_in_amount.signum() <= 0
        || pool_out_amount.signum() <= 0
        || asset_out_amount.signum() <= 0
        || asset_out_amount >= pool_out_amount
    {
        return None;
    }

    let pool_in = crate::domain::amm_helpers::stamount_as_number(pool_in_amount);
    let pool_out = crate::domain::amm_helpers::stamount_as_number(pool_out_amount);
    let asset_out = crate::domain::amm_helpers::stamount_as_number(asset_out_amount);

    let swap_in = if amm_rounding_enabled {
        // fixAMMv1_1: stage the calculation with the inverse directions of
        // swapAssetIn. The required input is maximized, protecting the AMM.
        let numerator = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
            pool_in * pool_out
        };
        let denominator = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Downward);
            pool_out - asset_out
        };
        if denominator <= RuntimeNumber::zero() {
            return None;
        }
        let numerator2 = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
            numerator / denominator - pool_in
        };
        let fee = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
            protocol::get_fee(trading_fee)
        };
        let fee_mult = {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Downward);
            RuntimeNumber::one(basics::number::get_mantissa_scale()) - fee
        };
        if fee_mult <= RuntimeNumber::zero() {
            return None;
        }
        {
            let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
            numerator2 / fee_mult
        }
    } else {
        ((pool_in * pool_out) / (pool_out - asset_out) - pool_in) / protocol::fee_mult(trading_fee)
    };

    if swap_in.signum() <= 0 {
        return None;
    }
    number_to_amount(pool_in_amount.asset(), swap_in, RoundingMode::Upward)
}

/// rippled presents AMM liquidity to BookStep as a synthetic `AMMOffer`.
/// With no CLOB tip, `AMMLiquidity::getOffer` returns `maxOffer`: 99% of the
/// output pool and the input required to buy it. BookStep checks that
/// synthetic offer's pool spot quality before `limitStepIn`/`limitStepOut`
/// reduce it to the caller's requested amounts. A CLOB-targeted offer instead
/// carries the generated amounts' quality.
#[derive(Clone)]
struct SyntheticAmmOffer {
    account: AccountID,
    pool_in: STAmount,
    pool_out: STAmount,
    amount_in: STAmount,
    amount_out: STAmount,
    quality: Quality,
    trading_fee: u16,
    amm_rounding_enabled: bool,
    multi_path: bool,
    fix_reduced_offers_v2: bool,
}

impl SyntheticAmmOffer {
    fn quality(&self) -> Quality {
        self.quality
    }

    fn limit(&self, max_in: &STAmount, max_out: &STAmount) -> Option<(STAmount, STAmount)> {
        let (mut amount_in, mut amount_out) = (self.amount_in.clone(), self.amount_out.clone());

        if amount_out > *max_out {
            if self.multi_path {
                let limited = self.quality.ceil_out_strict(
                    &Amounts::new(amount_in, amount_out),
                    max_out,
                    true,
                );
                amount_in = limited.r#in;
                amount_out = limited.out;
            } else {
                amount_out = max_out.clone();
                amount_in = amm_swap_asset_out(
                    &self.pool_in,
                    &self.pool_out,
                    &amount_out,
                    self.trading_fee,
                    self.amm_rounding_enabled,
                )?;
            }
        }
        if amount_in > *max_in {
            if self.multi_path {
                let amounts = Amounts::new(amount_in, amount_out);
                let limited = if self.fix_reduced_offers_v2 {
                    self.quality.ceil_in_strict(&amounts, max_in, false)
                } else {
                    self.quality.ceil_in(&amounts, max_in)
                };
                amount_in = limited.r#in;
                amount_out = limited.out;
            } else {
                amount_in = max_in.clone();
                amount_out = amm_swap_asset_in(
                    &self.pool_in,
                    &self.pool_out,
                    &amount_in,
                    self.trading_fee,
                    self.amm_rounding_enabled,
                )?;
            }
        }

        (amount_in.signum() > 0 && amount_out.signum() > 0).then_some((amount_in, amount_out))
    }
}

fn amm_offer_invariant_holds(
    offer: &SyntheticAmmOffer,
    consumed_in: &STAmount,
    consumed_out: &STAmount,
) -> bool {
    if *consumed_in > offer.amount_in || *consumed_out > offer.amount_out {
        return false;
    }
    let old = crate::domain::amm_helpers::stamount_as_number(&offer.pool_in)
        * crate::domain::amm_helpers::stamount_as_number(&offer.pool_out);
    let new = (crate::domain::amm_helpers::stamount_as_number(&offer.pool_in)
        + crate::domain::amm_helpers::stamount_as_number(consumed_in))
        * (crate::domain::amm_helpers::stamount_as_number(&offer.pool_out)
            - crate::domain::amm_helpers::stamount_as_number(consumed_out));
    new >= old
        || crate::domain::amm_helpers::within_relative_distance_amount(
            new,
            old,
            RuntimeNumber::from_i64_and_exponent(1, -7),
        )
}

fn amm_invariant_failure_is_fatal(invariant_holds: bool, fix_enabled: bool) -> bool {
    !invariant_holds && fix_enabled
}

fn amm_max_output(pool_out: &STAmount) -> Option<STAmount> {
    let max_out = {
        let _rounding = NumberRoundModeGuard::new(RoundingMode::Downward);
        crate::domain::amm_helpers::stamount_as_number(pool_out)
            * RuntimeNumber::from_i64_and_exponent(99, -2)
    };
    number_to_amount(pool_out.asset(), max_out, RoundingMode::Downward)
        .filter(|amount| amount.signum() > 0 && amount < pool_out)
}

fn amm_max_offer_amounts(
    pool_in: &STAmount,
    pool_out: &STAmount,
    trading_fee: u16,
    amm_rounding_enabled: bool,
    fix_overflow_offer: bool,
) -> Option<(STAmount, STAmount)> {
    if fix_overflow_offer {
        let out = amm_max_output(pool_out)?;
        let input = amm_swap_asset_out(pool_in, pool_out, &out, trading_fee, amm_rounding_enabled)?;
        Some((input, out))
    } else {
        let input = protocol::to_max_amount::<STAmount>(pool_in.asset());
        let out = amm_swap_asset_in(pool_in, pool_out, &input, trading_fee, amm_rounding_enabled)?;
        Some((input, out))
    }
}

fn amm_trading_fee(
    parent_close_time: u64,
    amm_sle: &STLedgerEntry,
    account: Option<&AccountID>,
) -> u16 {
    if let Some(account) = account
        && amm_sle.is_field_present(sf("sfAuctionSlot"))
    {
        let slot = amm_sle.get_field_object(sf("sfAuctionSlot"));
        let expiration = u64::from(slot.get_field_u32(sf("sfExpiration")));
        if parent_close_time < expiration {
            let owns_slot = slot.get_account_id(sf("sfAccount")) == *account;
            let is_authorized = slot.is_field_present(sf("sfAuthAccounts"))
                && slot
                    .get_field_array(sf("sfAuthAccounts"))
                    .iter()
                    .any(|entry| entry.get_account_id(sf("sfAccount")) == *account);
            if owns_slot || is_authorized {
                return slot.get_field_u16(sf("sfDiscountedFee"));
            }
        }
    }
    amm_sle.get_field_u16(sf("sfTradingFee"))
}

fn reduce_amm_offer(amount: RuntimeNumber) -> RuntimeNumber {
    let _rounding = NumberRoundModeGuard::new(RoundingMode::TowardsZero);
    amount * RuntimeNumber::from_i64_and_exponent(9_999, -4)
}

fn amm_offer_starting_with_gets(
    pool_in: &STAmount,
    pool_out: &STAmount,
    target: Quality,
    trading_fee: u16,
    amm_rounding_enabled: bool,
) -> Option<(STAmount, STAmount)> {
    let q = crate::domain::amm_helpers::stamount_as_number(&target.rate());
    if q == RuntimeNumber::zero() {
        return None;
    }
    let _rounding = NumberRoundModeGuard::new(RoundingMode::ToNearest);
    let one = RuntimeNumber::from_i64_and_exponent(1, 0);
    let two = RuntimeNumber::from_i64_and_exponent(2, 0);
    let pool_in_n = crate::domain::amm_helpers::stamount_as_number(pool_in);
    let pool_out_n = crate::domain::amm_helpers::stamount_as_number(pool_out);
    let fee_mult = protocol::fee_mult(trading_fee);
    let b = pool_in_n * (one - one / fee_mult) / q - two * pool_out_n;
    let c = pool_out_n * pool_out_n - (pool_in_n * pool_out_n) / q;
    let mut proposed = crate::domain::amm_helpers::solve_quadratic_eq_smallest(one, b, c)?;
    if proposed <= RuntimeNumber::zero() {
        return None;
    }
    let constraint = pool_out_n - pool_in_n / (q * fee_mult);
    if constraint <= RuntimeNumber::zero() {
        return None;
    }
    if constraint < proposed {
        proposed = constraint;
    }

    let amounts = |out_number| {
        let out = number_to_amount(pool_out.asset(), out_number, RoundingMode::Downward)?;
        let input = amm_swap_asset_out(pool_in, pool_out, &out, trading_fee, amm_rounding_enabled)?;
        Some((input, out))
    };
    let mut result = amounts(proposed)?;
    if Quality::from_amounts(&Amounts::new(result.0.clone(), result.1.clone())) < target {
        result = amounts(reduce_amm_offer(
            crate::domain::amm_helpers::stamount_as_number(&result.1),
        ))?;
    }
    Some(result)
}

fn amm_offer_starting_with_pays(
    pool_in: &STAmount,
    pool_out: &STAmount,
    target: Quality,
    trading_fee: u16,
    amm_rounding_enabled: bool,
) -> Option<(STAmount, STAmount)> {
    let q = crate::domain::amm_helpers::stamount_as_number(&target.rate());
    if q == RuntimeNumber::zero() {
        return None;
    }
    let _rounding = NumberRoundModeGuard::new(RoundingMode::ToNearest);
    let one = RuntimeNumber::from_i64_and_exponent(1, 0);
    let pool_in_n = crate::domain::amm_helpers::stamount_as_number(pool_in);
    let pool_out_n = crate::domain::amm_helpers::stamount_as_number(pool_out);
    let fee_mult = protocol::fee_mult(trading_fee);
    let b = pool_in_n * (one + fee_mult);
    let c = pool_in_n * pool_in_n - pool_in_n * pool_out_n * q;
    let mut proposed = crate::domain::amm_helpers::solve_quadratic_eq_smallest(fee_mult, b, c)?;
    if proposed <= RuntimeNumber::zero() {
        return None;
    }
    let constraint = pool_out_n * q - pool_in_n / fee_mult;
    if constraint <= RuntimeNumber::zero() {
        return None;
    }
    if constraint < proposed {
        proposed = constraint;
    }

    let amounts = |in_number| {
        let input = number_to_amount(pool_in.asset(), in_number, RoundingMode::Downward)?;
        let out = amm_swap_asset_in(pool_in, pool_out, &input, trading_fee, amm_rounding_enabled)?;
        Some((input, out))
    };
    let mut result = amounts(proposed)?;
    if Quality::from_amounts(&Amounts::new(result.0.clone(), result.1.clone())) < target {
        result = amounts(reduce_amm_offer(
            crate::domain::amm_helpers::stamount_as_number(&result.0),
        ))?;
    }
    Some(result)
}

fn amm_offer_for_clob_quality(
    pool_in: &STAmount,
    pool_out: &STAmount,
    target: Quality,
    trading_fee: u16,
    amm_rounding_enabled: bool,
) -> Option<(STAmount, STAmount)> {
    // Legacy changeSpotPriceQuality calculates takerPays first and permits a
    // 1e-7 quality-distance rounding tolerance.
    if !amm_rounding_enabled {
        let q = crate::domain::amm_helpers::stamount_as_number(&target.rate());
        if q == RuntimeNumber::zero() {
            return None;
        }
        let _rounding = NumberRoundModeGuard::new(RoundingMode::ToNearest);
        let one = RuntimeNumber::from_i64_and_exponent(1, 0);
        let pool_in_n = crate::domain::amm_helpers::stamount_as_number(pool_in);
        let pool_out_n = crate::domain::amm_helpers::stamount_as_number(pool_out);
        let fee_mult = protocol::fee_mult(trading_fee);
        let b = pool_in_n * (one + fee_mult);
        let c = pool_in_n * pool_in_n - pool_in_n * pool_out_n * q;
        let mut proposed = crate::domain::amm_helpers::solve_quadratic_eq_smallest(fee_mult, b, c)?;
        let constraint = pool_out_n * q - pool_in_n / fee_mult;
        if proposed > constraint {
            proposed = constraint;
        }
        if proposed <= RuntimeNumber::zero() {
            return None;
        }
        let input = number_to_amount(pool_in.asset(), proposed, RoundingMode::Upward)?;
        let out = amm_swap_asset_in(pool_in, pool_out, &input, trading_fee, false)?;
        let quality = Quality::from_amounts(&Amounts::new(input.clone(), out.clone()));
        return (quality >= target
            || crate::domain::amm_helpers::within_relative_distance_quality(
                quality,
                target,
                RuntimeNumber::from_i64_and_exponent(1, -7),
            ))
        .then_some((input, out));
    }

    let result = if pool_out.asset().integral()
        && (!pool_in.asset().integral()
            || crate::domain::amm_helpers::stamount_as_number(&target.rate())
                >= RuntimeNumber::from_i64_and_exponent(1, 0))
    {
        amm_offer_starting_with_gets(pool_in, pool_out, target, trading_fee, amm_rounding_enabled)
    } else {
        amm_offer_starting_with_pays(pool_in, pool_out, target, trading_fee, amm_rounding_enabled)
    }?;
    (Quality::from_amounts(&Amounts::new(result.0.clone(), result.1.clone())) >= target)
        .then_some(result)
}

const AMM_FIBONACCI: [u32; 30] = [
    1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1_597, 2_584, 4_181, 6_765, 10_946,
    17_711, 28_657, 46_368, 75_025, 121_393, 196_418, 317_811, 514_229, 832_040, 1_346_269,
];

fn generate_fibonacci_amm_offer(
    initial_in: &STAmount,
    initial_out: &STAmount,
    current_in: &STAmount,
    current_out: &STAmount,
    trading_fee: u16,
    amm_rounding_enabled: bool,
    iteration: u16,
) -> Option<(STAmount, STAmount)> {
    if iteration as usize >= AMM_FIBONACCI.len() {
        return None;
    }
    let initial_input_number = {
        let _rounding = NumberRoundModeGuard::new(RoundingMode::Upward);
        crate::domain::amm_helpers::stamount_as_number(initial_in)
            * RuntimeNumber::from_i64_and_exponent(5, 0)
            / RuntimeNumber::from_i64_and_exponent(20_000, 0)
    };
    let initial_input = number_to_amount(
        initial_in.asset(),
        initial_input_number,
        RoundingMode::Upward,
    )?;
    let initial_output = amm_swap_asset_in(
        initial_in,
        initial_out,
        &initial_input,
        trading_fee,
        amm_rounding_enabled,
    )?;
    if iteration == 0 {
        return Some((initial_input, initial_output));
    }

    let output_number = {
        let _rounding = NumberRoundModeGuard::new(RoundingMode::Downward);
        crate::domain::amm_helpers::stamount_as_number(&initial_output)
            * RuntimeNumber::from_i64_and_exponent(
                i64::from(AMM_FIBONACCI[usize::from(iteration - 1)]),
                0,
            )
    };
    let output = number_to_amount(current_out.asset(), output_number, RoundingMode::Downward)?;
    if output >= *current_out {
        return None;
    }
    let input = amm_swap_asset_out(
        current_in,
        current_out,
        &output,
        trading_fee,
        amm_rounding_enabled,
    )?;
    Some((input, output))
}

/// Returns rippled's unlimited `AMMLiquidity::maxOffer`, before BookStep
/// applies the transaction's input and output limits.
fn get_amm_offer<V: ApplyView>(
    view: &mut V,
    book: &Book,
    clob_quality: Option<Quality>,
    amm_context: &crate::domain::flow_engine::AmmContext,
) -> Option<SyntheticAmmOffer> {
    if amm_context.max_iterations_reached() {
        return None;
    }
    // Find AMM SLE for this book
    let amm_keylet = protocol::amm(book.r#in, book.out);
    let amm_sle = view.read(amm_keylet).ok()??;

    // An empty AMM object is not a source of synthetic liquidity.
    if amm_sle.get_field_amount(sf("sfLPTokenBalance")).signum() <= 0 {
        return None;
    }

    // Get AMM account
    let amm_account = amm_sle.get_account_id(sf("sfAccount"));

    // Get trading fee
    let fee_account = amm_context.account();
    let trading_fee = amm_trading_fee(
        u64::from(view.header().parent_close_time),
        &amm_sle,
        Some(&fee_account),
    );

    // `ammAccountHolds`: frozen IOU/MPT assets have no AMM liquidity and IOU
    // balances must carry the book issuer, irrespective of trust-line storage
    // orientation.
    let pool_in_amount = amm_account_holds(view, &amm_account, book.r#in)?;
    let pool_out_amount = amm_account_holds(view, &amm_account, book.out)?;

    if pool_in_amount.signum() <= 0 || pool_out_amount.signum() <= 0 {
        return None;
    }

    let amm_rounding_enabled = view.rules().enabled(&protocol::fix_ammv1_1());
    let spot_quality = Quality::from_amounts(&Amounts::new(
        pool_in_amount.clone(),
        pool_out_amount.clone(),
    ));
    if let Some(clob) = clob_quality
        && (spot_quality <= clob
            || crate::domain::amm_helpers::within_relative_distance_quality(
                spot_quality,
                clob,
                RuntimeNumber::from_i64_and_exponent(1, -7),
            ))
    {
        return None;
    }

    let max_offer = || {
        amm_max_offer_amounts(
            &pool_in_amount,
            &pool_out_amount,
            trading_fee,
            amm_rounding_enabled,
            view.rules()
                .enabled(&protocol::feature_id("fixAMMOverflowOffer")),
        )
    };
    if amm_context.multi_path() {
        let (initial_in, initial_out) = amm_context.initial_balances(
            (book.r#in, book.out),
            &(pool_in_amount.clone(), pool_out_amount.clone()),
        );
        let (offered_in, offered_out) = generate_fibonacci_amm_offer(
            &initial_in,
            &initial_out,
            &pool_in_amount,
            &pool_out_amount,
            trading_fee,
            amm_rounding_enabled,
            amm_context.iterations(),
        )?;
        let quality = Quality::from_amounts(&Amounts::new(offered_in.clone(), offered_out.clone()));
        if clob_quality.is_some_and(|clob| quality < clob) {
            return None;
        }
        return Some(SyntheticAmmOffer {
            account: amm_account,
            pool_in: pool_in_amount,
            pool_out: pool_out_amount,
            amount_in: offered_in,
            amount_out: offered_out,
            quality,
            trading_fee,
            amm_rounding_enabled,
            multi_path: true,
            fix_reduced_offers_v2: view
                .rules()
                .enabled(&protocol::feature_id("fixReducedOffersV2")),
        });
    }
    let (offered_in, offered_out, offer_quality) = match clob_quality {
        None => {
            let (input, out) = max_offer()?;
            (input, out, spot_quality)
        }
        Some(clob) => amm_offer_for_clob_quality(
            &pool_in_amount,
            &pool_out_amount,
            clob,
            trading_fee,
            amm_rounding_enabled,
        )
        .map(|(input, out)| {
            let quality = Quality::from_amounts(&Amounts::new(input.clone(), out.clone()));
            (input, out, quality)
        })
        .or_else(|| {
            view.rules()
                .enabled(&protocol::feature_id("fixAMMv1_2"))
                .then(|| max_offer())
                .flatten()
                .filter(|amounts| {
                    Quality::from_amounts(&Amounts::new(amounts.0.clone(), amounts.1.clone()))
                        > clob
                })
                .map(|(input, out)| (input, out, spot_quality))
        })?,
    };

    Some(SyntheticAmmOffer {
        account: amm_account,
        pool_in: pool_in_amount,
        pool_out: pool_out_amount,
        amount_in: offered_in,
        amount_out: offered_out,
        quality: offer_quality,
        trading_fee,
        amm_rounding_enabled,
        multi_path: false,
        fix_reduced_offers_v2: view
            .rules()
            .enabled(&protocol::feature_id("fixReducedOffersV2")),
    })
}

/// Non-mutating liquidity ordering key used by flow's ActiveStrands.  This is
/// the BookStep portion of rippled's `qualityUpperBound`: the best currently
/// executable CLOB/AMM tip, before amount limiting.
pub(crate) fn book_quality_upper_bound<V: ApplyView>(
    view: &mut V,
    book: &Book,
    quality_threshold: Option<Quality>,
    amm_context: &crate::domain::flow_engine::AmmContext,
    owner_pays_transfer_fee: bool,
    previous_redeems: bool,
    strand_dst: &AccountID,
    strand_deliver: Asset,
) -> Option<Quality> {
    let clob = get_book_offers(view, book, 1).first().map(|offer| {
        Quality::from_amounts(&Amounts::new(
            offer.get_field_amount(sf("sfTakerPays")),
            offer.get_field_amount(sf("sfTakerGets")),
        ))
    });
    let generation_quality = amm_target_quality(
        clob,
        quality_threshold,
        view.rules().enabled(&protocol::fix_ammv1_1()),
        amm_context.multi_path(),
    );
    let amm = book
        .domain
        .is_none()
        .then(|| get_amm_offer(view, book, generation_quality, amm_context))
        .flatten();
    let (quality, is_amm, amm_multi_path) = match (clob, amm) {
        (Some(lhs), Some(rhs)) if rhs.quality() > lhs => (rhs.quality(), true, rhs.multi_path),
        (Some(lhs), _) => (lhs, false, false),
        (None, Some(rhs)) => (rhs.quality(), true, rhs.multi_path),
        (None, None) => return None,
    };

    Some(adjust_quality_with_fees(
        view,
        book,
        quality,
        is_amm,
        amm_multi_path,
        owner_pays_transfer_fee,
        previous_redeems,
        strand_dst,
        strand_deliver,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn book_quality_function<V: ApplyView>(
    view: &mut V,
    book: &Book,
    quality_threshold: Option<Quality>,
    amm_context: &crate::domain::flow_engine::AmmContext,
    owner_pays_transfer_fee: bool,
    previous_redeems: bool,
    strand_dst: &AccountID,
    strand_deliver: Asset,
) -> Option<QualityFunction> {
    let clob = get_book_offers(view, book, 1).first().map(|offer| {
        Quality::from_amounts(&Amounts::new(
            offer.get_field_amount(sf("sfTakerPays")),
            offer.get_field_amount(sf("sfTakerGets")),
        ))
    });
    let target = amm_target_quality(
        clob,
        quality_threshold,
        view.rules().enabled(&protocol::fix_ammv1_1()),
        amm_context.multi_path(),
    );
    let amm = book
        .domain
        .is_none()
        .then(|| get_amm_offer(view, book, target, amm_context))
        .flatten();
    let choose_amm = match (clob, amm.as_ref()) {
        (Some(lhs), Some(rhs)) => rhs.quality() > lhs,
        (None, Some(_)) => true,
        _ => false,
    };
    if choose_amm {
        let offer = amm?;
        if offer.multi_path {
            return Some(QualityFunction::from_quality(
                adjust_quality_with_fees(
                    view,
                    book,
                    offer.quality(),
                    true,
                    true,
                    owner_pays_transfer_fee,
                    previous_redeems,
                    strand_dst,
                    strand_deliver,
                ),
                QualityFunctionClobLikeTag,
            ));
        }
        let compose_input_rate = should_compose_single_path_input_rate(
            previous_redeems,
            owner_pays_transfer_fee,
            view.rules().enabled(&protocol::fix_ammv1_1()),
        );
        let mut qf = if compose_input_rate {
            let tr = transfer_rate_for_asset(view, book.r#in, Some(strand_dst), strand_deliver);
            let input = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(i64::from(tr)));
            let output =
                STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(i64::from(QUALITY_ONE)));
            QualityFunction::from_quality(
                Quality::from_amounts(&Amounts::new(input, output)),
                QualityFunctionClobLikeTag,
            )
        } else {
            let one = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1));
            QualityFunction::from_quality(
                Quality::from_amounts(&Amounts::new(one.clone(), one)),
                QualityFunctionClobLikeTag,
            )
        };
        qf.combine(&QualityFunction::from_amm(
            &Amounts::new(offer.pool_in, offer.pool_out),
            offer.trading_fee,
            QualityFunctionAmmTag,
        ));
        Some(qf)
    } else {
        clob.map(|quality| {
            QualityFunction::from_quality(
                adjust_quality_with_fees(
                    view,
                    book,
                    quality,
                    false,
                    false,
                    owner_pays_transfer_fee,
                    previous_redeems,
                    strand_dst,
                    strand_deliver,
                ),
                QualityFunctionClobLikeTag,
            )
        })
    }
}

fn should_compose_single_path_input_rate(
    previous_redeems: bool,
    offer_crossing: bool,
    fix_ammv1_1: bool,
) -> bool {
    previous_redeems && (!offer_crossing || fix_ammv1_1)
}

#[allow(clippy::too_many_arguments)]
fn adjust_quality_with_fees<V: ApplyView>(
    view: &mut V,
    book: &Book,
    offer_quality: Quality,
    is_amm: bool,
    amm_multi_path: bool,
    owner_pays_transfer_fee: bool,
    previous_redeems: bool,
    strand_dst: &AccountID,
    strand_deliver: Asset,
) -> Quality {
    // BookOfferCrossingStep deliberately returns the raw upper bound for CLOB
    // and multipath AMM liquidity. For a single-path AMM, fixAMMv1_1 makes the
    // incoming transfer rate part of the nonlinear quality upper bound.
    if owner_pays_transfer_fee
        && (!view.rules().enabled(&protocol::fix_ammv1_1()) || !is_amm || amm_multi_path)
    {
        return offer_quality;
    }

    let tr_in = if previous_redeems {
        transfer_rate_for_asset(view, book.r#in, Some(strand_dst), strand_deliver)
    } else {
        QUALITY_ONE
    };
    // AMM synthetic offers waive the output transfer fee. Payments charge it
    // only when their BookStep policy says the offer owner pays it.
    let tr_out = if !is_amm && owner_pays_transfer_fee {
        transfer_rate_for_asset(view, book.out, Some(strand_dst), strand_deliver)
    } else {
        QUALITY_ONE
    };
    compose_transfer_quality(offer_quality, tr_in, tr_out)
}

fn compose_transfer_quality(offer_quality: Quality, tr_in: u32, tr_out: u32) -> Quality {
    let input = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(i64::from(tr_in)));
    let output = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(i64::from(tr_out)));
    protocol::composed_quality(
        Quality::from_amounts(&Amounts::new(input, output)),
        offer_quality,
    )
}

fn amm_account_holds<V: ApplyView>(
    view: &mut V,
    amm_account: &AccountID,
    asset: Asset,
) -> Option<STAmount> {
    match asset {
        Asset::Issue(issue) if issue.native() => {
            let keylet =
                protocol::account_keylet(basics::base_uint::Uint160::from_void(amm_account.data()));
            Some(view.read(keylet).ok()??.get_field_amount(sf("sfBalance")))
        }
        Asset::Issue(issue) => {
            if ripple_state_helpers::is_frozen(view, amm_account, &issue) {
                return Some(STAmount::new_with_asset(sf("sfAmount"), asset, 0, 0, false));
            }
            let balance = ripple_state_helpers::account_holds(
                view,
                amm_account,
                &issue.account,
                issue.currency,
            );
            Some(normalize_amount_to_asset(&balance, asset))
        }
        Asset::MPTIssue(issue) => {
            if crate::mptoken_helpers::is_frozen_mpt(view, amm_account, &issue).unwrap_or(true) {
                return Some(STAmount::new_with_asset(sf("sfAmount"), asset, 0, 0, false));
            }
            let token = view
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    basics::base_uint::Uint160::from_void(amm_account.data()),
                ))
                .ok()??;
            Some(STAmount::from_mpt_amount(
                sf("sfAmount"),
                MPTAmount::from_value(token.get_field_u64(sf("sfMPTAmount")) as i64),
                issue,
            ))
        }
    }
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
    // rippled AMMOffer::send constructs each transferred STAmount with the
    // exact Book asset (`toSTAmount(ofrAmt.in, book_.in)` / book_.out). Pool
    // math may carry the issuer identity from a stored RippleState balance;
    // forwarding that temporary identity to account_send can miss the
    // canonical AMM pool line and create a new receiver-owned trust line.
    let amount_in = normalize_amount_to_asset(amount_in, *book_in);
    let amount_out = normalize_amount_to_asset(amount_out, *book_out);

    // Taker pays amount_in to AMM (AMM receives book_in)
    let res = ripple_state_helpers::account_send(view, &book_in.issuer(), amm_account, &amount_in);
    if res != Ter::TES_SUCCESS {
        return res;
    }
    // AMM pays amount_out to taker (AMM sends book_out)
    ripple_state_helpers::account_send(view, amm_account, &book_out.issuer(), &amount_out)
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

/// Transaction-context-free identity fields for malformed-book diagnostics.
fn book_asset_identity(asset: Asset) -> (String, String) {
    match asset {
        Asset::Issue(issue) => (
            protocol::currency_to_string(issue.currency),
            protocol::to_base58(issue.account),
        ),
        Asset::MPTIssue(issue) => (
            format!("MPT:{:?}", issue.mpt_id()),
            protocol::to_base58(issue.issuer()),
        ),
    }
}

fn get_book_offers<V: ApplyView>(view: &mut V, book: &Book, max: u32) -> Vec<STLedgerEntry> {
    let mut offers = Vec::new();

    // Offers are stored under their executable TakerPays -> TakerGets book,
    // the same orientation a BookStep receives as `book.in -> book.out`.
    let proto_book = protocol::Book {
        r#in: book.r#in,
        out: book.out,
        domain: book.domain,
    };
    let consistent = protocol::is_consistent_book(proto_book);
    if !consistent {
        // Diagnostics only: retain the protocol-owned assertion and do not
        // normalize, reject, or otherwise change offer selection/state here.
        let (input_currency, input_issuer) = book_asset_identity(book.r#in);
        let (output_currency, output_issuer) = book_asset_identity(book.out);
        tracing::warn!(
            target: "ledger",
            book_input_currency = %input_currency,
            book_input_issuer = %input_issuer,
            book_output_currency = %output_currency,
            book_output_issuer = %output_issuer,
            consistent,
            "[book_step] inconsistent book passed to get_book_offers"
        );
    }
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
fn get_owner_funds<V: ApplyView>(
    view: &mut V,
    owner: &AccountID,
    default_amount: &STAmount,
) -> STAmount {
    let asset = default_amount.asset();
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
    if let Asset::MPTIssue(issue) = asset {
        if *owner == issue.issuer() {
            let issuance = view
                .read(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
                .ok()
                .flatten();
            let available = issuance
                .as_deref()
                .map(crate::mptoken_helpers::available_mpt_amount)
                .unwrap_or(0);
            return STAmount::from_mpt_amount(
                sf("sfAmount"),
                MPTAmount::from_value(available),
                issue,
            );
        }
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
        if crate::mptoken_helpers::is_frozen_mpt(view, owner, &issue).unwrap_or(true)
            || crate::mptoken_helpers::require_auth_mpt_with_type(
                view,
                &issue,
                owner,
                crate::mptoken_helpers::MPTAuthType::Strong,
            )
            .unwrap_or(Ter::TEC_NO_AUTH)
                != Ter::TES_SUCCESS
        {
            return STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::new(), issue);
        }
        return STAmount::from_mpt_amount(
            sf("sfAmount"),
            MPTAmount::from_value(token.get_field_u64(sf("sfMPTAmount")) as i64),
            issue,
        );
    }
    let Asset::Issue(issue) = asset else {
        unreachable!("handled above");
    };
    if *owner == issue.issuer() {
        // IOU issuers self-fund precisely the offer's output amount. Using a
        // synthetic fixed-exponent maximum can underfund valid large offers.
        return default_amount.clone();
    }
    // IOU: check freeze status first (reference FreezeHandling::ZeroIfFrozen)
    if ripple_state_helpers::is_frozen(view, owner, &issue) {
        return STAmount::default();
    }
    ripple_state_helpers::account_holds(view, owner, &issue.account, issue.currency)
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
///   If remaining_out < stpAmt.out: recompute from requested output
///   If remaining_in < stpAmt.in: recompute from remaining input
fn compute_offer_consumption(
    remaining_in: &STAmount,
    remaining_out: &STAmount,
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

    // reference: BookStep.cpp `limitStepOut` in `revImp`. The reverse pass
    // receives an unbounded input and must not consume more than the output
    // requested by the following step. `offer.limitOut(..., true)` delegates
    // to Quality::ceilOut, which uses mulRound with round-away-from-zero.
    // Do not use cross_type_scale here: its unchecked native conversion can
    // construct an out-of-range XRP STAmount before the bounded offer input
    // has been derived.
    if *remaining_out < stp_out {
        let offer_amounts = Amounts::new(actual_ofr_in.clone(), actual_ofr_out.clone());
        let clipped = Quality::from_amounts(&offer_amounts).ceil_out(&offer_amounts, remaining_out);
        actual_ofr_in = clipped.r#in;
        actual_ofr_out = clipped.out;
        stp_out = actual_ofr_out.clone();
        owner_gives = mul_ratio_amount(&stp_out, transfer_rate_out, QUALITY_ONE, false);
        stp_in = mul_ratio_amount(&actual_ofr_in, transfer_rate_in, QUALITY_ONE, true);
    }

    // reference: limitStepIn if remaining_in < stpAmt.in
    if *remaining_in < stp_in {
        stp_in = remaining_in.clone();
        let in_lmt = mul_ratio_amount(&stp_in, QUALITY_ONE, transfer_rate_in, false);
        // Reference: rippled `BookStep.cpp::limitStepIn` is generic over
        // `TIn`/`TOut`; this path must remain valid for IOU, XRP, and MPT
        // inputs and must not inspect an amount as XRP merely for diagnostics.
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
    let mantissa = (result_mantissa as u64).clamp(1_000_000_000_000_000, 9_999_999_999_999_999);

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
#[path = "book_step_success_path_tests.rs"]
mod success_path_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{STArray, STObject, StBase};

    #[test]
    fn amm_target_threshold_is_fix_and_single_path_conditional() {
        let input = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(2));
        let output = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1));
        let tip = Quality::from_amounts(&Amounts::new(input.clone(), output.clone()));
        let limit = Quality::from_amounts(&Amounts::new(output, input));
        assert!(limit > tip);

        assert_eq!(
            amm_target_quality(Some(tip), Some(limit), true, false),
            None
        );
        assert_eq!(
            amm_target_quality(Some(tip), Some(limit), false, false),
            Some(tip)
        );
        assert_eq!(
            amm_target_quality(Some(tip), Some(limit), true, true),
            Some(tip)
        );
    }

    fn amm_entry_with_auction_slot(
        owner: AccountID,
        authorized: AccountID,
        expiration: u32,
    ) -> STLedgerEntry {
        let mut amm = STLedgerEntry::from_type_and_key(
            protocol::LedgerEntryType::AMM,
            Uint256::from_array([0xA1; 32]),
        );
        amm.set_field_u16(sf("sfTradingFee"), 500);
        let mut slot = STObject::make_inner_object(sf("sfAuctionSlot"));
        slot.set_account_id(sf("sfAccount"), owner);
        slot.set_field_u16(sf("sfDiscountedFee"), 25);
        slot.set_field_u32(sf("sfExpiration"), expiration);
        let mut auth = STArray::new(sf("sfAuthAccounts"));
        let mut auth_entry = STObject::make_inner_object(sf("sfAuthAccount"));
        auth_entry.set_account_id(sf("sfAccount"), authorized);
        auth.push_back(auth_entry);
        slot.set_field_array(sf("sfAuthAccounts"), auth);
        amm.set_field_object(sf("sfAuctionSlot"), slot);
        amm
    }

    fn canonical_20106714_amm_offer() -> (SyntheticAmmOffer, STAmount, STAmount) {
        let issuer = protocol::parse_base58_account_id("r4gZcWbPcG2M8zcHmXiMLBtHGu4ZpN9cLS")
            .expect("canonical UAH issuer");
        let account = protocol::parse_base58_account_id("rPB6rsP7SjpsbHV6hcEB9imSyr9GBXFerd")
            .expect("canonical AMM account");
        let uah = protocol::Issue::new(protocol::currency_from_string("UAH"), issuer);
        let pool_in = STAmount::from_iou_amount(
            sf("sfAmount"),
            protocol::IOUAmount::from_parts(1_888_427_639_376_993, -12)
                .expect("canonical UAH pool"),
            uah,
        );
        let pool_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(42_375_381));
        let amount_out = amm_max_output(&pool_out).expect("max AMM output");
        let amount_in =
            amm_swap_asset_out(&pool_in, &pool_out, &amount_out, 500, true).expect("max AMM input");
        let offer = SyntheticAmmOffer {
            account,
            quality: Quality::from_amounts(&Amounts::new(pool_in.clone(), pool_out.clone())),
            pool_in,
            pool_out,
            amount_in,
            amount_out,
            trading_fee: 500,
            amm_rounding_enabled: true,
            multi_path: false,
            fix_reduced_offers_v2: false,
        };

        let max_in = STAmount::from_iou_amount(
            sf("sfAmount"),
            protocol::IOUAmount::from_parts(4_999_970_751_325_331, -13)
                .expect("tick-rounded UAH limit"),
            uah,
        );
        let max_out = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(8_205_452));
        (offer, max_in, max_out)
    }

    #[test]
    fn overflow_offer_amendment_switches_max_offer_shape() {
        let (offer, _, _) = canonical_20106714_amm_offer();
        let legacy = amm_max_offer_amounts(
            &offer.pool_in,
            &offer.pool_out,
            offer.trading_fee,
            offer.amm_rounding_enabled,
            false,
        )
        .expect("legacy max offer");
        let fixed = amm_max_offer_amounts(
            &offer.pool_in,
            &offer.pool_out,
            offer.trading_fee,
            offer.amm_rounding_enabled,
            true,
        )
        .expect("fixed max offer");
        assert_eq!(
            legacy.0,
            protocol::to_max_amount::<STAmount>(offer.pool_in.asset())
        );
        assert_eq!(
            fixed.1,
            amm_max_output(&offer.pool_out).expect("99% output")
        );
        assert_ne!(legacy, fixed);
    }

    #[test]
    fn invariant_failure_is_fatal_only_after_overflow_offer_fix() {
        assert!(!amm_invariant_failure_is_fatal(false, false));
        assert!(amm_invariant_failure_is_fatal(false, true));
        assert!(!amm_invariant_failure_is_fatal(true, false));
        assert!(!amm_invariant_failure_is_fatal(true, true));
    }

    #[test]
    fn max_amm_offer_gates_with_pool_spot_quality_before_limits() {
        // AMMLiquidity::maxOffer carries Quality{balances}, not the effective
        // quality of buying 99% of the output pool. This distinction changes
        // the conclusion for Testnet ledger 20,106,714: the pool spot and the
        // requested slice both satisfy this OfferCreate boundary.
        let (offer, max_in, max_out) = canonical_20106714_amm_offer();
        let threshold = Quality::from_amounts(&Amounts::new(max_in.clone(), max_out.clone()));

        assert!(quality_satisfies_threshold(
            offer.quality(),
            Some(threshold)
        ));
        let max_amount_quality = Quality::from_amounts(&Amounts::new(
            offer.amount_in.clone(),
            offer.amount_out.clone(),
        ));
        assert!(!quality_satisfies_threshold(
            max_amount_quality,
            Some(threshold)
        ));

        let (limited_in, limited_out) = offer
            .limit(&max_in, &max_out)
            .expect("requested AMM slice is arithmetically available");
        let limited_quality =
            Quality::from_amounts(&Amounts::new(limited_in.clone(), limited_out.clone()));
        assert!(quality_satisfies_threshold(
            limited_quality,
            Some(threshold)
        ));
        assert_eq!(limited_out, max_out);
        assert!(limited_in < max_in);
    }

    #[test]
    fn qualifying_max_amm_offer_is_limited_only_after_quality_gate() {
        let (offer, max_in, max_out) = canonical_20106714_amm_offer();
        let permissive = offer.quality();

        assert!(quality_satisfies_threshold(
            offer.quality(),
            Some(permissive)
        ));
        let (limited_in, limited_out) = offer
            .limit(&max_in, &max_out)
            .expect("qualifying AMM offer should execute within request limits");
        assert_eq!(limited_out, max_out);
        assert!(limited_in > max_in.zeroed());
        assert!(limited_in < max_in);
    }

    #[test]
    fn amm_invariant_accepts_bounded_swap_and_rejects_overconsumption() {
        let (offer, max_in, max_out) = canonical_20106714_amm_offer();
        let (input, output) = offer.limit(&max_in, &max_out).expect("bounded swap");
        assert!(amm_offer_invariant_holds(&offer, &input, &output));
        assert!(!amm_offer_invariant_holds(
            &offer,
            &(offer.amount_in.clone() + max_in),
            &output,
        ));
    }

    #[test]
    fn multipath_amm_limit_preserves_generated_quality() {
        let (mut offer, max_in, max_out) = canonical_20106714_amm_offer();
        offer.multi_path = true;
        let original = Amounts::new(offer.amount_in.clone(), offer.amount_out.clone());
        let expected_out = offer.quality.ceil_out_strict(&original, &max_out, true);
        let after_out = offer
            .limit(&offer.amount_in, &max_out)
            .expect("limited offer");
        assert_eq!(after_out, (expected_out.r#in, expected_out.out));

        offer.fix_reduced_offers_v2 = true;
        let expected_in = offer.quality.ceil_in_strict(&original, &max_in, false);
        let after_in = offer
            .limit(&max_in, &offer.amount_out)
            .expect("limited offer");
        assert_eq!(after_in, (expected_in.r#in, expected_in.out));
    }

    #[test]
    fn legacy_multipath_amm_input_limit_uses_non_strict_ceil() {
        let (mut offer, max_in, _) = canonical_20106714_amm_offer();
        offer.multi_path = true;
        offer.fix_reduced_offers_v2 = false;
        let original = Amounts::new(offer.amount_in.clone(), offer.amount_out.clone());
        let expected = offer.quality.ceil_in(&original, &max_in);
        let limited = offer
            .limit(&max_in, &offer.amount_out)
            .expect("limited offer");
        assert_eq!(limited, (expected.r#in, expected.out));
    }

    #[test]
    fn book_step_offer_cap_matches_rippled() {
        assert_eq!(MAX_OFFERS_TO_CONSUME, 1000);
    }

    #[test]
    fn payment_consumes_taker_owned_offer_but_default_offer_create_cancels_it() {
        let taker = AccountID::from_array([0x44; 20]);
        let destination = AccountID::from_array([0x55; 20]);
        let limit = Some(Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
        )));
        let good = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(200)),
        ));
        let worse = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(50)),
        ));
        assert!(!is_self_crossing_offer(
            true,
            Some(&taker),
            Some(&destination),
            &taker,
            good,
            limit
        ));
        assert!(!is_self_crossing_offer(
            false,
            Some(&taker),
            Some(&taker),
            &taker,
            good,
            limit
        ));
        assert!(is_self_crossing_offer(
            true,
            Some(&taker),
            Some(&taker),
            &taker,
            good,
            limit
        ));
        assert!(!is_self_crossing_offer(
            true,
            Some(&taker),
            Some(&taker),
            &taker,
            worse,
            limit
        ));
        assert!(!is_self_crossing_offer(
            true,
            Some(&taker),
            Some(&taker),
            &taker,
            good,
            None
        ));
    }

    #[test]
    fn fee_adjusted_theoretical_quality_can_reorder_multistrand_books() {
        let raw_better = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(2)),
        ));
        let raw_worse = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(2)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(3)),
        ));
        assert!(raw_better > raw_worse);

        // A 50% input transfer rate applies when the preceding DirectStep is
        // redeeming. The raw 2.0 book therefore falls below the fee-free 1.5
        // book and ActiveStrands must reverse their raw-quality order.
        let adjusted = compose_transfer_quality(raw_better, 1_500_000_000, QUALITY_ONE);
        assert!(adjusted < raw_worse);
    }

    #[test]
    fn amm_auction_slot_discount_uses_bookstep_taker_context() {
        let owner = AccountID::from_array([0x11; 20]);
        let authorized = AccountID::from_array([0x22; 20]);
        let stranger = AccountID::from_array([0x33; 20]);
        let amm = amm_entry_with_auction_slot(owner, authorized, 1_000);

        assert_eq!(amm_trading_fee(999, &amm, Some(&owner)), 25);
        assert_eq!(amm_trading_fee(999, &amm, Some(&authorized)), 25);
        assert_eq!(amm_trading_fee(999, &amm, Some(&stranger)), 500);
        assert_eq!(amm_trading_fee(999, &amm, None), 500);
        assert_eq!(amm_trading_fee(1_000, &amm, Some(&owner)), 500);
    }

    #[test]
    fn clob_tip_generates_quality_bounded_amm_offer_before_execution_limits() {
        let (max_offer, requested_in, requested_out) = canonical_20106714_amm_offer();
        let clob_tip = Quality::from_amounts(&Amounts::new(requested_in, requested_out));
        let (amount_in, amount_out) = amm_offer_for_clob_quality(
            &max_offer.pool_in,
            &max_offer.pool_out,
            clob_tip,
            max_offer.trading_fee,
            max_offer.amm_rounding_enabled,
        )
        .expect("favorable AMM spot should produce a CLOB-tip-bounded offer");
        let generated = Quality::from_amounts(&Amounts::new(amount_in, amount_out.clone()));

        assert!(generated >= clob_tip);
        assert!(amount_out < max_offer.amount_out);
    }

    #[test]
    fn amm_offer_establishes_quality_directory_before_clob_tip() {
        let (max_offer, requested_in, requested_out) = canonical_20106714_amm_offer();
        let clob_tip = Quality::from_amounts(&Amounts::new(requested_in, requested_out));
        let (amount_in, amount_out) = amm_offer_for_clob_quality(
            &max_offer.pool_in,
            &max_offer.pool_out,
            clob_tip,
            max_offer.trading_fee,
            max_offer.amm_rounding_enabled,
        )
        .expect("AMM should compete with the CLOB tip");
        let amm_quality = Quality::from_amounts(&Amounts::new(amount_in, amount_out));
        let mut step_quality = None;

        assert!(accepts_step_quality(&mut step_quality, amm_quality));
        assert_eq!(step_quality, Some(amm_quality));
        assert_eq!(
            accepts_step_quality(&mut step_quality, clob_tip),
            amm_quality == clob_tip
        );
    }

    #[test]
    fn multipath_fibonacci_offer_uses_initial_pool_and_shared_iteration() {
        let (pool, _, _) = canonical_20106714_amm_offer();
        let first = generate_fibonacci_amm_offer(
            &pool.pool_in,
            &pool.pool_out,
            &pool.pool_in,
            &pool.pool_out,
            pool.trading_fee,
            true,
            0,
        )
        .expect("initial Fibonacci AMM offer");
        let third = generate_fibonacci_amm_offer(
            &pool.pool_in,
            &pool.pool_out,
            &pool.pool_in,
            &pool.pool_out,
            pool.trading_fee,
            true,
            2,
        )
        .expect("third Fibonacci AMM offer");

        assert!(first.0.signum() > 0 && first.1.signum() > 0);
        assert!(third.0 > first.0);
        assert!(third.1 > first.1);
        assert_eq!(
            crate::domain::amm_helpers::stamount_as_number(&third.1),
            crate::domain::amm_helpers::stamount_as_number(&first.1)
                * RuntimeNumber::from_i64_and_exponent(2, 0)
        );
    }

    #[test]
    fn shared_amm_context_counts_only_selected_used_iterations() {
        let account = AccountID::from_array([0x44; 20]);
        let context = crate::domain::flow_engine::AmmContext::new(account, true);
        assert!(context.multi_path());
        assert_eq!(context.iterations(), 0);

        context.set_amm_used();
        context.clear();
        context.update();
        assert_eq!(
            context.iterations(),
            0,
            "discarded candidate must not count"
        );

        context.set_amm_used();
        context.update();
        assert_eq!(context.iterations(), 1);
        assert_eq!(context.account(), account);
    }

    #[test]
    fn amm_swap_asset_out_par_xrp_regression_uses_typed_rounding() {
        // This concrete pool has a 0.22% trading fee and is deliberately
        // output-limited at 980 XRP. `swapAssetOut` must derive the input
        // from the requested output, not from a float forward estimate.
        let par = protocol::Issue::new(
            protocol::currency_from_string("PAR"),
            AccountID::from_array([0xA5; 20]),
        );
        let pool_par = STAmount::from_iou_amount(
            sf("sfAmount"),
            protocol::IOUAmount::from_parts(4_752_046_925_146_200, -11).expect("PAR pool amount"),
            par,
        );
        let pool_xrp = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(49_980_000_000));
        let requested_xrp = STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(980_000_000));

        let required_par = amm_swap_asset_out(&pool_par, &pool_xrp, &requested_xrp, 220, true)
            .expect("output below pool balance must be swappable");

        assert_eq!(
            required_par.iou(),
            protocol::IOUAmount::from_parts(9_525_048_958_000_000, -13)
                .expect("canonical required PAR")
        );
        assert_eq!(required_par.text(), "952.5048958");
    }

    #[test]
    fn malformed_xah_to_xrp_book_is_identified_without_lookup_normalization() {
        let issuer = protocol::parse_base58_account_id("rswh1fvyLqHizBS2awu1vs6QcmwTBd9qiv")
            .expect("canonical XAH issuer must parse");
        let xah = protocol::currency_from_string("XAH");
        let raw = Book {
            r#in: Asset::Issue(protocol::Issue::new(xah, issuer)),
            // A nonzero issuer on XRP violates the keylet contract. The
            // lookup boundary must diagnose it, not silently select a
            // different canonical book.
            out: Asset::Issue(protocol::Issue::new(protocol::xrp_currency(), issuer)),
            domain: None,
        };

        let raw_protocol = protocol::Book::new(raw.r#in, raw.out, raw.domain);
        assert!(!protocol::is_consistent_book(raw_protocol));

        let (input_currency, input_issuer) = book_asset_identity(raw.r#in);
        let (output_currency, output_issuer) = book_asset_identity(raw.out);
        assert_eq!(input_currency, "XAH");
        assert_eq!(input_issuer, protocol::to_base58(issuer));
        assert_eq!(output_currency, "XRP");
        assert_eq!(output_issuer, protocol::to_base58(issuer));
    }

    #[test]
    fn quality_threshold_rejects_worse_crossing() {
        let threshold = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
        ));
        let better = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(200)),
        ));
        let worse = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(100)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(50)),
        ));

        assert!(quality_satisfies_threshold(better, Some(threshold)));
        assert!(!quality_satisfies_threshold(worse, Some(threshold)));
    }

    #[test]
    fn explicit_offer_crossing_does_not_enforce_component_quality() {
        let threshold = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(2)),
        ));
        let worse = Quality::from_amounts(&Amounts::new(
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1)),
            STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(1)),
        ));
        assert!(rejects_step_quality(true, worse, Some(threshold)));
        assert!(!rejects_step_quality(false, worse, Some(threshold)));
    }

    #[test]
    fn transfer_rate_endpoint_prefers_strand_destination_over_taker() {
        let source = AccountID::from_array([0x11; 20]);
        let destination = AccountID::from_array([0x22; 20]);
        assert_eq!(
            effective_strand_dst(Some(&destination), Some(&source)),
            Some(&destination)
        );
    }

    #[test]
    fn single_path_amm_fee_function_honors_fix_gate_only_for_offer_crossing() {
        assert!(should_compose_single_path_input_rate(true, false, false));
        assert!(!should_compose_single_path_input_rate(true, true, false));
        assert!(should_compose_single_path_input_rate(true, true, true));
        assert!(!should_compose_single_path_input_rate(false, false, true));
    }
    #[test]
    fn reverse_xrp_to_iou_output_cap_derives_only_required_3300_xrp_sendmax() {
        // Regression for an XRP→IOU self-payment shape with a 3,300 XRP
        // SendMax. The reverse probe is input-unbounded, but a 1-IOU request
        // must consume only 3.3 XRP from a 3,300-XRP-for-1,000-IOU offer.
        // rippled BookStep.cpp applies this through limitStepOut before
        // deriving stpAmt.in in revImp.
        let issuer = AccountID::from_array([0x33; 20]);
        let issue = protocol::Issue::new(protocol::currency_from_string("USD"), issuer);
        let consumption = compute_offer_consumption(
            &STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(3_300_000_000)),
            &STAmount::from_iou_amount(
                sf("sfAmount"),
                protocol::IOUAmount::from_parts(1, 0).expect("canonical one IOU"),
                issue,
            ),
            &STAmount::from_xrp_amount(protocol::XRPAmount::from_drops(3_300_000_000)),
            &STAmount::from_iou_amount(
                sf("sfAmount"),
                protocol::IOUAmount::from_parts(1_000, 0).expect("canonical offer output"),
                issue,
            ),
            &STAmount::from_iou_amount(
                sf("sfAmount"),
                protocol::IOUAmount::from_parts(1_000, 0).expect("funded offer output"),
                issue,
            ),
            QUALITY_ONE,
            QUALITY_ONE,
        );

        assert_eq!(consumption.step_in.xrp().drops(), 3_300_000);
        assert_eq!(consumption.step_out.iou().to_string(), "1");
        assert_eq!(consumption.offer_in.xrp().drops(), 3_300_000);
        assert_eq!(consumption.offer_out.iou().to_string(), "1");
    }

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

/// Construct a transfer amount with the exact executable Book asset while
/// preserving its numeric representation. Pool calculations may originate
/// from a RippleState balance whose embedded issue is oriented differently.
fn normalize_amount_to_asset(amount: &STAmount, asset: Asset) -> STAmount {
    if amount.asset() == asset {
        return amount.clone();
    }
    STAmount::new_with_asset(
        sf("sfAmount"),
        asset,
        amount.mantissa(),
        amount.exponent(),
        amount.negative(),
    )
}
