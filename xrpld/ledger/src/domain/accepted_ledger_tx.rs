//! `AcceptedLedgerTx` owner port.

use crate::Ledger;
use crate::token_helpers::{FreezeHandling, account_funds_text};
use basics::{base_uint::Uint256, string_utilities::sql_blob_literal};
use protocol::{
    AccountID, JsonOptions, JsonValue, STObject, STTx, StBase, Ter, TxMeta, TxType,
    get_field_by_symbol, ter::trans_human,
};
use shamap::traversal::TraversalError;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub type AcceptedLedgerTxMeta = TxMeta;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedLedgerTx {
    txn: Arc<STTx>,
    meta: AcceptedLedgerTxMeta,
    affected: BTreeSet<AccountID>,
    raw_meta: Vec<u8>,
    json: JsonValue,
}

impl AcceptedLedgerTx {
    pub fn new(
        ledger: &Ledger,
        txn: Arc<STTx>,
        meta: AcceptedLedgerTxMeta,
    ) -> Result<Self, TraversalError> {
        let raw_meta = meta.get_as_object().get_serializer().data().to_vec();
        let affected = meta.get_affected_accounts();
        let json = build_json(Some(ledger), txn.as_ref(), &meta, &raw_meta, &affected)?;

        Ok(Self {
            txn,
            meta,
            affected,
            raw_meta,
            json,
        })
    }

    pub fn from_meta(ledger_seq: u32, txn: STTx, meta: STObject) -> Self {
        let txn = Arc::new(txn);
        let raw_meta = meta.get_serializer().data().to_vec();
        let meta = AcceptedLedgerTxMeta::from_stobject(txn.get_transaction_id(), ledger_seq, meta);
        let affected = meta.get_affected_accounts();
        let json = build_json(None, txn.as_ref(), &meta, &raw_meta, &affected)
            .expect("accepted ledger tx fixture json should build");

        Self {
            txn,
            meta,
            affected,
            raw_meta,
            json,
        }
    }

    pub fn get_txn(&self) -> &STTx {
        self.txn.as_ref()
    }

    pub fn get_meta(&self) -> &TxMeta {
        &self.meta
    }

    pub fn get_affected(&self) -> &BTreeSet<AccountID> {
        &self.affected
    }

    pub fn get_transaction_id(&self) -> Uint256 {
        self.txn.get_transaction_id()
    }

    pub fn get_txn_type(&self) -> TxType {
        self.txn.get_txn_type()
    }

    pub fn get_result(&self) -> Ter {
        self.meta.get_result_ter()
    }

    pub fn get_txn_seq(&self) -> u32 {
        self.meta.get_index()
    }

    pub fn get_esc_meta(&self) -> String {
        assert!(
            !self.raw_meta.is_empty(),
            "xrpl::AcceptedLedgerTx::getEscMeta : metadata is set"
        );
        sql_blob_literal(&self.raw_meta)
    }

    pub fn get_json(&self) -> &JsonValue {
        &self.json
    }
}

fn build_json(
    ledger: Option<&Ledger>,
    txn: &STTx,
    meta: &TxMeta,
    raw_meta: &[u8],
    affected: &BTreeSet<AccountID>,
) -> Result<JsonValue, TraversalError> {
    let mut transaction = match txn.json(JsonOptions::NONE) {
        JsonValue::Object(object) => object,
        _ => panic!("accepted ledger transaction json should be an object"),
    };

    if let Some(ledger) = ledger {
        maybe_insert_owner_funds(ledger, txn, &mut transaction)?;
    }

    let mut json = BTreeMap::from([
        ("transaction".to_string(), JsonValue::Object(transaction)),
        ("meta".to_string(), meta.get_json(JsonOptions::NONE)),
        (
            "raw_meta".to_string(),
            JsonValue::String(basics::str_hex::str_hex(raw_meta)),
        ),
        (
            "result".to_string(),
            JsonValue::String(trans_human(meta.get_result_ter()).to_string()),
        ),
    ]);

    if !affected.is_empty() {
        json.insert(
            "affected".to_string(),
            JsonValue::Array(
                affected
                    .iter()
                    .copied()
                    .map(protocol::to_base58)
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }

    Ok(JsonValue::Object(json))
}

fn maybe_insert_owner_funds(
    ledger: &Ledger,
    txn: &STTx,
    transaction: &mut BTreeMap<String, JsonValue>,
) -> Result<(), TraversalError> {
    if txn.get_txn_type() != TxType::OFFER_CREATE {
        return Ok(());
    }

    let account = txn.get_account_id(get_field_by_symbol("sfAccount"));
    let amount = txn.get_field_amount(get_field_by_symbol("sfTakerGets"));
    if amount.issue().issuer() == account {
        return Ok(());
    }

    // Keep accepted-ledger persistence resilient when a specific owner-funds
    // lookup cannot be resolved from the current state snapshot.
    let Ok(owner_funds) =
        account_funds_text(ledger, account, &amount, FreezeHandling::IgnoreFreeze)
    else {
        return Ok(());
    };
    transaction.insert("owner_funds".to_string(), JsonValue::String(owner_funds));
    Ok(())
}
