//! `STPathSet` port from `xrpl/protocol/STPathSet.*`.

use std::collections::BTreeMap;

use crate::{
    AccountID, JsonOptions, JsonValue, MPTID, PathAsset, SField, SerialIter, SerializedTypeId,
    Serializer, StBase, StBaseCore, currency_to_string, parse_base58_account_id, to_base58,
    to_currency,
};

#[derive(Debug, Clone)]
pub struct STPathElement {
    node_type: u8,
    account_id: AccountID,
    asset_id: PathAsset,
    issuer_id: AccountID,
    is_offer: bool,
    hash_value: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct STPath {
    path: Vec<STPathElement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct STPathSet {
    core: StBaseCore,
    value: Vec<STPath>,
}

impl STPathElement {
    pub const TYPE_NONE: u8 = 0x00;
    pub const TYPE_ACCOUNT: u8 = 0x01;
    pub const TYPE_CURRENCY: u8 = 0x10;
    pub const TYPE_ISSUER: u8 = 0x20;
    pub const TYPE_MPT: u8 = 0x40;
    pub const TYPE_BOUNDARY: u8 = 0xFF;
    pub const TYPE_ASSET: u8 = Self::TYPE_CURRENCY | Self::TYPE_MPT;
    pub const TYPE_ALL: u8 =
        Self::TYPE_ACCOUNT | Self::TYPE_CURRENCY | Self::TYPE_ISSUER | Self::TYPE_MPT;

    pub fn new() -> Self {
        let mut element = Self {
            node_type: Self::TYPE_NONE,
            account_id: AccountID::zero(),
            asset_id: PathAsset::default(),
            issuer_id: AccountID::zero(),
            is_offer: true,
            hash_value: 0,
        };
        element.hash_value = Self::get_hash(&element);
        element
    }

    pub fn from_optionals(
        account: Option<AccountID>,
        asset: Option<PathAsset>,
        issuer: Option<AccountID>,
    ) -> Self {
        let mut element = Self {
            node_type: Self::TYPE_NONE,
            account_id: AccountID::zero(),
            asset_id: PathAsset::default(),
            issuer_id: AccountID::zero(),
            is_offer: account.is_none(),
            hash_value: 0,
        };

        if let Some(account) = account {
            element.account_id = account;
            element.node_type |= Self::TYPE_ACCOUNT;
            assert_ne!(
                element.account_id,
                crate::no_account(),
                "xrpl::STPathElement::STPathElement : account is set"
            );
        }

        if let Some(asset) = asset {
            element.asset_id = asset;
            element.node_type |= if element.asset_id.holds_currency() {
                Self::TYPE_CURRENCY
            } else {
                Self::TYPE_MPT
            };
        }

        if let Some(issuer) = issuer {
            element.issuer_id = issuer;
            element.node_type |= Self::TYPE_ISSUER;
            assert_ne!(
                element.issuer_id,
                crate::no_account(),
                "xrpl::STPathElement::STPathElement : issuer is set"
            );
        }

        element.hash_value = Self::get_hash(&element);
        element
    }

    pub fn inferred(
        account: AccountID,
        asset: impl Into<PathAsset>,
        issuer: AccountID,
        force_asset: bool,
    ) -> Self {
        let asset = asset.into();
        let is_offer = account.is_zero();
        let mut node_type = Self::TYPE_NONE;

        if !is_offer {
            node_type |= Self::TYPE_ACCOUNT;
        }
        if force_asset || !asset.is_xrp() {
            node_type |= if asset.holds_currency() {
                Self::TYPE_CURRENCY
            } else {
                Self::TYPE_MPT
            };
        }
        if !issuer.is_zero() {
            node_type |= Self::TYPE_ISSUER;
        }

        let mut element = Self {
            node_type,
            account_id: account,
            asset_id: asset,
            issuer_id: issuer,
            is_offer,
            hash_value: 0,
        };
        element.hash_value = Self::get_hash(&element);
        element
    }

    pub fn raw(
        node_type: u8,
        account: AccountID,
        asset: impl Into<PathAsset>,
        issuer: AccountID,
    ) -> Self {
        let asset = asset.into();
        let mut element = Self {
            node_type,
            account_id: account,
            asset_id: asset,
            issuer_id: issuer,
            is_offer: account.is_zero(),
            hash_value: 0,
        };
        element.node_type &= if element.asset_id.holds_currency() {
            !Self::TYPE_MPT
        } else {
            !Self::TYPE_CURRENCY
        };
        element.hash_value = Self::get_hash(&element);
        element
    }

    pub fn node_type(&self) -> u8 {
        self.node_type
    }

    pub fn is_offer(&self) -> bool {
        self.is_offer
    }

    pub fn is_account(&self) -> bool {
        !self.is_offer()
    }

    pub fn has_issuer(&self) -> bool {
        (self.node_type() & Self::TYPE_ISSUER) != 0
    }

    pub fn has_currency(&self) -> bool {
        (self.node_type() & Self::TYPE_CURRENCY) != 0
    }

    pub fn has_mpt(&self) -> bool {
        (self.node_type() & Self::TYPE_MPT) != 0
    }

    pub fn has_asset(&self) -> bool {
        (self.node_type() & Self::TYPE_ASSET) != 0
    }

    pub fn is_none(&self) -> bool {
        self.node_type() == Self::TYPE_NONE
    }

    pub fn account_id(&self) -> AccountID {
        self.account_id
    }

    pub fn path_asset(&self) -> PathAsset {
        self.asset_id
    }

    pub fn currency(&self) -> crate::Currency {
        self.asset_id.currency()
    }

    pub fn mpt_id(&self) -> MPTID {
        self.asset_id.mpt_id()
    }

    pub fn issuer_id(&self) -> AccountID {
        self.issuer_id
    }

    fn get_hash(element: &Self) -> usize {
        let mut hash_account = 2_654_435_761usize;
        let mut hash_asset = 2_654_435_761usize;
        let mut hash_issuer = 2_654_435_761usize;

        for byte in element.account_id.data() {
            hash_account =
                hash_account.wrapping_add(hash_account.wrapping_mul(257) ^ usize::from(*byte));
        }

        match element.asset_id {
            PathAsset::Currency(currency) => {
                for byte in currency.data() {
                    hash_asset =
                        hash_asset.wrapping_add(hash_asset.wrapping_mul(509) ^ usize::from(*byte));
                }
            }
            PathAsset::MPTID(mpt_id) => {
                for byte in mpt_id.data() {
                    hash_asset = hash_asset.wrapping_add(usize::from(*byte));
                }
            }
        }

        for byte in element.issuer_id.data() {
            hash_issuer =
                hash_issuer.wrapping_add(hash_issuer.wrapping_mul(911) ^ usize::from(*byte));
        }

        hash_account ^ hash_asset ^ hash_issuer
    }
}

impl Default for STPathElement {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for STPathElement {
    fn eq(&self, other: &Self) -> bool {
        (self.node_type & Self::TYPE_ACCOUNT) == (other.node_type & Self::TYPE_ACCOUNT)
            && self.hash_value == other.hash_value
            && self.account_id == other.account_id
            && self.asset_id == other.asset_id
            && self.issuer_id == other.issuer_id
    }
}

impl Eq for STPathElement {}

impl STPath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_vec(path: Vec<STPathElement>) -> Self {
        Self { path }
    }

    pub fn size(&self) -> usize {
        self.path.len()
    }

    pub fn empty(&self) -> bool {
        self.path.is_empty()
    }

    pub fn push_back(&mut self, element: STPathElement) {
        self.path.push(element);
    }

    pub fn has_seen(&self, account: AccountID, asset: PathAsset, issuer: AccountID) -> bool {
        self.path.iter().any(|element| {
            element.account_id() == account
                && element.path_asset() == asset
                && element.issuer_id() == issuer
        })
    }

    pub fn json(&self, _options: JsonOptions) -> JsonValue {
        JsonValue::Array(
            self.path
                .iter()
                .map(|element| {
                    let mut entry = BTreeMap::new();
                    let node_type = element.node_type();
                    entry.insert(
                        "type".to_string(),
                        JsonValue::Unsigned(u64::from(node_type)),
                    );

                    if (node_type & STPathElement::TYPE_ACCOUNT) != 0 {
                        entry.insert(
                            "account".to_string(),
                            JsonValue::String(to_base58(element.account_id())),
                        );
                    }
                    if (node_type & STPathElement::TYPE_CURRENCY) != 0 {
                        entry.insert(
                            "currency".to_string(),
                            JsonValue::String(currency_to_string(element.currency())),
                        );
                    }
                    if (node_type & STPathElement::TYPE_MPT) != 0 {
                        entry.insert(
                            "mpt_issuance_id".to_string(),
                            JsonValue::String(element.mpt_id().to_string()),
                        );
                    }
                    if (node_type & STPathElement::TYPE_ISSUER) != 0 {
                        entry.insert(
                            "issuer".to_string(),
                            JsonValue::String(to_base58(element.issuer_id())),
                        );
                    }

                    JsonValue::Object(entry)
                })
                .collect(),
        )
    }

    pub fn iter(&self) -> impl Iterator<Item = &STPathElement> {
        self.path.iter()
    }

    pub fn back(&self) -> Option<&STPathElement> {
        self.path.last()
    }

    pub fn front(&self) -> Option<&STPathElement> {
        self.path.first()
    }

    pub fn reserve(&mut self, size: usize) {
        self.path.reserve(size);
    }
}

impl std::ops::Index<usize> for STPath {
    type Output = STPathElement;

    fn index(&self, index: usize) -> &Self::Output {
        &self.path[index]
    }
}

impl std::ops::IndexMut<usize> for STPath {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.path[index]
    }
}

impl STPathSet {
    pub fn new(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            value: Vec::new(),
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        let mut path_set = Self::new(field);
        let mut path = Vec::new();

        loop {
            let node_type = sit.get8();

            if node_type == STPathElement::TYPE_NONE || node_type == STPathElement::TYPE_BOUNDARY {
                if path.is_empty() {
                    panic!("empty path");
                }

                path_set.push_back(STPath::from_vec(path));
                path = Vec::new();

                if node_type == STPathElement::TYPE_NONE {
                    return path_set;
                }
            } else if (node_type & !STPathElement::TYPE_ALL) != 0 {
                panic!("bad path element");
            } else {
                let has_account = (node_type & STPathElement::TYPE_ACCOUNT) != 0;
                let has_currency = (node_type & STPathElement::TYPE_CURRENCY) != 0;
                let has_issuer = (node_type & STPathElement::TYPE_ISSUER) != 0;
                let has_mpt = (node_type & STPathElement::TYPE_MPT) != 0;

                assert!(
                    !(has_currency && has_mpt),
                    "xrpl::STPathSet::STPathSet : not has Currency and MPT"
                );

                let mut account = AccountID::zero();
                let mut asset = PathAsset::default();
                let mut issuer = AccountID::zero();

                if has_account {
                    account = AccountID::from_slice(sit.get160().data())
                        .expect("path account width should match");
                }
                if has_currency {
                    asset = PathAsset::from(
                        crate::Currency::from_slice(sit.get160().data())
                            .expect("path currency width should match"),
                    );
                }
                if has_mpt {
                    asset = PathAsset::from(sit.get192());
                }
                if has_issuer {
                    issuer = AccountID::from_slice(sit.get160().data())
                        .expect("path issuer width should match");
                }

                path.push(STPathElement::inferred(
                    account,
                    asset,
                    issuer,
                    has_currency || has_mpt,
                ));
            }
        }
    }

