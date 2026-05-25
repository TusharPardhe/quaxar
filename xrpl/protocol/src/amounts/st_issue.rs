//! `STIssue` port from `xrpl/protocol/STIssue.*`.

use crate::{
    AccountID, Asset, Currency, Issue, JsonOptions, JsonValue, MPTIssue, SField, SerialIter,
    SerializedTypeId, Serializer, StBase, StBaseCore, asset_from_json, downcast_stbase_ref,
    is_consistent, is_xrp_currency, no_account, xrp_issue,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct STIssue {
    core: StBaseCore,
    asset: Asset,
}

impl STIssue {
    pub fn with_field(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            asset: Asset::Issue(xrp_issue()),
        }
    }

    pub fn new_with_asset(field: &'static SField, asset: impl Into<Asset>) -> Self {
        let asset = asset.into();
        if let Asset::Issue(issue) = asset {
            assert!(
                is_consistent(issue),
                "Invalid asset: currency and account native mismatch"
            );
        }

        Self {
            core: StBaseCore::with_field(field),
            asset,
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        let currency_or_account =
            Currency::from_slice(sit.get160().data()).expect("currency width should match");

        if is_xrp_currency(currency_or_account) {
            return Self::new_with_asset(field, xrp_issue());
        }

        let account =
            AccountID::from_slice(sit.get160().data()).expect("account width should match");
        if account == no_account() {
            let sequence = sit.get32();
            let mut bytes = [0u8; crate::MPTID::BYTES];
            bytes[..4].copy_from_slice(&sequence.to_be_bytes());
            bytes[4..].copy_from_slice(currency_or_account.data());
            return Self::new_with_asset(field, MPTIssue::new(crate::MPTID::from_array(bytes)));
        }

        let issue = Issue::new(currency_or_account, account);
        assert!(
            is_consistent(issue),
            "invalid issue: currency and account native mismatch"
        );
        Self::new_with_asset(field, issue)
    }

    pub fn asset(&self) -> Asset {
        self.asset
    }
}

impl StBase for STIssue {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        &mut self.core
    }

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::Issue
    }

    fn text(&self) -> String {
        self.asset.text()
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        let mut object = std::collections::BTreeMap::new();
        self.asset.set_json(&mut object);
        JsonValue::Object(object)
    }

    fn add(&self, serializer: &mut Serializer) {
        match self.asset {
            Asset::Issue(issue) => {
                serializer.add_bit_string(issue.currency);
                if !is_xrp_currency(issue.currency) {
                    serializer.add_bit_string(issue.account);
                }
            }
            Asset::MPTIssue(issue) => {
                serializer.add_bit_string(issue.issuer());
                serializer.add_bit_string(no_account());
                let sequence = u32::from_be_bytes(
                    issue.mpt_id().data()[..4]
                        .try_into()
                        .expect("MPT sequence width should match"),
                );
                serializer.add32(sequence);
            }
        }
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).asset == self.asset
    }

    fn is_default(&self) -> bool {
        matches!(self.asset, Asset::Issue(issue) if issue == xrp_issue())
    }
}

pub fn st_issue_from_json(field: &'static SField, value: &JsonValue) -> Result<STIssue, String> {
    asset_from_json(value).map(|asset| STIssue::new_with_asset(field, asset))
}
