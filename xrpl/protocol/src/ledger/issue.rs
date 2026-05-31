//! `Issue` / `Asset` helpers ported from `xrpl/protocol/Issue.*` and `Asset.*`.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use basics::number::{NumberArithmeticError, NumberParts as RuntimeNumber};

use crate::{
    AccountID, Currency, JsonValue, MPTID, MPTIssue, bad_currency, currency_from_string,
    currency_to_string, is_xrp_currency, mpt_issue_from_json, mpt_issue_to_string, no_account,
    no_currency, parse_base58_account_id, to_base58, xrp_account, xrp_currency,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BadAsset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetToken {
    Currency(Currency),
    MPTID(MPTID),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetAmountType {
    XRP,
    IOU,
    MPT,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Issue {
    pub currency: Currency,
    pub account: AccountID,
}

impl std::hash::Hash for Issue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.currency.hash(state);
        if !is_xrp_currency(self.currency) {
            self.account.hash(state);
        }
    }
}

impl Issue {
    pub fn new(currency: Currency, account: AccountID) -> Self {
        Self { currency, account }
    }

    pub fn issuer(&self) -> AccountID {
        self.account
    }

    pub fn text(&self) -> String {
        let mut text = currency_to_string(self.currency);

        if !is_xrp_currency(self.currency) {
            text.push('/');
            if self.account == xrp_account() {
                text.push('0');
            } else if self.account == no_account() {
                text.push('1');
            } else {
                text.push_str(&to_base58(self.account));
            }
        }

        text
    }

    pub fn set_json(&self, json: &mut BTreeMap<String, JsonValue>) {
        json.insert(
            "currency".to_string(),
            JsonValue::String(currency_to_string(self.currency)),
        );
        if !is_xrp_currency(self.currency) {
            json.insert(
                "issuer".to_string(),
                JsonValue::String(to_base58(self.account)),
            );
        }
    }

    pub fn native(&self) -> bool {
        *self == xrp_issue()
    }

    pub fn integral(&self) -> bool {
        self.native()
    }
}

impl PartialEq for Issue {
    fn eq(&self, other: &Self) -> bool {
        self.currency == other.currency
            && (is_xrp_currency(self.currency) || self.account == other.account)
    }
}

impl Eq for Issue {}

impl PartialOrd for Issue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Issue {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.currency.cmp(&other.currency) {
            Ordering::Equal if is_xrp_currency(self.currency) => Ordering::Equal,
            Ordering::Equal => self.account.cmp(&other.account),
            ordering => ordering,
        }
    }
}

pub fn is_consistent(issue: Issue) -> bool {
    is_xrp_currency(issue.currency) == issue.account.is_zero()
}

pub fn issue_to_string(issue: Issue) -> String {
    if issue.account.is_zero() {
        currency_to_string(issue.currency)
    } else {
        format!(
            "{}/{}",
            to_base58(issue.account),
            currency_to_string(issue.currency)
        )
    }
}

pub fn xrp_issue() -> Issue {
    Issue::new(xrp_currency(), xrp_account())
}

pub fn no_issue() -> Issue {
    Issue::new(no_currency(), no_account())
}

