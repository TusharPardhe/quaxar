//! Tests for the transaction entry RPC handler.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

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
use rpc::{TransactionEntryRequest, TransactionEntrySource, do_transaction_entry};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

pub use rpc::LedgerLookupSource;

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn account_uint160(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn payment_tx() -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(1));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(2));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
    })
}

fn signed_payment_tx(
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

fn funded_parent_ledger(seq: u32, account: AccountID, account_sequence: u32) -> Ledger {
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

fn meta(tx_id: Uint256) -> TxMeta {
    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 3);
    object.set_field_array(
        get_field_by_symbol("sfAffectedNodes"),
        STArray::new(get_field_by_symbol("sfAffectedNodes")),
    );
    TxMeta::from_stobject(tx_id, 9, object)
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Debug, Clone)]
struct FakeSource {
    current: Option<rpc::LedgerLookupLedger>,
    closed: Option<rpc::LedgerLookupLedger>,
    validated: Option<rpc::LedgerLookupLedger>,
    by_seq: BTreeMap<u32, rpc::LedgerLookupLedger>,
    seq_hashes: BTreeMap<u32, Uint256>,
    entries: BTreeMap<(u32, String), (STTx, Option<TxMeta>)>,
    close_times: BTreeMap<u32, NetClockTimePoint>,
}

impl LedgerLookupSource for FakeSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<rpc::LedgerLookupLedger> {
        self.by_seq
            .values()
            .copied()
            .find(|ledger| ledger.hash == hash)
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<rpc::LedgerLookupLedger> {
        self.by_seq.get(&seq).copied()
    }

    fn get_current_ledger(&self) -> Option<rpc::LedgerLookupLedger> {
        self.current
    }

    fn get_closed_ledger(&self) -> Option<rpc::LedgerLookupLedger> {
        self.closed
    }

    fn get_validated_ledger(&self) -> Option<rpc::LedgerLookupLedger> {
        self.validated
    }

    fn get_valid_ledger_index(&self) -> u32 {
        9
    }

    fn get_validated_ledger_age(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn is_validated(&self, ledger: &rpc::LedgerLookupLedger) -> bool {
        self.validated
            .is_some_and(|validated| validated.seq == ledger.seq)
    }
}

impl TransactionEntrySource for FakeSource {
    fn read_transaction_entry(
        &self,
        ledger: &rpc::LedgerLookupLedger,
        tx_hash: Uint256,
    ) -> Option<(STTx, Option<TxMeta>)> {
        self.entries
            .get(&(ledger.seq, tx_hash.to_string()))
            .cloned()
    }

    fn get_close_time_by_seq(&self, ledger_seq: u32) -> Option<NetClockTimePoint> {
        self.close_times.get(&ledger_seq).copied()
    }

    fn get_hash_by_seq(&self, ledger_seq: u32) -> Option<Uint256> {
        self.seq_hashes.get(&ledger_seq).copied()
    }
}

fn closed_ledger() -> rpc::LedgerLookupLedger {
    rpc::LedgerLookupLedger {
        hash: Uint256::from_array([0x44; 32]),
        seq: 9,
        open: false,
    }
}

#[test]
fn transaction_entry_reports_missing_current_and_malformed_inputs() {
    let source = FakeSource {
        current: Some(rpc::LedgerLookupLedger {
            hash: Uint256::from_array([0x11; 32]),
            seq: 10,
            open: true,
        }),
        closed: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        by_seq: BTreeMap::from([(9, closed_ledger())]),
        seq_hashes: BTreeMap::from([(9, Uint256::from_array([0x99; 32]))]),
        entries: BTreeMap::new(),
        close_times: BTreeMap::new(),
    };

    let missing_hash = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([("ledger_index", JsonValue::Unsigned(9))]),
            api_version: 1,
            role: rpc::RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(missing_hash) = missing_hash else {
        panic!("result must be an object");
    };
    assert_eq!(
        missing_hash.get("error"),
        Some(&JsonValue::String("fieldNotFoundTransaction".to_owned()))
    );
    assert!(missing_hash.contains_key("ledger_hash"));

    let current = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("ledger_index", JsonValue::String("current".to_owned())),
                ("tx_hash", JsonValue::String("DEADBEEF".to_owned())),
            ]),
            api_version: 1,
            role: rpc::RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(current) = current else {
        panic!("result must be an object");
    };
    assert_eq!(
        current.get("error"),
        Some(&JsonValue::String("notYetImplemented".to_owned()))
    );
    assert!(current.contains_key("ledger_current_index"));

    let malformed = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("ledger_index", JsonValue::Unsigned(9)),
                ("tx_hash", JsonValue::String("DEADBEEF".to_owned())),
            ]),
            api_version: 1,
            role: rpc::RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(malformed) = malformed else {
        panic!("result must be an object");
    };
    assert_eq!(
        malformed.get("error"),
        Some(&JsonValue::String("malformedRequest".to_owned()))
    );
    assert!(malformed.contains_key("ledger_hash"));
}

