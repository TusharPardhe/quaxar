//! `STXChainBridge` port from `xrpl/protocol/STXChainBridge.*`.

use crate::{
    AccountID, Asset, JsonOptions, JsonValue, SField, STAccount, STIssue, STObject, SerialIter,
    SerializedTypeId, Serializer, StBase, StBaseCore, asset_from_json, downcast_stbase_ref,
    get_field_by_symbol, parse_base58_account_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainType {
    Locking,
    Issuing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct STXChainBridge {
    core: StBaseCore,
    locking_chain_door: STAccount,
    locking_chain_issue: STIssue,
    issuing_chain_door: STAccount,
    issuing_chain_issue: STIssue,
}

impl STXChainBridge {
    pub fn new() -> Self {
        Self::with_field(get_field_by_symbol("sfXChainBridge"))
    }

    pub fn with_field(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            locking_chain_door: STAccount::with_field(get_field_by_symbol("sfLockingChainDoor")),
            locking_chain_issue: STIssue::with_field(get_field_by_symbol("sfLockingChainIssue")),
            issuing_chain_door: STAccount::with_field(get_field_by_symbol("sfIssuingChainDoor")),
            issuing_chain_issue: STIssue::with_field(get_field_by_symbol("sfIssuingChainIssue")),
        }
    }

    pub fn from_parts(
        locking_chain_door: AccountID,
        locking_chain_issue: impl Into<Asset>,
        issuing_chain_door: AccountID,
        issuing_chain_issue: impl Into<Asset>,
    ) -> Self {
        Self {
            core: StBaseCore::with_field(get_field_by_symbol("sfXChainBridge")),
            locking_chain_door: STAccount::from_value(
                get_field_by_symbol("sfLockingChainDoor"),
                locking_chain_door,
            ),
            locking_chain_issue: STIssue::new_with_asset(
                get_field_by_symbol("sfLockingChainIssue"),
                locking_chain_issue,
            ),
            issuing_chain_door: STAccount::from_value(
                get_field_by_symbol("sfIssuingChainDoor"),
                issuing_chain_door,
            ),
            issuing_chain_issue: STIssue::new_with_asset(
                get_field_by_symbol("sfIssuingChainIssue"),
                issuing_chain_issue,
            ),
        }
    }

    pub fn from_st_object(object: &STObject) -> Self {
        Self {
            core: StBaseCore::with_field(get_field_by_symbol("sfXChainBridge")),
            locking_chain_door: STAccount::from_value(
                get_field_by_symbol("sfLockingChainDoor"),
                object.get_account_id(get_field_by_symbol("sfLockingChainDoor")),
            ),
            locking_chain_issue: object.get_field_issue(get_field_by_symbol("sfLockingChainIssue")),
            issuing_chain_door: STAccount::from_value(
                get_field_by_symbol("sfIssuingChainDoor"),
                object.get_account_id(get_field_by_symbol("sfIssuingChainDoor")),
            ),
            issuing_chain_issue: object.get_field_issue(get_field_by_symbol("sfIssuingChainIssue")),
        }
    }

    pub fn from_json_value(field: &'static SField, value: &JsonValue) -> Result<Self, String> {
        let JsonValue::Object(object) = value else {
            return Err("STXChainBridge can only be specified with an object Json value".into());
        };

        let expected = [
            "LockingChainDoor",
            "LockingChainIssue",
            "IssuingChainDoor",
            "IssuingChainIssue",
        ];
        for key in object.keys() {
            if !expected.contains(&key.as_str()) {
                return Err(format!("STXChainBridge extra field detected: {key}"));
            }
        }

        let locking_chain_door = object
            .get("LockingChainDoor")
            .and_then(|value| match value {
                JsonValue::String(value) => parse_base58_account_id(value),
                _ => None,
            })
            .ok_or_else(|| "STXChainBridge LockingChainDoor must be a valid account".to_owned())?;
        let issuing_chain_door = object
            .get("IssuingChainDoor")
            .and_then(|value| match value {
                JsonValue::String(value) => parse_base58_account_id(value),
                _ => None,
            })
            .ok_or_else(|| "STXChainBridge IssuingChainDoor must be a valid account".to_owned())?;

        let locking_chain_issue = asset_from_json(
            object
                .get("LockingChainIssue")
                .ok_or_else(|| "STXChainBridge LockingChainIssue is required".to_owned())?,
        )?;
        let issuing_chain_issue = asset_from_json(
            object
                .get("IssuingChainIssue")
                .ok_or_else(|| "STXChainBridge IssuingChainIssue is required".to_owned())?,
        )?;

        let mut bridge = Self::from_parts(
            locking_chain_door,
            locking_chain_issue,
            issuing_chain_door,
            issuing_chain_issue,
        );
        bridge.set_fname(field);
        Ok(bridge)
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        let mut bridge = Self::with_field(field);
        bridge.locking_chain_door =
            STAccount::from_serial_iter(sit, get_field_by_symbol("sfLockingChainDoor"));
        bridge.locking_chain_issue =
            STIssue::from_serial_iter(sit, get_field_by_symbol("sfLockingChainIssue"));
        bridge.issuing_chain_door =
            STAccount::from_serial_iter(sit, get_field_by_symbol("sfIssuingChainDoor"));
        bridge.issuing_chain_issue =
            STIssue::from_serial_iter(sit, get_field_by_symbol("sfIssuingChainIssue"));
        bridge
    }

    pub const fn other_chain(chain_type: ChainType) -> ChainType {
        match chain_type {
            ChainType::Locking => ChainType::Issuing,
            ChainType::Issuing => ChainType::Locking,
        }
    }

    pub const fn src_chain(was_locking_chain_send: bool) -> ChainType {
        if was_locking_chain_send {
            ChainType::Locking
        } else {
            ChainType::Issuing
        }
    }

    pub const fn dst_chain(was_locking_chain_send: bool) -> ChainType {
        if was_locking_chain_send {
            ChainType::Issuing
        } else {
            ChainType::Locking
        }
    }

    pub fn to_st_object(&self) -> STObject {
        let mut object = STObject::new(get_field_by_symbol("sfXChainBridge"));
        object.set_account_id(
            get_field_by_symbol("sfLockingChainDoor"),
            *self.locking_chain_door.value(),
        );
        object.set_field_issue(
            get_field_by_symbol("sfLockingChainIssue"),
            self.locking_chain_issue.clone(),
        );
        object.set_account_id(
            get_field_by_symbol("sfIssuingChainDoor"),
            *self.issuing_chain_door.value(),
        );
        object.set_field_issue(
            get_field_by_symbol("sfIssuingChainIssue"),
            self.issuing_chain_issue.clone(),
        );
        object
    }

    pub fn locking_chain_door(&self) -> AccountID {
        *self.locking_chain_door.value()
    }

    pub fn locking_chain_issue(&self) -> Asset {
        self.locking_chain_issue.asset()
    }

    pub fn issuing_chain_door(&self) -> AccountID {
        *self.issuing_chain_door.value()
    }

    pub fn issuing_chain_issue(&self) -> Asset {
        self.issuing_chain_issue.asset()
    }

    pub fn door(&self, chain_type: ChainType) -> AccountID {
        match chain_type {
            ChainType::Locking => self.locking_chain_door(),
            ChainType::Issuing => self.issuing_chain_door(),
        }
    }

    pub fn issue(&self, chain_type: ChainType) -> Asset {
        match chain_type {
            ChainType::Locking => self.locking_chain_issue(),
            ChainType::Issuing => self.issuing_chain_issue(),
        }
    }
}

impl Default for STXChainBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl StBase for STXChainBridge {
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
        SerializedTypeId::XChainBridge
    }

    fn text(&self) -> String {
        format!(
            "{{ {} = {}, {} = {}, {} = {}, {} = {} }}",
            get_field_by_symbol("sfLockingChainDoor").name(),
            self.locking_chain_door.text(),
            get_field_by_symbol("sfLockingChainIssue").name(),
            self.locking_chain_issue.text(),
            get_field_by_symbol("sfIssuingChainDoor").name(),
            self.issuing_chain_door.text(),
            get_field_by_symbol("sfIssuingChainIssue").name(),
            self.issuing_chain_issue.text()
        )
    }

    fn json(&self, options: JsonOptions) -> JsonValue {
        JsonValue::Object(
            [
                (
                    "LockingChainDoor".to_owned(),
                    self.locking_chain_door.json(options),
                ),
                (
                    "LockingChainIssue".to_owned(),
                    self.locking_chain_issue.json(options),
                ),
                (
                    "IssuingChainDoor".to_owned(),
                    self.issuing_chain_door.json(options),
                ),
                (
                    "IssuingChainIssue".to_owned(),
                    self.issuing_chain_issue.json(options),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }

    fn add(&self, serializer: &mut Serializer) {
        self.locking_chain_door.add(serializer);
        self.locking_chain_issue.add(serializer);
        self.issuing_chain_door.add(serializer);
        self.issuing_chain_issue.add(serializer);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other) == self
    }

    fn is_default(&self) -> bool {
        self.locking_chain_door.is_default()
            && self.locking_chain_issue.is_default()
            && self.issuing_chain_door.is_default()
            && self.issuing_chain_issue.is_default()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Currency, Issue, JsonValue, StBase, xrp_issue};

    use super::{ChainType, STXChainBridge};

    #[test]
    fn xchain_bridge_serializes_in() {
        let bridge = STXChainBridge::from_parts(
            crate::AccountID::from_u64(11),
            xrp_issue(),
            crate::AccountID::from_u64(22),
            Issue::new(Currency::from_u64(7), crate::AccountID::from_u64(33)),
        );
        let serializer = {
            let mut serializer = crate::Serializer::default();
            bridge.add(&mut serializer);
            serializer
        };
        let reparsed = STXChainBridge::from_serial_iter(
            &mut crate::SerialIter::new(serializer.data()),
            crate::get_field_by_symbol("sfXChainBridge"),
        );
        assert_eq!(reparsed, bridge);
        assert_eq!(
            bridge.door(ChainType::Locking),
            crate::AccountID::from_u64(11)
        );
        assert_eq!(
            bridge.door(ChainType::Issuing),
            crate::AccountID::from_u64(22)
        );
    }

    #[test]
    fn xchain_bridge_json_round_trips() {
        let value = JsonValue::Object(
            [
                (
                    "LockingChainDoor".to_owned(),
                    JsonValue::String(crate::to_base58(crate::AccountID::from_u64(11))),
                ),
                (
                    "LockingChainIssue".to_owned(),
                    JsonValue::Object(
                        [("currency".to_owned(), JsonValue::String("XRP".to_owned()))]
                            .into_iter()
                            .collect(),
                    ),
                ),
                (
                    "IssuingChainDoor".to_owned(),
                    JsonValue::String(crate::to_base58(crate::AccountID::from_u64(22))),
                ),
                (
                    "IssuingChainIssue".to_owned(),
                    JsonValue::Object(
                        [("currency".to_owned(), JsonValue::String("XRP".to_owned()))]
                            .into_iter()
                            .collect(),
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        );
        let bridge =
            STXChainBridge::from_json_value(crate::get_field_by_symbol("sfXChainBridge"), &value)
                .expect("bridge");
        assert_eq!(bridge.json(crate::JsonOptions::NONE), value);
    }
}
