//! Step execution for the flow engine.
//!
//! A strand is evaluated backwards first and then, when a step limits the
//! request, forwards from that step.  `StepAmount` deliberately carries the
//! ledger asset with the numeric amount: a SendMax in USD must never be
//! compared with a requested XRP delivery.

use super::{SelfCrossCancellation, StepKind};
use crate::ApplyView;
use crate::domain::ripple_state_helpers;
use protocol::{
    AccountID, Asset, Issue, Quality, STAmount, Ter, XRPAmount, get_field_by_symbol as sf,
    xrp_account,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AmountTag {
    ExactAsset(Asset),
    Currency(protocol::Currency),
}

/// A ledger amount tagged with the identity valid for this step. Book and
/// endpoint quantities retain an exact asset; direct trust-line quantities are
/// deliberately tagged by currency because an issuer representation changes as
/// value ripples across a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepAmount {
    amount: STAmount,
    tag: AmountTag,
}

impl StepAmount {
    pub fn new(amount: STAmount) -> Self {
        Self {
            tag: AmountTag::ExactAsset(amount.asset()),
            amount,
        }
    }

    fn with_currency(amount: STAmount, currency: protocol::Currency) -> Self {
        Self {
            amount,
            tag: AmountTag::Currency(currency),
        }
    }

    pub fn amount(&self) -> &STAmount {
        &self.amount
    }

    pub fn asset(&self) -> Asset {
        self.amount.asset()
    }

    pub fn is_zero(&self) -> bool {
        self.amount.signum() <= 0
    }

    /// `Some` only when both amounts have the same step-valid identity.
    pub fn greater_than(&self, other: &STAmount) -> Option<bool> {
        self.matches_amount(other).then(|| self.amount > *other)
    }

    fn matches_amount(&self, other: &STAmount) -> bool {
        match self.tag {
            AmountTag::ExactAsset(asset) => asset == other.asset(),
            AmountTag::Currency(currency) => {
                if protocol::is_xrp_currency(currency) {
                    other.native()
                } else {
                    !other.native() && other.issue().currency == currency
                }
            }
        }
    }

    pub(crate) fn equivalent(&self, other: &StepAmount) -> bool {
        let same_unit = match (self.tag, other.tag) {
            (AmountTag::ExactAsset(lhs), AmountTag::ExactAsset(rhs)) => lhs == rhs,
            (AmountTag::Currency(currency), _) | (_, AmountTag::Currency(currency)) => {
                self.matches_currency(currency) && other.matches_currency(currency)
            }
        };
        same_unit
            && self.amount.mantissa() == other.amount.mantissa()
            && self.amount.exponent() == other.amount.exponent()
            && self.amount.negative() == other.amount.negative()
    }

    fn matches_book_asset(&self, asset: Asset) -> bool {
        match self.tag {
            AmountTag::ExactAsset(actual) => actual == asset,
            AmountTag::Currency(currency) => match asset {
                Asset::Issue(issue) => !issue.native() && issue.currency == currency,
                Asset::MPTIssue(_) => false,
            },
        }
    }

    fn matches_currency(&self, currency: protocol::Currency) -> bool {
        self.amount.native() == protocol::is_xrp_currency(currency)
            && (self.amount.native() || self.amount.issue().currency == currency)
    }
}

/// Input/output pair returned by every `rev` and `fwd` call.
#[derive(Debug, Clone)]
pub struct StepAmounts {
    pub input: StepAmount,
    pub output: StepAmount,
}

impl StepAmounts {
    fn new(input: STAmount, output: STAmount) -> Self {
        Self {
            input: StepAmount::new(input),
            output: StepAmount::new(output),
        }
    }

    fn direct(input: STAmount, output: STAmount, currency: protocol::Currency) -> Self {
        Self {
            input: StepAmount::with_currency(input, currency),
            output: StepAmount::with_currency(output, currency),
        }
    }
}

/// Immutable execution context shared by a strand's concrete steps.
#[derive(Debug, Clone)]
pub struct StepContext<'a> {
    pub strand_src: &'a AccountID,
    pub quality_threshold: Option<Quality>,
    /// Present only for direct/default OfferCreate crossings. It is separate
    /// from the flow sandbox so self-offer cancellations survive a dry flow.
    pub self_cross_cancellation: Option<SelfCrossCancellation>,
}

/// The Rust counterpart to rippled's `Step`.  Both directions mutate only the
/// sandbox supplied by the caller and return typed input/output quantities.
pub trait FlowStep {
    fn rev<V: ApplyView>(
        &self,
        view: &mut V,
        requested_out: &StepAmount,
        context: &StepContext<'_>,
    ) -> Result<StepAmounts, Ter>;

    fn fwd<V: ApplyView>(
        &self,
        view: &mut V,
        requested_in: &StepAmount,
        reverse_cache: &StepAmounts,
        context: &StepContext<'_>,
    ) -> Result<StepAmounts, Ter>;
}

