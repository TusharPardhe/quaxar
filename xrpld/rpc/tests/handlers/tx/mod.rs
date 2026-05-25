//! Tests for the tx RPC handler.

//! Tests for the tx RPC handler.

use std::{collections::BTreeMap, sync::Arc};

use app::{AppOpenLedgerView, ApplicationRoot, Transaction};
use basics::{
    base_uint::{Uint160, Uint256},
    chrono::NetClockTimePoint,
};
use ledger::{LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader};
use protocol::{
    AccountID, JsonValue, KeyType, LedgerEntryType, STAmount, STArray, STLedgerEntry, STObject,
    STTx, SecretKey, TxMeta, TxType, account_keylet, calc_account_id, derive_public_key,
    get_field_by_symbol,
};
use rpc::{TxRequest, TxSource, do_tx};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

pub use rpc::{TxLookupOutcome, TxRecord, TxSearched};

pub(super) fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

pub(super) fn account_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

pub(super) fn payment_tx() -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 5);
    })
}

pub(super) fn signed_payment_tx(
    seed: u8,
    destination: AccountID,
    sequence: u32,
    fee_drops: u64,
) -> (AccountID, Arc<STTx>) {
    let secret = SecretKey::from_bytes([seed; 32]);
    let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
    let source = calc_account_id(public.as_bytes());
    let mut tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        tx.set_field_vl(get_field_by_symbol("sfSigningPubKey"), public.as_bytes());
    });
    tx.sign(&public, &secret, None)
        .expect("signature should succeed");
    (source, Arc::new(tx))
}

pub(super) fn funded_parent_ledger(seq: u32, account: AccountID, account_sequence: u32) -> Ledger {
    let mut state_tree = MutableTree::new(1);
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_uint160(account)).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
    account_root.set_field_u32(get_field_by_symbol("sfSequence"), account_sequence);
    account_root.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(1_000_000_000, false),
    );
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(
                account_keylet(account_uint160(account)).key,
                account_root.get_serializer().data().to_vec(),
            ),
        )
        .expect("account root should insert");

    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 800 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            MutableTree::new(1).root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Modifying,
        ),
    )
}

pub(super) fn payment_meta(tx_id: Uint256) -> TxMeta {
    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
    object.set_field_amount(
        get_field_by_symbol("sfDeliveredAmount"),
        STAmount::new_native(1_000_000, false),
    );
    object.set_field_array(
        get_field_by_symbol("sfAffectedNodes"),
        STArray::new(get_field_by_symbol("sfAffectedNodes")),
    );
    TxMeta::from_stobject(tx_id, 3, object)
}

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Debug, Clone)]
struct FakeTxSource {
    enabled: bool,
    synced: bool,
    network_id: u32,
    by_hash: BTreeMap<String, Result<TxLookupOutcome, rpc::TxLookupError>>,
    by_ctid: BTreeMap<(u32, u16), Result<TxLookupOutcome, rpc::TxLookupError>>,
}

impl TxSource for FakeTxSource {
    fn tx_tables_enabled(&self) -> bool {
        self.enabled
    }

    fn network_id(&self) -> u32 {
        self.network_id
    }

    fn network_synced(&self) -> bool {
        self.synced
    }

    fn lookup_transaction_by_hash(
        &self,
        hash: Uint256,
        _ledger_range: Option<(u32, u32)>,
    ) -> Result<TxLookupOutcome, rpc::TxLookupError> {
        self.by_hash
            .get(&hash.to_string())
            .cloned()
            .unwrap_or(Ok(TxLookupOutcome::NotFound(TxSearched::Unknown)))
    }

    fn lookup_transaction_by_ctid(
        &self,
        ledger_seq: u32,
        txn_index: u16,
        _ledger_range: Option<(u32, u32)>,
    ) -> Result<TxLookupOutcome, rpc::TxLookupError> {
        self.by_ctid
            .get(&(ledger_seq, txn_index))
            .cloned()
            .unwrap_or(Ok(TxLookupOutcome::NotFound(TxSearched::Unknown)))
    }
}

pub(super) fn found_record(txn: Arc<STTx>, meta: Option<TxMeta>) -> TxRecord {
    TxRecord {
        txn,
        meta,
        ledger_index: 3,
        close_time: Some(NetClockTimePoint::new(10)),
        ledger_hash: Some(Uint256::from_array([0x33; 32])),
        validated: true,
        txn_index: Some(3),
        network_id: Some(0),
    }
}

pub(super) fn found_record_with_network(
    txn: Arc<STTx>,
    meta: Option<TxMeta>,
    network_id: Option<u32>,
) -> TxRecord {
    let mut record = found_record(txn, meta);
    record.network_id = network_id;
    record
}

mod formatting;
mod lookup;