pub fn issue_from_json(value: &JsonValue) -> Result<Issue, String> {
    let JsonValue::Object(object) = value else {
        return Err("issueFromJson can only be specified with an 'object' Json value".into());
    };

    if object.contains_key("mpt_issuance_id") {
        return Err("issueFromJson, Issue should not have mpt_issuance_id".into());
    }

    let Some(JsonValue::String(currency_text)) = object.get("currency") else {
        return Err("issueFromJson currency must be a string Json value".into());
    };

    let currency = currency_from_string(currency_text);
    if currency == bad_currency() || currency == no_currency() {
        return Err("issueFromJson currency must be a valid currency".into());
    }

    if is_xrp_currency(currency) {
        if !matches!(object.get("issuer"), None | Some(JsonValue::Null)) {
            return Err("Issue, XRP should not have issuer".into());
        }
        return Ok(xrp_issue());
    }

    let Some(JsonValue::String(issuer_text)) = object.get("issuer") else {
        return Err("issueFromJson issuer must be a string Json value".into());
    };

    let Some(issuer) = parse_base58_account_id(issuer_text) else {
        return Err("issueFromJson issuer must be a valid account".into());
    };
    if issuer == no_account() || issuer == xrp_account() {
        return Err("issueFromJson issuer must be a valid account".into());
    }

    Ok(Issue::new(currency, issuer))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Asset {
    Issue(Issue),
    MPTIssue(MPTIssue),
}

impl Default for Asset {
    fn default() -> Self {
        Self::Issue(xrp_issue())
    }
}

impl Asset {
    pub fn get<T: AssetIssueAccess>(&self) -> &T {
        T::get_from(self).expect("Asset is not a requested issue")
    }

    pub fn get_mut<T: AssetIssueAccess>(&mut self) -> &mut T {
        T::get_from_mut(self).expect("Asset is not a requested issue")
    }

    pub fn holds<T: AssetIssueAccess>(&self) -> bool {
        T::get_from(self).is_some()
    }

    pub fn issuer(&self) -> AccountID {
        match self {
            Self::Issue(issue) => issue.issuer(),
            Self::MPTIssue(issue) => issue.issuer(),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Self::Issue(issue) => issue.text(),
            Self::MPTIssue(issue) => issue.text(),
        }
    }

    pub fn value(&self) -> Self {
        *self
    }

    pub fn token(&self) -> AssetToken {
        match self {
            Self::Issue(issue) => AssetToken::Currency(issue.currency),
            Self::MPTIssue(issue) => AssetToken::MPTID(issue.mpt_id()),
        }
    }

    pub fn set_json(&self, json: &mut BTreeMap<String, JsonValue>) {
        match self {
            Self::Issue(issue) => issue.set_json(json),
            Self::MPTIssue(issue) => issue.set_json(json),
        }
    }

    pub fn amount(&self, number: RuntimeNumber) -> Result<crate::STAmount, NumberArithmeticError> {
        match *self {
            Self::Issue(issue) if issue.native() => Ok(crate::STAmount::from_xrp_amount(
                crate::XRPAmount::from_number(number)?,
            )),
            Self::Issue(issue) => Ok(crate::STAmount::from_iou_amount(
                crate::sf_generic(),
                crate::IOUAmount::from_number(number)?,
                issue,
            )),
            Self::MPTIssue(issue) => Ok(crate::STAmount::from_mpt_amount(
                crate::sf_generic(),
                crate::MPTAmount::from_number(number)?,
                issue,
            )),
        }
    }

    pub fn get_amount_type(&self) -> AssetAmountType {
        match self {
            Self::Issue(issue) if issue.native() => AssetAmountType::XRP,
            Self::Issue(_) => AssetAmountType::IOU,
            Self::MPTIssue(_) => AssetAmountType::MPT,
        }
    }

    pub fn visit<R>(
        &self,
        on_issue: impl FnOnce(&Issue) -> R,
        on_mpt_issue: impl FnOnce(&MPTIssue) -> R,
    ) -> R {
        match self {
            Self::Issue(issue) => on_issue(issue),
            Self::MPTIssue(issue) => on_mpt_issue(issue),
        }
    }

    pub fn native(&self) -> bool {
        matches!(self, Self::Issue(issue) if issue.native())
    }

    pub fn integral(&self) -> bool {
        match self {
            Self::Issue(issue) => issue.integral(),
            Self::MPTIssue(_) => true,
        }
    }
}

impl From<Issue> for Asset {
    fn from(value: Issue) -> Self {
        Self::Issue(value)
    }
}

impl From<MPTIssue> for Asset {
    fn from(value: MPTIssue) -> Self {
        Self::MPTIssue(value)
    }
}

impl From<MPTID> for Asset {
    fn from(value: MPTID) -> Self {
        Self::MPTIssue(MPTIssue::new(value))
    }
}

impl PartialOrd for Asset {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Asset {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Issue(left), Self::Issue(right)) => left.cmp(right),
            (Self::MPTIssue(left), Self::MPTIssue(right)) => left.cmp(right),
            (Self::Issue(_), Self::MPTIssue(_)) => Ordering::Greater,
            (Self::MPTIssue(_), Self::Issue(_)) => Ordering::Less,
        }
    }
}

