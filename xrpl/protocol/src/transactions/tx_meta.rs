//! `xrpl/protocol/TxMeta.*` owner port.

use std::collections::BTreeSet;

use basics::base_uint::Uint256;

use crate::{
    AccountID, JsonOptions, JsonValue, MPTID, SField, STAmount, STArray, STLedgerEntry, STObject,
    STUInt192, SerialIter, Serializer, StBase, Ter, get_field_by_symbol,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMeta {
    transaction_id: Uint256,
    ledger_seq: u32,
    index: u32,
    result: i32,
    delivered_amount: Option<STAmount>,
    parent_batch_id: Option<Uint256>,
    nodes: STArray,
}

impl TxMeta {
    pub fn new(transaction_id: Uint256, ledger_seq: u32) -> Self {
        let mut nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
        nodes.reserve(32);
        Self {
            transaction_id,
            ledger_seq,
            index: u32::MAX,
            result: 255,
            delivered_amount: None,
            parent_batch_id: None,
            nodes,
        }
    }

    pub fn from_stobject(transaction_id: Uint256, ledger_seq: u32, object: STObject) -> Self {
        let nodes = object.get_field_array(get_field_by_symbol("sfAffectedNodes"));
        let mut meta = Self {
            transaction_id,
            ledger_seq,
            index: object.get_field_u32(get_field_by_symbol("sfTransactionIndex")),
            result: i32::from(object.get_field_u8(get_field_by_symbol("sfTransactionResult"))),
            delivered_amount: None,
            parent_batch_id: None,
            nodes,
        };
        meta.set_additional_fields(&object);
        meta
    }

    pub fn from_raw(transaction_id: Uint256, ledger_seq: u32, bytes: &[u8]) -> Self {
        let mut sit = SerialIter::new(bytes);
        let object = STObject::from_serial_iter(&mut sit, get_field_by_symbol("sfMetadata"), 0);
        Self::from_stobject(transaction_id, ledger_seq, object)
    }

    pub fn get_tx_id(&self) -> Uint256 {
        self.transaction_id
    }

    pub fn get_lgr_seq(&self) -> u32 {
        self.ledger_seq
    }

    pub fn get_result(&self) -> i32 {
        self.result
    }

    pub fn get_result_ter(&self) -> Ter {
        Ter::from_int(self.result)
    }

    pub fn get_index(&self) -> u32 {
        self.index
    }

    pub fn set_affected_node(&mut self, node: Uint256, type_: &'static SField, node_type: u16) {
        for existing in self.nodes.iter_mut() {
            if existing.get_field_h256(get_field_by_symbol("sfLedgerIndex")) == node {
                existing.set_fname(type_);
                existing.set_field_u16(get_field_by_symbol("sfLedgerEntryType"), node_type);
                return;
            }
        }

        self.nodes.push_back(STObject::new(type_));
        let object = self
            .nodes
            .last_mut()
            .expect("pushed affected node must be present");
        assert_eq!(
            object.fname(),
            type_,
            "xrpl::TxMeta::setAffectedNode : field type match"
        );
        object.set_field_h256(get_field_by_symbol("sfLedgerIndex"), node);
        object.set_field_u16(get_field_by_symbol("sfLedgerEntryType"), node_type);
    }

    pub fn get_affected_node_for_sle(
        &mut self,
        node: &STLedgerEntry,
        type_: &'static SField,
    ) -> &mut STObject {
        let node_key = *node.key();
        let existing_index = self
            .nodes
            .iter()
            .enumerate()
            .find_map(|(index, candidate)| {
                (candidate.get_field_h256(get_field_by_symbol("sfLedgerIndex")) == node_key)
                    .then_some(index)
            });

        if let Some(index) = existing_index {
            return self
                .nodes
                .get_mut(index)
                .expect("existing affected node index must remain valid");
        }

        self.nodes.push_back(STObject::new(type_));
        let object = self
            .nodes
            .last_mut()
            .expect("pushed affected node must be present");
        assert_eq!(
            object.fname(),
            type_,
            "xrpl::TxMeta::getAffectedNode(SLE::ref) : field type match"
        );
        object.set_field_h256(get_field_by_symbol("sfLedgerIndex"), node_key);
        object.set_field_u16(
            get_field_by_symbol("sfLedgerEntryType"),
            node.get_field_u16(get_field_by_symbol("sfLedgerEntryType")),
        );
        object
    }

    pub fn get_affected_node(&mut self, node: Uint256) -> &mut STObject {
        let index = self
            .nodes
            .iter()
            .position(|candidate| {
                candidate.get_field_h256(get_field_by_symbol("sfLedgerIndex")) == node
            })
            .unwrap_or_else(|| panic!("Affected node not found"));
        self.nodes
            .get_mut(index)
            .expect("existing affected node index must remain valid")
    }

    pub fn get_affected_accounts(&self) -> BTreeSet<AccountID> {
        let mut accounts = BTreeSet::new();

        for node in self.nodes.iter() {
            let payload_field = if node.fname() == get_field_by_symbol("sfCreatedNode") {
                get_field_by_symbol("sfNewFields")
            } else {
                get_field_by_symbol("sfFinalFields")
            };

            if !node.is_field_present(payload_field) {
                continue;
            }

            let inner = node.get_field_object(payload_field);
            for field in inner.iter() {
                if let Some(account) = field.as_any().downcast_ref::<crate::STAccount>() {
                    if !account.is_default() {
                        accounts.insert(*account.value());
                    }
                    continue;
                }

                if matches!(
                    field.fname(),
                    name if name == get_field_by_symbol("sfLowLimit")
                        || name == get_field_by_symbol("sfHighLimit")
                        || name == get_field_by_symbol("sfTakerPays")
                        || name == get_field_by_symbol("sfTakerGets")
                ) {
                    if let Some(amount) = field.as_any().downcast_ref::<STAmount>()
                        && amount.holds_issue()
                    {
                        let issuer = amount.issue().issuer();
                        if !issuer.is_zero() {
                            accounts.insert(issuer);
                        }
                    }
                    continue;
                }

                if field.fname() == get_field_by_symbol("sfMPTokenIssuanceID")
                    && let Some(mpt_id) = field.as_any().downcast_ref::<STUInt192>()
                {
                    let mpt_id = MPTID::from_array(*mpt_id.value().data());
                    let issuer = AccountID::from_slice(&mpt_id.data()[4..])
                        .expect("MPTID should contain 20-byte AccountID");
                    if !issuer.is_zero() {
                        accounts.insert(issuer);
                    }
                }
            }
        }

        accounts
    }

    pub fn get_json(&self, options: JsonOptions) -> JsonValue {
        self.get_as_object().json(options)
    }

    pub fn add_raw(&mut self, serializer: &mut Serializer, result: Ter, index: u32) {
        self.result = result.to_int();
        self.index = index;
        assert!(
            self.result == 0 || ((self.result > 100) && (self.result <= 255)),
            "xrpl::TxMeta::addRaw : valid TER input"
        );

        let mut sorted_nodes: Vec<_> = self.nodes.iter().cloned().collect();
        sorted_nodes.sort_by_key(|node| node.get_field_h256(get_field_by_symbol("sfLedgerIndex")));

        self.nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
        self.nodes.reserve(sorted_nodes.len());
        for node in sorted_nodes {
            self.nodes.push_back(node);
        }

        self.get_as_object().add(serializer);
    }

    pub fn get_as_object(&self) -> STObject {
        let mut metadata = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
        assert_ne!(
            self.result, 255,
            "xrpl::TxMeta::getAsObject : result_ is set"
        );
        metadata.set_field_u8(
            get_field_by_symbol("sfTransactionResult"),
            self.result as u8,
        );
        metadata.set_field_u32(get_field_by_symbol("sfTransactionIndex"), self.index);
        metadata.set_field_array(get_field_by_symbol("sfAffectedNodes"), self.nodes.clone());

        if let Some(amount) = &self.delivered_amount {
            metadata.set_field_amount(get_field_by_symbol("sfDeliveredAmount"), amount.clone());
        }

        if let Some(parent_batch_id) = self.parent_batch_id {
            metadata.set_field_h256(get_field_by_symbol("sfParentBatchID"), parent_batch_id);
        }

        metadata
    }

    pub fn get_nodes(&self) -> &STArray {
        &self.nodes
    }

    pub fn get_nodes_mut(&mut self) -> &mut STArray {
        &mut self.nodes
    }

    pub fn set_additional_fields(&mut self, object: &STObject) {
        if object.is_field_present(get_field_by_symbol("sfDeliveredAmount")) {
            self.delivered_amount =
                Some(object.get_field_amount(get_field_by_symbol("sfDeliveredAmount")));
        }

        if object.is_field_present(get_field_by_symbol("sfParentBatchID")) {
            self.parent_batch_id =
                Some(object.get_field_h256(get_field_by_symbol("sfParentBatchID")));
        }
    }

    pub fn get_delivered_amount(&self) -> Option<&STAmount> {
        self.delivered_amount.as_ref()
    }

    pub fn set_delivered_amount(&mut self, amount: Option<STAmount>) {
        self.delivered_amount = amount;
    }

    pub fn get_parent_batch_id(&self) -> Option<Uint256> {
        self.parent_batch_id
    }

    pub fn set_parent_batch_id(&mut self, id: Option<Uint256>) {
        self.parent_batch_id = id;
    }
}