impl FlowStep for StepKind {
    fn rev<V: ApplyView>(
        &self,
        view: &mut V,
        requested_out: &StepAmount,
        context: &StepContext<'_>,
    ) -> Result<StepAmounts, Ter> {
        match self {
            StepKind::Direct { src, dst, currency } => {
                if !requested_out.matches_currency(*currency) {
                    return Err(Ter::TEF_INTERNAL);
                }
                let (input, output) = execute_direct_fwd(view, src, dst, requested_out.amount())?;
                Ok(StepAmounts::direct(input, output, *currency))
            }
            StepKind::XrpEndpoint { account, is_last } => {
                if !requested_out.amount().native() {
                    return Err(Ter::TEF_INTERNAL);
                }
                let (input, output) =
                    execute_xrp_endpoint_fwd(view, account, *is_last, requested_out.amount())?;
                Ok(StepAmounts::new(input, output))
            }
            StepKind::Book {
                book_in,
                book_out,
                owner_pays_transfer_fee,
                remove_self_crossing,
            } => {
                let in_asset = Asset::Issue(*book_in);
                let out_asset = Asset::Issue(*book_out);
                if !requested_out.matches_book_asset(out_asset) {
                    return Err(Ter::TEF_INTERNAL);
                }
                let book = crate::domain::ripple_calc::book_step::Book {
                    r#in: in_asset,
                    out: out_asset,
                    domain: None,
                };
                // Reverse book execution asks for output and supplies an
                // effectively unbounded input.  The book consumes only the
                // input required to produce the requested output.
                let requested_out = normalize_amount_asset(requested_out.amount(), out_asset);
                let result = crate::domain::ripple_calc::book_step::execute_book_step_with_options(
                    view,
                    &book,
                    &unlimited_amount(in_asset),
                    &requested_out,
                    crate::domain::ripple_calc::book_step::BookStepOptions {
                        owner_pays_transfer_fee: *owner_pays_transfer_fee,
                        taker: Some(context.strand_src),
                        quality_threshold: context.quality_threshold,
                        remove_self_crossing: *remove_self_crossing,
                        self_cross_cancellation: context.self_cross_cancellation.clone(),
                    },
                );
                if result.ter != Ter::TES_SUCCESS {
                    return Err(result.ter);
                }
                Ok(StepAmounts::new(result.amount_in, result.amount_out))
            }
        }
    }

    fn fwd<V: ApplyView>(
        &self,
        view: &mut V,
        requested_in: &StepAmount,
        reverse_cache: &StepAmounts,
        context: &StepContext<'_>,
    ) -> Result<StepAmounts, Ter> {
        match self {
            StepKind::Direct { src, dst, currency } => {
                if !requested_in.matches_currency(*currency) {
                    return Err(Ter::TEF_INTERNAL);
                }
                let (input, output) = execute_direct_fwd(view, src, dst, requested_in.amount())?;
                Ok(StepAmounts::direct(input, output, *currency))
            }
            StepKind::XrpEndpoint { account, is_last } => {
                if !requested_in.amount().native() {
                    return Err(Ter::TEF_INTERNAL);
                }
                let (input, output) =
                    execute_xrp_endpoint_fwd(view, account, *is_last, requested_in.amount())?;
                Ok(StepAmounts::new(input, output))
            }
            StepKind::Book {
                book_in,
                book_out,
                owner_pays_transfer_fee,
                remove_self_crossing,
            } => {
                let in_asset = Asset::Issue(*book_in);
                let out_asset = Asset::Issue(*book_out);
                if !requested_in.matches_book_asset(in_asset)
                    || !reverse_cache.output.matches_book_asset(out_asset)
                {
                    return Err(Ter::TEF_INTERNAL);
                }
                let book = crate::domain::ripple_calc::book_step::Book {
                    r#in: in_asset,
                    out: out_asset,
                    domain: None,
                };
                let requested_in = normalize_amount_asset(requested_in.amount(), in_asset);
                let reverse_out = normalize_amount_asset(reverse_cache.output.amount(), out_asset);
                let result = crate::domain::ripple_calc::book_step::execute_book_step_with_options(
                    view,
                    &book,
                    &requested_in,
                    &reverse_out,
                    crate::domain::ripple_calc::book_step::BookStepOptions {
                        owner_pays_transfer_fee: *owner_pays_transfer_fee,
                        taker: Some(context.strand_src),
                        quality_threshold: context.quality_threshold,
                        remove_self_crossing: *remove_self_crossing,
                        self_cross_cancellation: context.self_cross_cancellation.clone(),
                    },
                );
                if result.ter != Ter::TES_SUCCESS {
                    return Err(result.ter);
                }
                Ok(StepAmounts::new(result.amount_in, result.amount_out))
            }
        }
    }
}