impl PartialEq<BadAsset> for Asset {
    fn eq(&self, _other: &BadAsset) -> bool {
        is_bad_asset(*self)
    }
}

impl PartialEq<Asset> for BadAsset {
    fn eq(&self, other: &Asset) -> bool {
        is_bad_asset(*other)
    }
}

impl PartialEq<Currency> for Asset {
    fn eq(&self, other: &Currency) -> bool {
        matches!(self, Asset::Issue(issue) if issue.currency == *other)
    }
}

impl PartialEq<Asset> for Currency {
    fn eq(&self, other: &Asset) -> bool {
        other == self
    }
}

pub trait AssetIssueAccess: crate::ValidIssueType {
    fn get_from(asset: &Asset) -> Option<&Self>;
    fn get_from_mut(asset: &mut Asset) -> Option<&mut Self>;
}

impl AssetIssueAccess for Issue {
    fn get_from(asset: &Asset) -> Option<&Self> {
        match asset {
            Asset::Issue(issue) => Some(issue),
            Asset::MPTIssue(_) => None,
        }
    }

    fn get_from_mut(asset: &mut Asset) -> Option<&mut Self> {
        match asset {
            Asset::Issue(issue) => Some(issue),
            Asset::MPTIssue(_) => None,
        }
    }
}

impl AssetIssueAccess for MPTIssue {
    fn get_from(asset: &Asset) -> Option<&Self> {
        match asset {
            Asset::Issue(_) => None,
            Asset::MPTIssue(issue) => Some(issue),
        }
    }

    fn get_from_mut(asset: &mut Asset) -> Option<&mut Self> {
        match asset {
            Asset::Issue(_) => None,
            Asset::MPTIssue(issue) => Some(issue),
        }
    }
}

pub fn equal_tokens(left: Asset, right: Asset) -> bool {
    match (left, right) {
        (Asset::Issue(left), Asset::Issue(right)) => left.currency == right.currency,
        (Asset::MPTIssue(left), Asset::MPTIssue(right)) => left.mpt_id() == right.mpt_id(),
        _ => false,
    }
}

pub fn is_xrp_asset(asset: Asset) -> bool {
    asset.native()
}

pub fn bad_asset() -> BadAsset {
    BadAsset
}

pub fn is_bad_asset(asset: Asset) -> bool {
    match asset {
        Asset::Issue(issue) => issue.currency == bad_currency(),
        Asset::MPTIssue(issue) => issue.issuer().is_zero(),
    }
}

pub fn asset_to_string(asset: Asset) -> String {
    match asset {
        Asset::Issue(issue) => issue_to_string(issue),
        Asset::MPTIssue(issue) => mpt_issue_to_string(issue),
    }
}

pub fn valid_json_asset(value: &JsonValue) -> bool {
    let JsonValue::Object(object) = value else {
        return false;
    };
    if object.contains_key("mpt_issuance_id") {
        !(object.contains_key("currency") || object.contains_key("issuer"))
    } else {
        object.contains_key("currency")
    }
}

pub fn asset_from_json(value: &JsonValue) -> Result<Asset, String> {
    let JsonValue::Object(object) = value else {
        return Err("assetFromJson must contain currency or mpt_issuance_id".into());
    };

    if object.contains_key("currency") {
        return issue_from_json(value).map(Asset::from);
    }
    if object.contains_key("mpt_issuance_id") {
        return mpt_issue_from_json(value).map(Asset::from);
    }

    Err("assetFromJson must contain currency or mpt_issuance_id".into())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{JsonValue, no_account, to_base58, xrp_account};

    use super::issue_from_json;

    fn issue_json(issuer: String) -> JsonValue {
        JsonValue::Object(BTreeMap::from([
            ("currency".to_owned(), JsonValue::String("USD".to_owned())),
            ("issuer".to_owned(), JsonValue::String(issuer)),
        ]))
    }

    #[test]
    fn issue_from_json_rejects_special_non_xrp_issuers() {
        assert!(issue_from_json(&issue_json(to_base58(no_account()))).is_err());
        assert!(issue_from_json(&issue_json(to_base58(xrp_account()))).is_err());
    }
}
