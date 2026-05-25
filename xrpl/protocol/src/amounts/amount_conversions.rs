//! `AmountConversions.h` parity helpers.

use basics::number::{
    NumberArithmeticError, NumberParts as RuntimeNumber, NumberRoundModeGuard, RoundingMode,
};

use crate::{
    Asset, IOUAmount, Issue, MPTAmount, MPTID, MPTIssue, ST_AMOUNT_MAX_MANTISSA,
    ST_AMOUNT_MAX_NATIVE_NETWORK, ST_AMOUNT_MAX_OFFSET, STAmount, XRPAmount, no_issue, sf_generic,
    xrp_issue,
};

pub trait ToStAmountSource {
    fn to_st_amount_with_asset(self, asset: Option<Asset>) -> STAmount;
}

impl ToStAmountSource for IOUAmount {
    fn to_st_amount_with_asset(self, asset: Option<Asset>) -> STAmount {
        let asset = asset.unwrap_or_else(|| Asset::Issue(no_issue()));
        let issue = match asset {
            Asset::Issue(issue) => issue,
            Asset::MPTIssue(_) => panic!("xrpl::toSTAmount : is Issue"),
        };
        STAmount::from_iou_amount(sf_generic(), self, issue)
    }
}

impl ToStAmountSource for XRPAmount {
    fn to_st_amount_with_asset(self, asset: Option<Asset>) -> STAmount {
        if let Some(asset) = asset {
            assert!(asset.native(), "xrpl::toSTAmount : is XRP");
        }
        STAmount::from_xrp_amount(self)
    }
}

impl ToStAmountSource for MPTAmount {
    fn to_st_amount_with_asset(self, asset: Option<Asset>) -> STAmount {
        let asset = asset.unwrap_or_else(|| Asset::MPTIssue(MPTIssue::new(MPTID::zero())));
        let issue = match asset {
            Asset::Issue(_) => panic!("xrpl::toSTAmount : is MPT"),
            Asset::MPTIssue(issue) => issue,
        };
        STAmount::from_mpt_amount(sf_generic(), self, issue)
    }
}

pub fn to_st_amount<T>(amount: T) -> STAmount
where
    T: ToStAmountSource,
{
    amount.to_st_amount_with_asset(None)
}

pub fn to_st_amount_with_asset<T>(amount: T, asset: Asset) -> STAmount
where
    T: ToStAmountSource,
{
    amount.to_st_amount_with_asset(Some(asset))
}

pub trait FromAmountSource<S>: Sized {
    fn from_amount_source(source: S) -> Self;
}

impl FromAmountSource<&STAmount> for STAmount {
    fn from_amount_source(source: &STAmount) -> Self {
        source.clone()
    }
}

impl FromAmountSource<&STAmount> for IOUAmount {
    fn from_amount_source(source: &STAmount) -> Self {
        source.iou()
    }
}

impl FromAmountSource<&STAmount> for XRPAmount {
    fn from_amount_source(source: &STAmount) -> Self {
        source.xrp()
    }
}

impl FromAmountSource<&STAmount> for MPTAmount {
    fn from_amount_source(source: &STAmount) -> Self {
        source.mpt()
    }
}

impl FromAmountSource<IOUAmount> for IOUAmount {
    fn from_amount_source(source: IOUAmount) -> Self {
        source
    }
}

impl FromAmountSource<XRPAmount> for XRPAmount {
    fn from_amount_source(source: XRPAmount) -> Self {
        source
    }
}

impl FromAmountSource<MPTAmount> for MPTAmount {
    fn from_amount_source(source: MPTAmount) -> Self {
        source
    }
}

pub fn to_amount<T, S>(amount: S) -> T
where
    T: FromAmountSource<S>,
{
    T::from_amount_source(amount)
}

pub trait FromNumberAmount: Sized {
    fn from_number_amount(
        asset: Asset,
        number: RuntimeNumber,
    ) -> Result<Self, NumberArithmeticError>;
}

impl FromNumberAmount for IOUAmount {
    fn from_number_amount(
        _asset: Asset,
        number: RuntimeNumber,
    ) -> Result<Self, NumberArithmeticError> {
        IOUAmount::from_number(number)
    }
}

impl FromNumberAmount for XRPAmount {
    fn from_number_amount(
        _asset: Asset,
        number: RuntimeNumber,
    ) -> Result<Self, NumberArithmeticError> {
        XRPAmount::from_number(number)
    }
}

