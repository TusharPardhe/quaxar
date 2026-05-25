//! Rust trait equivalents for the small concept checks in `Concepts.h`.

use crate::{Asset, Currency, IOUAmount, Issue, MPTAmount, MPTID, MPTIssue, XRPAmount};

pub trait StepAmount {}
impl StepAmount for XRPAmount {}
impl StepAmount for IOUAmount {}
impl StepAmount for MPTAmount {}

pub trait ValidIssueType {}
impl ValidIssueType for Issue {}
impl ValidIssueType for MPTIssue {}

pub trait AssetType: Into<Asset> {}
impl AssetType for Asset {}
impl AssetType for Issue {}
impl AssetType for MPTIssue {}
impl AssetType for MPTID {}

pub trait ValidPathAsset {}
impl ValidPathAsset for Currency {}
impl ValidPathAsset for MPTID {}

pub trait ValidTaker<TakerGets> {}
impl ValidTaker<IOUAmount> for IOUAmount {}
impl ValidTaker<XRPAmount> for IOUAmount {}
impl ValidTaker<MPTAmount> for IOUAmount {}
impl ValidTaker<IOUAmount> for XRPAmount {}
impl ValidTaker<MPTAmount> for XRPAmount {}
impl ValidTaker<IOUAmount> for MPTAmount {}
impl ValidTaker<XRPAmount> for MPTAmount {}
impl ValidTaker<MPTAmount> for MPTAmount {}
