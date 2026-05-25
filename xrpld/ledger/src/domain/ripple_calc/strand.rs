//! Strand construction and execution helpers for RippleCalc explicit paths.
// Kept for compatibility with the broader explicit-path normalization and strand
// surfaces; the current Rust owner still routes payments through the narrower
// landed execution seam in `ripple_calc::mod`.
#![allow(dead_code)]

use std::collections::BTreeSet;

use protocol::{
    AccountID, Amounts, Asset, PathAsset, Quality, STAmount, STPath, STPathElement, Ter,
    get_field_by_symbol,
};

use crate::{ApplyView, ReadView, ViewError};

use super::xrp_endpoint_step::XrpEndpointStep;
use super::{
    QUALITY_ONE, RippleCalcOutput, account_to_uint160, apply_direct_iou_transfer,
    book_step::{estimate_explicit_book_step, execute_explicit_book_step},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrandError {
    Ter(Ter),
    View(ViewError),
}

impl From<Ter> for StrandError {
    fn from(value: Ter) -> Self {
        Self::Ter(value)
    }
}

impl From<ViewError> for StrandError {
    fn from(value: ViewError) -> Self {
        Self::View(value)
    }
}

impl StrandError {
    pub fn ter(&self) -> Option<Ter> {
        match self {
            Self::Ter(ter) => Some(*ter),
            Self::View(_) => None,
        }
    }
}

pub type StrandResult<T> = Result<T, StrandError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedPath {
    pub source: AccountID,
    pub destination: AccountID,
    pub deliver: Asset,
    pub send_max: Option<Asset>,
    pub initial_asset: Asset,
    pub original_path: STPath,
    pub elements: Vec<STPathElement>,
    pub is_default_path: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountTransferStep {
    pub source: AccountID,
    pub destination: AccountID,
    pub asset: Asset,
}

impl AccountTransferStep {
    pub fn direct_accounts(self) -> (AccountID, AccountID) {
        (self.source, self.destination)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookStep {
    pub input: Asset,
    pub output: Asset,
}

impl BookStep {
    pub fn direct_accounts(self) -> (AccountID, AccountID) {
        (self.input.issuer(), self.output.issuer())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrandStep {
    AccountTransfer(AccountTransferStep),
    Book(BookStep),
    XrpEndpoint(XrpEndpointStep),
}

impl StrandStep {
    pub fn direct_accounts(self) -> (AccountID, AccountID) {
        match self {
            Self::AccountTransfer(step) => step.direct_accounts(),
            Self::Book(step) => step.direct_accounts(),
            Self::XrpEndpoint(step) => step.direct_accounts(),
        }
    }

    pub fn book(self) -> Option<BookStep> {
        match self {
            Self::Book(step) => Some(step),
            Self::AccountTransfer(_) | Self::XrpEndpoint(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strand {
    pub source: AccountID,
    pub destination: AccountID,
    pub deliver: Asset,
    pub initial_asset: Asset,
    pub normalized_path: Vec<STPathElement>,
    pub is_default_path: bool,
    pub steps: Vec<StrandStep>,
}

impl Strand {
    pub fn new(normalized: &NormalizedPath) -> Self {
        Self {
            source: normalized.source,
            destination: normalized.destination,
            deliver: normalized.deliver,
            initial_asset: normalized.initial_asset,
            normalized_path: normalized.elements.clone(),
            is_default_path: normalized.is_default_path,
            steps: Vec::new(),
        }
    }

    pub fn push(&mut self, step: StrandStep) {
        self.steps.push(step);
    }
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

#[derive(Debug, Clone)]
pub(crate) struct ExplicitStep {
    pub from: AccountID,
    pub to: AccountID,
}

#[derive(Debug, Clone)]
pub(crate) struct ExplicitStrand {
    pub path_index: usize,
    pub path: STPath,
    pub steps: Vec<ExplicitStep>,
    pub kind: ExplicitStrandKind,
    pub src_account: AccountID,
    pub dst_account: AccountID,
    pub source_asset: Asset,
    pub issue: protocol::Issue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExplicitStrandKind {
    Direct,
    Book,
}

#[derive(Debug, Clone)]
pub(crate) struct ExplicitStrandEstimate {
    pub required_in: STAmount,
    pub actual_out: STAmount,
    pub hop_outputs: Vec<STAmount>,
    pub quality: Quality,
}

pub(crate) fn build_explicit_strand(
    path_index: usize,
    src_account: &AccountID,
    dst_account: &AccountID,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    path: &STPath,
) -> Option<ExplicitStrand> {
    if path.iter().any(|element| element.is_offer()) {
        return build_book_explicit_strand(
            path_index,
            src_account,
            dst_account,
            max_source_amount,
            dst_amount,
            path,
        );
    }

    if max_source_amount.native()
        || dst_amount.native()
        || max_source_amount.holds_mpt_issue()
        || dst_amount.holds_mpt_issue()
        || max_source_amount.asset() != dst_amount.asset()
    {
        return None;
    }

    let issue = dst_amount.issue();
    let expected_asset = PathAsset::from(issue.currency);
    let mut accounts = Vec::with_capacity(path.size() + 3);
    accounts.push(*src_account);

    for element in path.iter() {
        if element.has_mpt() || element.is_offer() {
            return None;
        }

        if element.has_asset() && element.path_asset() != expected_asset {
            return None;
        }

        if element.has_issuer() && element.issuer_id() != issue.account {
            return None;
        }

        if element.is_account() {
            push_account(&mut accounts, element.account_id());
        } else if element.has_issuer() {
            push_account(&mut accounts, element.issuer_id());
        }
    }

    if issue.account != *dst_account {
        push_account(&mut accounts, issue.account);
    }
    push_account(&mut accounts, *dst_account);

    if accounts.len() < 2 {
        return None;
    }

    let mut seen = BTreeSet::new();
    for account in accounts
        .iter()
        .copied()
        .skip(1)
        .take(accounts.len().saturating_sub(2))
    {
        if !seen.insert(account) {
            return None;
        }
    }

    let steps = accounts
        .windows(2)
        .map(|window| ExplicitStep {
            from: window[0],
            to: window[1],
        })
        .collect::<Vec<_>>();

    if steps.is_empty() {
        return None;
    }

    Some(ExplicitStrand {
        path_index,
        path: path.clone(),
        steps,
        kind: ExplicitStrandKind::Direct,
        src_account: *src_account,
        dst_account: *dst_account,
        source_asset: max_source_amount.asset(),
        issue,
    })
}

fn build_book_explicit_strand(
    path_index: usize,
    src_account: &AccountID,
    dst_account: &AccountID,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    path: &STPath,
) -> Option<ExplicitStrand> {
    if max_source_amount.holds_mpt_issue() || dst_amount.holds_mpt_issue() {
        return None;
    }

    if path.iter().filter(|element| element.is_offer()).count() != 1 {
        return None;
    }

    let source_asset = max_source_amount.asset();
    let deliver_asset = dst_amount.asset();
    if source_asset == deliver_asset {
        return None;
    }

    Some(ExplicitStrand {
        path_index,
        path: path.clone(),
        steps: Vec::new(),
        kind: ExplicitStrandKind::Book,
        src_account: *src_account,
        dst_account: *dst_account,
        source_asset,
        issue: dst_amount.issue(),
    })
}

pub(crate) fn estimate_direct_strand<V: ReadView>(
    view: &V,
    strand: &ExplicitStrand,
    requested_out: &STAmount,
) -> Result<Option<ExplicitStrandEstimate>, ViewError> {
    if strand.kind == ExplicitStrandKind::Book {
        let Some(estimate) = estimate_explicit_book_step(view, strand.source_asset, requested_out)?
        else {
            return Ok(None);
        };

        return Ok(Some(ExplicitStrandEstimate {
            required_in: estimate.actual_amount_in,
            actual_out: estimate.actual_amount_out.clone(),
            hop_outputs: vec![estimate.actual_amount_out],
            quality: estimate.quality,
        }));
    }

    if requested_out.signum() <= 0 {
        return Ok(None);
    }

    let mut hop_outputs = vec![requested_out.zeroed(); strand.steps.len()];
    let mut current_in = requested_out.clone();

    for (index, step) in strand.steps.iter().enumerate().rev() {
        hop_outputs[index] = current_in.clone();
        current_in = amount_with_transfer_rate(
            &current_in,
            transfer_rate(view, step.from, step.to, strand.issue.account)?,
        );
    }

    let quality = Quality::from_amounts(&Amounts::new(current_in.clone(), requested_out.clone()));
    Ok(Some(ExplicitStrandEstimate {
        required_in: current_in,
        actual_out: requested_out.clone(),
        hop_outputs,
        quality,
    }))
}

pub(crate) fn execute_direct_strand<V: ApplyView>(
    view: &mut V,
    strand: &ExplicitStrand,
    max_source_amount: &STAmount,
    dst_amount: &STAmount,
    partial_payment_allowed: bool,
) -> Result<Option<RippleCalcOutput>, ViewError> {
    if strand.kind == ExplicitStrandKind::Book {
        let Some(result) = execute_explicit_book_step(
            view,
            &strand.src_account,
            &strand.dst_account,
            max_source_amount,
            dst_amount,
            None,
        )?
        else {
            return Ok(None);
        };

        return Ok(Some(RippleCalcOutput {
            result: protocol::Ter::TES_SUCCESS,
            actual_amount_in: result.actual_amount_in,
            actual_amount_out: result.actual_amount_out,
        }));
    }

    let Some(full) = estimate_direct_strand(view, strand, dst_amount)? else {
        return Ok(None);
    };

    let (actual, actual_in) = if full.required_in <= *max_source_amount {
        let actual_in = full.required_in.clone();
        (full, actual_in)
    } else if partial_payment_allowed {
        let scaled_out = scaled_output(dst_amount, max_source_amount, &full.required_in);
        if scaled_out.signum() <= 0 {
            return Ok(None);
        }
        let Some(partial) = estimate_direct_strand(view, strand, &scaled_out)? else {
            return Ok(None);
        };
        if partial.required_in.signum() <= 0 || partial.actual_out.signum() <= 0 {
            return Ok(None);
        }
        (partial, max_source_amount.clone())
    } else {
        return Ok(None);
    };

    for (step, amount) in strand.steps.iter().zip(&actual.hop_outputs) {
        if amount.signum() <= 0 {
            return Ok(None);
        }
        apply_direct_iou_transfer(
            view,
            &step.from,
            &step.to,
            amount,
            &strand.issue.account,
            strand.issue.currency,
        )?;
    }

    Ok(Some(RippleCalcOutput {
        result: protocol::Ter::TES_SUCCESS,
        actual_amount_in: actual_in,
        actual_amount_out: actual.actual_out,
    }))
}

fn push_account(accounts: &mut Vec<AccountID>, account: AccountID) {
    if accounts.last().copied() != Some(account) {
        accounts.push(account);
    }
}

fn transfer_rate<V: ReadView>(
    view: &V,
    from: AccountID,
    to: AccountID,
    issuer: AccountID,
) -> Result<u32, ViewError> {
    if from == issuer || to == issuer {
        return Ok(QUALITY_ONE);
    }

    let issuer_keylet = protocol::account_keylet(account_to_uint160(&issuer));
    let Some(issuer_sle) = view.read(issuer_keylet)? else {
        return Ok(QUALITY_ONE);
    };

    let rate = issuer_sle.get_field_u32(sf("sfTransferRate"));
    Ok(if rate == 0 { QUALITY_ONE } else { rate })
}

fn amount_with_transfer_rate(amount: &STAmount, rate: u32) -> STAmount {
    if rate == QUALITY_ONE {
        amount.clone()
    } else {
        let rate_amount =
            STAmount::new_with_asset(sf("sfAmount"), amount.asset(), rate as u64, -9, false);
        amount.multiply(&rate_amount, amount.asset())
    }
}

fn scaled_output(
    requested_out: &STAmount,
    max_source: &STAmount,
    required_in: &STAmount,
) -> STAmount {
    let scale = max_source.divide(required_in, requested_out.asset());
    let scaled = requested_out.multiply(&scale, requested_out.asset());
    if scaled > *requested_out {
        requested_out.clone()
    } else {
        scaled
    }
}