#[test]
fn transaction_entry_reports_not_found_and_v1_v2_shapes() {
    let tx = payment_tx();
    let tx_id = tx.get_transaction_id();
    let meta = meta(tx_id);
    let source = FakeSource {
        current: None,
        closed: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        by_seq: BTreeMap::from([(9, closed_ledger())]),
        seq_hashes: BTreeMap::from([(9, Uint256::from_array([0x99; 32]))]),
        entries: BTreeMap::from([((9, tx_id.to_string()), (tx.clone(), Some(meta.clone())))]),
        close_times: BTreeMap::from([(9, NetClockTimePoint::new(30))]),
    };

    let not_found = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("ledger_index", JsonValue::Unsigned(9)),
                (
                    "tx_hash",
                    JsonValue::String(Uint256::from_array([0x77; 32]).to_string()),
                ),
            ]),
            api_version: 1,
            role: rpc::RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(not_found) = not_found else {
        panic!("result must be an object");
    };
    assert_eq!(
        not_found.get("error"),
        Some(&JsonValue::String("transactionNotFound".to_owned()))
    );

    let v1 = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("ledger_index", JsonValue::Unsigned(9)),
                ("tx_hash", JsonValue::String(tx_id.to_string())),
            ]),
            api_version: 1,
            role: rpc::RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(v1) = v1 else {
        panic!("result must be an object");
    };
    let JsonValue::Object(v1_tx) = v1.get("tx_json").expect("tx_json must exist") else {
        panic!("tx_json must be an object");
    };
    assert_eq!(v1.get("hash"), None);
    assert_eq!(
        v1.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x44; 32]).to_string()
        ))
    );
    assert_eq!(
        v1_tx.get("hash"),
        Some(&JsonValue::String(tx_id.to_string()))
    );
    assert!(v1_tx.contains_key("Amount"));
    assert_eq!(v1_tx.get("DeliverMax"), v1_tx.get("Amount"));
    assert!(v1.contains_key("metadata"));

    let v2 = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("ledger_index", JsonValue::Unsigned(9)),
                ("tx_hash", JsonValue::String(tx_id.to_string())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(v2) = v2 else {
        panic!("result must be an object");
    };
    let JsonValue::Object(v2_tx) = v2.get("tx_json").expect("tx_json must exist") else {
        panic!("tx_json must be an object");
    };
    assert_eq!(v2.get("hash"), Some(&JsonValue::String(tx_id.to_string())));
    assert_eq!(v2.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(v2.get("ledger_index"), Some(&JsonValue::Unsigned(9)));
    assert_eq!(
        v2.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x99; 32]).to_string()
        ))
    );
    assert_eq!(
        v2.get("close_time_iso"),
        Some(&JsonValue::String("2000-01-01T00:00:30Z".to_owned()))
    );
    assert!(!v2_tx.contains_key("hash"));
    assert!(!v2_tx.contains_key("Amount"));
    assert!(v2_tx.contains_key("DeliverMax"));
    assert!(v2.contains_key("meta"));
}

