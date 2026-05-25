//! Shared step traits and value types for RippleCalc explicit-path execution.
// Kept for compatibility with the broader explicit-step/pathfinding surfaces in
// Steps.h / the reference source / the reference source; the current Rust owner does not
// wire this seam into the narrowed runtime yet.
#![allow(dead_code)]

use std::collections::{BTreeSet, HashSet};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, Asset, Book, IOUAmount, MPTAmount, Quality, QualityFunction,
    QualityFunctionClobLikeTag, Ter, XRPAmount,
};

use crate::views::apply_view::ApplyView;
use crate::views::read_view::{ReadView, ViewError};

pub(crate) const QUALITY_ONE: u32 = protocol::QUALITY_ONE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebtDirection {
    Issues,
    Redeems,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrandDirection {
    Forward,
    Reverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferCrossing {
    No,
    Yes,
    Sell,
}

#[must_use]
pub const fn redeems(direction: DebtDirection) -> bool {
    matches!(direction, DebtDirection::Redeems)
}

#[must_use]
pub const fn issues(direction: DebtDirection) -> bool {
    matches!(direction, DebtDirection::Issues)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EitherAmount {
    Xrp(XRPAmount),
    Iou(IOUAmount),
    Mpt(MPTAmount),
}

impl EitherAmount {
    #[must_use]
    pub fn kind_name(self) -> &'static str {
        match self {
            Self::Xrp(_) => "XRP",
            Self::Iou(_) => "IOU",
            Self::Mpt(_) => "MPT",
        }
    }

    pub fn expect_iou(self) -> Result<IOUAmount, StepError> {
        match self {
            Self::Iou(amount) => Ok(amount),
            other => Err(StepError::InvalidAmountType {
                expected: "IOU",
                actual: other.kind_name(),
            }),
        }
    }

    #[must_use]
    pub fn is_zero(self) -> bool {
        match self {
            Self::Xrp(amount) => amount == XRPAmount::new(),
            Self::Iou(amount) => amount == IOUAmount::new(),
            Self::Mpt(amount) => amount == MPTAmount::new(),
        }
    }
}

impl From<XRPAmount> for EitherAmount {
    fn from(value: XRPAmount) -> Self {
        Self::Xrp(value)
    }
}

impl From<IOUAmount> for EitherAmount {
    fn from(value: IOUAmount) -> Self {
        Self::Iou(value)
    }
}

impl From<MPTAmount> for EitherAmount {
    fn from(value: MPTAmount) -> Self {
        Self::Mpt(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFlow {
    pub input: EitherAmount,
    pub output: EitherAmount,
}

impl StepFlow {
    #[must_use]
    pub const fn new(input: EitherAmount, output: EitherAmount) -> Self {
        Self { input, output }
    }
}

#[derive(Debug)]
pub enum StepError {
    Ter(Ter),
    View(ViewError),
    InvalidAmountType {
        expected: &'static str,
        actual: &'static str,
    },
    MissingPreviousStep(&'static str),
    Invariant(&'static str),
    Conversion(String),
}

impl From<ViewError> for StepError {
    fn from(value: ViewError) -> Self {
        Self::View(value)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StepExecutionContext<'a> {
    pub prev_step: Option<&'a dyn Step>,
}

impl<'a> StepExecutionContext<'a> {
    #[must_use]
    pub const fn new(prev_step: Option<&'a dyn Step>) -> Self {
        Self { prev_step }
    }
}

#[derive(Debug)]
pub struct StrandContext<'a> {
    pub view: &'a dyn ReadView,
    pub strand_src: AccountID,
    pub strand_dst: AccountID,
    pub strand_deliver: Asset,
    pub limit_quality: Option<Quality>,
    pub is_first: bool,
    pub is_last: bool,
    pub owner_pays_transfer_fee: bool,
    pub offer_crossing: OfferCrossing,
    pub is_default_path: bool,
    pub strand_size: usize,
    pub prev_step: Option<&'a dyn Step>,
    pub seen_direct_assets: &'a mut [HashSet<Asset>; 2],
    pub seen_book_outs: &'a mut HashSet<Asset>,
    pub domain_id: Option<Uint256>,
}

pub(crate) type Strand = Vec<Box<dyn Step>>;

pub trait Step: std::fmt::Debug {
    fn rev(
        &mut self,
        exec: &StepExecutionContext<'_>,
        sandbox: &mut dyn ApplyView,
        af_view: &mut dyn ApplyView,
        offers_to_remove: &mut BTreeSet<Uint256>,
        out: EitherAmount,
    ) -> Result<StepFlow, StepError>;

    fn fwd(
        &mut self,
        exec: &StepExecutionContext<'_>,
        sandbox: &mut dyn ApplyView,
        af_view: &mut dyn ApplyView,
        offers_to_remove: &mut BTreeSet<Uint256>,
        input: EitherAmount,
    ) -> Result<StepFlow, StepError>;

    fn cached_in(&self) -> Option<EitherAmount>;

    fn cached_out(&self) -> Option<EitherAmount>;

    fn direct_step_src_acct(&self) -> Option<AccountID> {
        None
    }

    fn direct_step_accounts(&self) -> Option<(AccountID, AccountID)> {
        None
    }

    fn debt_direction(
        &self,
        exec: &StepExecutionContext<'_>,
        view: &dyn ReadView,
        direction: StrandDirection,
    ) -> Result<DebtDirection, StepError>;

    fn line_quality_in(&self, _view: &dyn ReadView) -> u32 {
        QUALITY_ONE
    }

    fn quality_upper_bound(
        &self,
        exec: &StepExecutionContext<'_>,
        view: &dyn ReadView,
        prev_step_dir: DebtDirection,
    ) -> Result<(Option<Quality>, DebtDirection), StepError>;

    fn get_quality_func(
        &self,
        exec: &StepExecutionContext<'_>,
        view: &dyn ReadView,
        prev_step_dir: DebtDirection,
    ) -> Result<(Option<QualityFunction>, DebtDirection), StepError> {
        let (quality, direction) = self.quality_upper_bound(exec, view, prev_step_dir)?;
        Ok((
            quality
                .map(|quality| QualityFunction::from_quality(quality, QualityFunctionClobLikeTag)),
            direction,
        ))
    }

    fn offers_used(&self) -> u32 {
        0
    }

    fn book_step_book(&self) -> Option<Book> {
        None
    }

    fn is_zero(&self, amount: EitherAmount) -> bool;

    fn inactive(&self) -> bool {
        false
    }

    fn equal_out(&self, lhs: EitherAmount, rhs: EitherAmount) -> bool;

    fn equal_in(&self, lhs: EitherAmount, rhs: EitherAmount) -> bool;

    fn valid_fwd(
        &mut self,
        exec: &StepExecutionContext<'_>,
        sandbox: &mut dyn ApplyView,
        af_view: &mut dyn ApplyView,
        input: EitherAmount,
    ) -> Result<(bool, EitherAmount), StepError>;
}

#[must_use]
pub fn offers_used(strand: &[Box<dyn Step>]) -> u32 {
    strand.iter().map(|step| step.offers_used()).sum()
}

#[must_use]
pub fn check_near_iou(expected: IOUAmount, actual: IOUAmount) -> bool {
    const RATIO_TOLERANCE: f64 = 0.001;

    if (expected.exponent() - actual.exponent()).abs() > 1 {
        return false;
    }

    if actual.exponent() < -20 {
        return true;
    }

    let lhs = if expected.exponent() < actual.exponent() {
        expected.mantissa() / 10
    } else {
        expected.mantissa()
    };
    let rhs = if actual.exponent() < expected.exponent() {
        actual.mantissa() / 10
    } else {
        actual.mantissa()
    };

    if lhs == rhs {
        return true;
    }

    let diff = (lhs - rhs).abs() as f64;
    let scale = lhs.abs().max(rhs.abs()) as f64;
    diff / scale <= RATIO_TOLERANCE
}
