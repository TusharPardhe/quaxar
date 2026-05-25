//! `STLedgerEntry` core port from `xrpl/protocol/STLedgerEntry.*`.

use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
};

use basics::base_uint::Uint256;

use crate::keylet::ledger_entry_type_from_code;
use crate::{
    JsonOptions, JsonValue, Keylet, LedgerEntryType, LedgerFormats, Rules, STObject, SerialIter,
    SerializedTypeId, StBase, StBaseCore, ValidationError, fix_previous_txn_id,
    get_field_by_symbol, make_mpt_id,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct STLedgerEntry {
    object: STObject,
    key: Uint256,
    entry_type: LedgerEntryType,
}

impl STLedgerEntry {
    pub fn new(keylet: Keylet) -> Self {
        let mut object = STObject::new(get_field_by_symbol("sfLedgerEntry"));
        let format = LedgerFormats::get_instance()
            .find_by_type(keylet.entry_type)
            .unwrap_or_else(|| {
                panic!(
                    "Attempt to create a SLE of unknown type {}",
                    keylet.entry_type as u16
                )
            });
        object.set(format.so_template());
        object.set_field_u16(
            get_field_by_symbol("sfLedgerEntryType"),
            keylet.entry_type as u16,
        );

        Self {
            object,
            key: keylet.key,
            entry_type: keylet.entry_type,
        }
    }

    pub fn from_type_and_key(entry_type: LedgerEntryType, key: Uint256) -> Self {
        Self::new(Keylet::new(entry_type, key))
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, index: Uint256) -> Self {
        let object = STObject::from_serial_iter(sit, get_field_by_symbol("sfLedgerEntry"), 0);
        Self::from_stobject(object, index)
    }

    pub fn from_stobject(object: STObject, index: Uint256) -> Self {
        let mut entry = Self {
            object,
            key: index,
            entry_type: LedgerEntryType::Any,
        };
        entry.set_sle_type();
        entry
    }

    pub fn key(&self) -> &Uint256 {
        &self.key
    }

    pub fn get_type(&self) -> LedgerEntryType {
        self.entry_type
    }

    pub fn clone_as_object(&self) -> STObject {
        self.object.clone()
    }

    pub fn is_threaded_type(&self, rules: &Rules) -> bool {
        let excluded = matches!(
            self.entry_type,
            LedgerEntryType::DirectoryNode
                | LedgerEntryType::Amendments
                | LedgerEntryType::FeeSettings
                | LedgerEntryType::NegativeUnl
                | LedgerEntryType::AMM
        );

        (!excluded || rules.enabled(&fix_previous_txn_id()))
            && self.get_field_index(get_field_by_symbol("sfPreviousTxnID")) != -1
    }

    pub fn thread(
        &mut self,
        tx_id: Uint256,
        ledger_seq: u32,
        prev_tx_id: &mut Uint256,
        prev_ledger_id: &mut u32,
    ) -> bool {
        let old_prev_tx_id = self.get_field_h256(get_field_by_symbol("sfPreviousTxnID"));
        if old_prev_tx_id == tx_id {
            assert_eq!(
                self.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq")),
                ledger_seq,
                "xrpl::STLedgerEntry::thread : ledger sequence match"
            );
            return false;
        }

        *prev_tx_id = old_prev_tx_id;
        *prev_ledger_id = self.get_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"));
        self.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), tx_id);
        self.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), ledger_seq);
        true
    }

    fn set_sle_type(&mut self) {
        let entry_type = ledger_entry_type_from_code(
            self.get_field_u16(get_field_by_symbol("sfLedgerEntryType")),
        )
        .and_then(|entry_type| LedgerFormats::get_instance().find_by_type(entry_type))
        .unwrap_or_else(|| panic!("invalid ledger entry type"));

        self.entry_type = entry_type.format_type();
        self.object.apply_template(entry_type.so_template());
    }
}

impl Deref for STLedgerEntry {
    type Target = STObject;

    fn deref(&self) -> &Self::Target {
        &self.object
    }
}

impl DerefMut for STLedgerEntry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.object
    }
}

impl StBase for STLedgerEntry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        self.object.core()
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        self.object.core_mut()
    }

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::LedgerEntry
    }

    fn full_text(&self) -> String {
        let format = LedgerFormats::get_instance()
            .find_by_type(self.entry_type)
            .unwrap_or_else(|| panic!("invalid ledger entry type"));

        format!(
            "\"{}\" = {{ {}, {}}}",
            self.key,
            format.name(),
            self.object.full_text()
        )
    }

    fn text(&self) -> String {
        format!("{{ {}, {} }}", self.key, self.object.text())
    }

    fn json(&self, options: JsonOptions) -> JsonValue {
        let JsonValue::Object(mut object) = self.object.json(options) else {
            unreachable!("STObject::json must produce an object");
        };

        object.insert("index".to_string(), JsonValue::String(self.key.to_string()));

        if self.entry_type == LedgerEntryType::MPTokenIssuance {
            object.insert(
                "mpt_issuance_id".to_string(),
                JsonValue::String(
                    make_mpt_id(
                        self.get_field_u32(get_field_by_symbol("sfSequence")),
                        self.get_account_id(get_field_by_symbol("sfIssuer")),
                    )
                    .to_string(),
                ),
            );
        }

        JsonValue::Object(BTreeMap::from_iter(object))
    }

    fn add(&self, serializer: &mut crate::Serializer) {
        self.object.add(serializer);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        let Some(other) = other.as_any().downcast_ref::<Self>() else {
            return false;
        };
        self.key == other.key
            && self.entry_type == other.entry_type
            && self.object.is_equivalent(&other.object)
    }

    fn is_default(&self) -> bool {
        self.object.is_default()
    }

    fn is_valid(&self) -> bool {
        self.object.is_valid()
    }

    fn check(&self) -> Result<(), ValidationError> {
        self.object.check()
    }
}