#[test]
fn transaction_entry_reads_committed_live_transactions_from_application_server_info() {
    let mut app = ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let _ = app.attach_default_network_ops_runtime();
    let (source, tx) = signed_payment_tx(0x31, account(2), 5, 10);
    let mut parent = funded_parent_ledger(1, source, 5);
    parent.set_accepted(1_111, LEDGER_DEFAULT_TIME_RESOLUTION, true);
    let parent = Arc::new(parent);
    app.on_closed_ledger(Arc::clone(&parent));
    let _ = app.open_ledger().modify(|view| {
        *view = AppOpenLedgerView::new(2, 10);
        true
    });
    let tx_id = tx.get_transaction_id();
    let mut cached = Arc::new(std::sync::Mutex::new(Transaction::new(Arc::clone(&tx))));
    app.canonicalize_transaction(&mut cached);
    assert!(app.add_held_transaction(&Transaction::new(Arc::clone(&tx))));
    app.accept_standalone_ledger()
        .expect("standalone ledger accept should succeed");

    let response = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("ledger_index", JsonValue::Unsigned(2)),
                ("tx_hash", JsonValue::String(tx_id.to_string())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::User,
        },
        &rpc::ApplicationServerInfo::new(&app),
    );
    let JsonValue::Object(object) = response else {
        panic!("response must be an object");
    };

    assert_eq!(object.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(object.get("ledger_index"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(
        object.get("hash"),
        Some(&JsonValue::String(tx_id.to_string()))
    );
}

#[test]
fn transaction_entry_invalid_tx_hash_types() {
    let source = FakeSource {
        current: Some(closed_ledger()),
        closed: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        by_seq: BTreeMap::from([(9, closed_ledger())]),
        seq_hashes: BTreeMap::new(),
        entries: BTreeMap::new(),
        close_times: BTreeMap::new(),
    };

    // Non-string tx_hash
    let result = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("tx_hash", JsonValue::Unsigned(42)),
                ("ledger_index", JsonValue::String("validated".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));

    // Boolean tx_hash
    let result = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("tx_hash", JsonValue::Bool(true)),
                ("ledger_index", JsonValue::String("validated".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));

    // Invalid hex string
    let result = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("tx_hash", JsonValue::String("not_a_hash".to_owned())),
                ("ledger_index", JsonValue::String("validated".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
}

#[test]
fn transaction_entry_ledger_not_found() {
    let source = FakeSource {
        current: Some(closed_ledger()),
        closed: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        by_seq: BTreeMap::from([(9, closed_ledger())]),
        seq_hashes: BTreeMap::new(),
        entries: BTreeMap::new(),
        close_times: BTreeMap::new(),
    };
    let tx_hash = Uint256::from_array([0xBB; 32]);

    let result = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("tx_hash", JsonValue::String(tx_hash.to_string())),
                ("ledger_index", JsonValue::Unsigned(999)),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("lgrNotFound".to_owned()))
    );
}

#[test]
fn transaction_entry_tx_not_found_in_ledger() {
    let source = FakeSource {
        current: Some(closed_ledger()),
        closed: Some(closed_ledger()),
        validated: Some(closed_ledger()),
        by_seq: BTreeMap::from([(9, closed_ledger())]),
        seq_hashes: BTreeMap::new(),
        entries: BTreeMap::new(),
        close_times: BTreeMap::new(),
    };
    let tx_hash = Uint256::from_array([0xCC; 32]);

    let result = do_transaction_entry(
        &TransactionEntryRequest {
            params: &object([
                ("tx_hash", JsonValue::String(tx_hash.to_string())),
                ("ledger_index", JsonValue::String("validated".to_owned())),
            ]),
            api_version: 2,
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("transactionNotFound".to_owned()))
    );
}