    pub fn assemble_add(&mut self, base: &STPath, tail: STPathElement) -> bool {
        let mut new_path = base.clone();
        new_path.push_back(tail);

        if self.value.contains(&new_path) {
            return false;
        }

        self.value.push(new_path);
        true
    }

    pub fn size(&self) -> usize {
        self.value.len()
    }

    pub fn empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn push_back(&mut self, path: STPath) {
        self.value.push(path);
    }

    pub fn iter(&self) -> impl Iterator<Item = &STPath> {
        self.value.iter()
    }
}

impl std::ops::Index<usize> for STPathSet {
    type Output = STPath;

    fn index(&self, index: usize) -> &Self::Output {
        &self.value[index]
    }
}

impl std::ops::IndexMut<usize> for STPathSet {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.value[index]
    }
}

impl StBase for STPathSet {
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
        SerializedTypeId::PathSet
    }

    fn json(&self, options: JsonOptions) -> JsonValue {
        JsonValue::Array(self.value.iter().map(|path| path.json(options)).collect())
    }

    fn add(&self, serializer: &mut Serializer) {
        assert!(
            self.fname().is_binary(),
            "xrpl::STPathSet::add : field is binary"
        );
        assert_eq!(
            self.fname().field_type(),
            SerializedTypeId::PathSet,
            "xrpl::STPathSet::add : valid field type"
        );

        let mut first = true;
        for path in &self.value {
            if !first {
                serializer.add8(STPathElement::TYPE_BOUNDARY);
            }

            for element in path.iter() {
                let node_type = element.node_type();
                serializer.add8(node_type);

                if (node_type & STPathElement::TYPE_ACCOUNT) != 0 {
                    serializer.add_bit_string(element.account_id());
                }
                if (node_type & STPathElement::TYPE_MPT) != 0 {
                    serializer.add_bit_string(element.mpt_id());
                }
                if (node_type & STPathElement::TYPE_CURRENCY) != 0 {
                    serializer.add_bit_string(element.currency());
                }
                if (node_type & STPathElement::TYPE_ISSUER) != 0 {
                    serializer.add_bit_string(element.issuer_id());
                }
            }

            first = false;
        }

        serializer.add8(STPathElement::TYPE_NONE);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| other.value == self.value)
    }

    fn is_default(&self) -> bool {
        self.value.is_empty()
    }
}