impl FromNumberAmount for MPTAmount {
    fn from_number_amount(
        _asset: Asset,
        number: RuntimeNumber,
    ) -> Result<Self, NumberArithmeticError> {
        MPTAmount::from_number(number)
    }
}

impl FromNumberAmount for STAmount {
    fn from_number_amount(
        asset: Asset,
        number: RuntimeNumber,
    ) -> Result<Self, NumberArithmeticError> {
        match asset {
            Asset::Issue(issue) if issue.native() => {
                Ok(STAmount::from_xrp_amount(XRPAmount::from_number(number)?))
            }
            Asset::Issue(issue) => Ok(STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_number(number)?,
                issue,
            )),
            Asset::MPTIssue(issue) => Ok(STAmount::from_mpt_amount(
                sf_generic(),
                MPTAmount::from_number(number)?,
                issue,
            )),
        }
    }
}

pub fn to_amount_from_number<T>(
    asset: Asset,
    number: RuntimeNumber,
    mode: RoundingMode,
) -> Result<T, NumberArithmeticError>
where
    T: FromNumberAmount,
{
    let _guard = asset.native().then(|| NumberRoundModeGuard::new(mode));
    T::from_number_amount(asset, number)
}

pub trait MaxAmountForAsset: Sized {
    fn max_amount_for_asset(asset: Asset) -> Self;
}

impl MaxAmountForAsset for IOUAmount {
    fn max_amount_for_asset(_asset: Asset) -> Self {
        IOUAmount::from_parts(ST_AMOUNT_MAX_MANTISSA as i64, ST_AMOUNT_MAX_OFFSET)
            .expect("max IOU amount should stay canonical")
    }
}

impl MaxAmountForAsset for XRPAmount {
    fn max_amount_for_asset(_asset: Asset) -> Self {
        XRPAmount::from_drops(ST_AMOUNT_MAX_NATIVE_NETWORK as i64)
    }
}

impl MaxAmountForAsset for MPTAmount {
    fn max_amount_for_asset(_asset: Asset) -> Self {
        MPTAmount::from_value(crate::MAX_MP_TOKEN_AMOUNT)
    }
}

impl MaxAmountForAsset for STAmount {
    fn max_amount_for_asset(asset: Asset) -> Self {
        match asset {
            Asset::Issue(issue) if issue.native() => STAmount::from_xrp_amount(
                XRPAmount::from_drops(ST_AMOUNT_MAX_NATIVE_NETWORK as i64),
            ),
            Asset::Issue(issue) => STAmount::new_with_asset(
                sf_generic(),
                issue,
                ST_AMOUNT_MAX_MANTISSA,
                ST_AMOUNT_MAX_OFFSET,
                false,
            ),
            Asset::MPTIssue(issue) => STAmount::from_mpt_amount(
                sf_generic(),
                MPTAmount::from_value(crate::MAX_MP_TOKEN_AMOUNT),
                issue,
            ),
        }
    }
}

pub fn to_max_amount<T>(asset: Asset) -> T
where
    T: MaxAmountForAsset,
{
    T::max_amount_for_asset(asset)
}

pub trait AmountAsset {
    fn asset(&self) -> Asset;
}

impl AmountAsset for IOUAmount {
    fn asset(&self) -> Asset {
        Asset::Issue(no_issue())
    }
}

impl AmountAsset for XRPAmount {
    fn asset(&self) -> Asset {
        Asset::Issue(xrp_issue())
    }
}

impl AmountAsset for MPTAmount {
    fn asset(&self) -> Asset {
        Asset::MPTIssue(MPTIssue::new(MPTID::zero()))
    }
}

impl AmountAsset for STAmount {
    fn asset(&self) -> Asset {
        self.asset()
    }
}

pub fn get_asset<T>(amount: &T) -> Asset
where
    T: AmountAsset,
{
    amount.asset()
}

pub fn get<T>(amount: &STAmount) -> T
where
    T: for<'a> FromAmountSource<&'a STAmount>,
{
    to_amount(amount)
}

pub fn issue_from_asset(asset: Asset) -> Option<Issue> {
    match asset {
        Asset::Issue(issue) => Some(issue),
        Asset::MPTIssue(_) => None,
    }
}