fn normalize_amount_asset(amount: &STAmount, asset: Asset) -> STAmount {
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

fn unlimited_amount(asset: Asset) -> STAmount {
    if asset.native() {
        // The maximum valid XRPAmount is well above any practical ledger
        // balance and avoids a cross-asset sentinel.
        STAmount::from_xrp_amount(XRPAmount::from_drops(
            protocol::ST_AMOUNT_MAX_NATIVE_NETWORK as i64,
        ))
    } else if let Asset::MPTIssue(issue) = asset {
        STAmount::from_mpt_amount(
            sf("sfAmount"),
            protocol::MPTAmount::from_value(protocol::MAX_MP_TOKEN_AMOUNT),
            issue,
        )
    } else {
        STAmount::new_with_asset(sf("sfAmount"), asset, u64::MAX / 2, 0, false)
    }
}

/// Limits delivery to trust-line capacity.  It is used by both directions;
/// reverse execution requests an output and receives the asset-correct input
/// that would be consumed by forward execution.
fn execute_direct_fwd<V: ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    input: &STAmount,
) -> Result<(STAmount, STAmount), Ter> {
    if input.signum() <= 0 {
        return Ok((input.zeroed(), input.zeroed()));
    }

    if input.native() {
        let result = ripple_state_helpers::account_send(view, src, dst, input);
        if result != Ter::TES_SUCCESS {
            return Err(result);
        }
        return Ok((input.clone(), input.clone()));
    }

    let currency = input.issue().currency;
    let (max_flow, debt_dir) =
        crate::domain::ripple_calc::direct_step::max_payment_flow(view, src, dst, currency);
    if max_flow.is_zero() || max_flow.signum() <= 0 {
        return Ok((input.zeroed(), input.zeroed()));
    }

    let input_iou = input.iou();
    let step_issue = Issue {
        currency,
        account: if debt_dir == crate::domain::ripple_calc::direct_step::DebtDirection::Redeems {
            *dst
        } else {
            *src
        },
    };
    let rate = if debt_dir == crate::domain::ripple_calc::direct_step::DebtDirection::Redeems {
        ripple_state_helpers::transfer_rate(view, dst)
    } else {
        crate::domain::mul_ratio::QUALITY_ONE
    };
    let has_rate = rate > crate::domain::mul_ratio::QUALITY_ONE;
    let effective_max = if has_rate {
        crate::domain::mul_ratio::mul_ratio(
            max_flow,
            crate::domain::mul_ratio::QUALITY_ONE,
            rate,
            false,
        )
    } else {
        max_flow
    };
    let deliver_iou = input_iou.min(effective_max);
    let deliver = STAmount::from_iou_amount(sf("sfAmount"), deliver_iou, step_issue);
    if deliver.signum() <= 0 {
        return Ok((input.zeroed(), input.zeroed()));
    }
    let consumed = if has_rate {
        let adjusted_iou = crate::domain::mul_ratio::mul_ratio(
            deliver_iou,
            rate,
            crate::domain::mul_ratio::QUALITY_ONE,
            true,
        );
        STAmount::from_iou_amount(sf("sfAmount"), adjusted_iou, step_issue)
    } else {
        deliver.clone()
    };
    let result = ripple_state_helpers::account_send(view, src, dst, &consumed);
    if result != Ter::TES_SUCCESS {
        return Err(result);
    }
    Ok((consumed, deliver))
}

fn execute_xrp_endpoint_fwd<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    is_last: bool,
    input: &STAmount,
) -> Result<(STAmount, STAmount), Ter> {
    let drops = input.xrp().drops();
    if drops <= 0 {
        let zero = STAmount::from_xrp_amount(XRPAmount::from_drops(0));
        return Ok((zero.clone(), zero));
    }
    let actual_drops = if !is_last {
        drops.min(xrp_liquid(view, account))
    } else {
        drops
    };
    if actual_drops <= 0 {
        let zero = STAmount::from_xrp_amount(XRPAmount::from_drops(0));
        return Ok((zero.clone(), zero));
    }
    let sender = if is_last { xrp_account() } else { *account };
    let receiver = if is_last { *account } else { xrp_account() };
    let ter = ripple_state_helpers::transfer_xrp(
        view,
        &sender,
        &receiver,
        XRPAmount::from_drops(actual_drops),
    );
    if ter != Ter::TES_SUCCESS {
        return Err(ter);
    }
    let amount = STAmount::from_xrp_amount(XRPAmount::from_drops(actual_drops));
    Ok((amount.clone(), amount))
}

fn xrp_liquid<V: ApplyView>(view: &mut V, account: &AccountID) -> i64 {
    let acct_keylet =
        protocol::account_keylet(basics::base_uint::Uint160::from_void(account.data()));
    let Some(sle) = view.peek(acct_keylet).ok().flatten() else {
        return 0;
    };
    let balance = sle.get_field_amount(sf("sfBalance")).xrp().drops();
    let owner_count = sle.get_field_u32(sf("sfOwnerCount"));
    let reserve = view.fees().account_reserve(owner_count as usize) as i64;
    (balance - reserve).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_amount_rejects_cross_asset_send_max_comparison() {
        let issuer = AccountID::from_array([9; 20]);
        let usd = protocol::currency_from_string("USD");
        let required = StepAmount::new(STAmount::from_iou_amount(
            sf("sfAmount"),
            protocol::IOUAmount::from_parts(10, 0).expect("valid iou"),
            Issue::new(usd, issuer),
        ));
        let xrp_send_max = STAmount::from_xrp_amount(XRPAmount::from_drops(1_000_000));

        assert_eq!(required.greater_than(&xrp_send_max), None);
    }
}