pub fn st_path_set_from_json(
    field: &'static SField,
    value: &JsonValue,
) -> Result<STPathSet, String> {
    let mut tail = STPathSet::new(field);
    let paths = match value {
        JsonValue::Null => &[][..],
        JsonValue::Array(paths) => paths.as_slice(),
        _ => return Err("pathset must be an array or null".to_string()),
    };

    for path_value in paths {
        let entries = match path_value {
            JsonValue::Null => &[][..],
            JsonValue::Array(entries) => entries.as_slice(),
            _ => return Err("path entry must be an array or null".to_string()),
        };

        let mut path = STPath::new();
        for entry in entries {
            let JsonValue::Object(entry) = entry else {
                return Err("path element must be an object".to_string());
            };

            let account = entry.get("account");
            let currency = entry.get("currency");
            let mpt = entry.get("mpt_issuance_id");
            let issuer = entry.get("issuer");

            if account.is_none() && currency.is_none() && mpt.is_none() && issuer.is_none() {
                return Err(
                    "path element must contain account, currency, mpt_issuance_id, or issuer"
                        .to_string(),
                );
            }
            if currency.is_some() && mpt.is_some() {
                return Err("path element cannot contain both currency and mpt_issuance_id".into());
            }

            let mut account_id = AccountID::zero();
            let mut issuer_id = AccountID::zero();
            let mut asset_id = PathAsset::default();
            let mut has_asset = false;

            if let Some(account) = account {
                let JsonValue::String(account) = account else {
                    return Err("path account must be a string".to_string());
                };
                if !account_id.parse_hex(account) {
                    account_id = parse_base58_account_id(account)
                        .ok_or_else(|| "invalid path account".to_string())?;
                }
            }

            if let Some(currency) = currency {
                let JsonValue::String(currency) = currency else {
                    return Err("path currency must be a string".to_string());
                };
                has_asset = true;
                let mut currency_id = crate::Currency::zero();
                if !currency_id.parse_hex(currency) && !to_currency(&mut currency_id, currency) {
                    return Err("invalid path currency".to_string());
                }
                asset_id = PathAsset::from(currency_id);
            }

            if let Some(mpt) = mpt {
                let JsonValue::String(mpt) = mpt else {
                    return Err("path mpt_issuance_id must be a string".to_string());
                };
                has_asset = true;
                let mut mpt_id = MPTID::zero();
                if !mpt_id.parse_hex(mpt) {
                    return Err("invalid path mpt_issuance_id".to_string());
                }
                asset_id = PathAsset::from(mpt_id);
            }

            if let Some(issuer) = issuer {
                let JsonValue::String(issuer) = issuer else {
                    return Err("path issuer must be a string".to_string());
                };
                if !issuer_id.parse_hex(issuer) {
                    issuer_id = parse_base58_account_id(issuer)
                        .ok_or_else(|| "invalid path issuer".to_string())?;
                }
            }

            path.push_back(STPathElement::inferred(
                account_id, asset_id, issuer_id, has_asset,
            ));
        }

        tail.push_back(path);
    }

    Ok(tail)
}
