//! Tests for the book changes RPC handler.

use std::{collections::BTreeMap, time::Duration};

use basics::base_uint::Uint256;
use protocol::{
    AccountID, IOUAmount, Issue, JsonValue, LedgerEntryType, STAmount, STArray, STObject, STTx,
    TxMeta, TxType, currency_from_string, get_field_by_symbol,
};
use rpc::{
    BookChangesLedger, BookChangesRequest, BookChangesSource, BookChangesTransaction,
    LedgerLookupLedger, LedgerLookupSource, RpcRole, do_book_changes,
};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn issue(code: &str, issuer: u8) -> Issue {
    Issue::new(currency_from_string(code), account(issuer))
}

fn amount(field: &'static protocol::SField, issue: Issue, mantissa: i64) -> STAmount {
    STAmount::from_iou_amount(
        field,
        IOUAmount::from_parts(mantissa, -6).expect("IOU amount should normalize"),
        issue,
    )
}

fn payment_tx(sequence: u32) -> STTx {
    STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x10));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x20));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    })
}

fn offer_change_meta(domain: Option<Uint256>) -> TxMeta {
    let usd = issue("USD", 0x31);
    let eur = issue("EUR", 0x32);

    let mut previous = STObject::new(get_field_by_symbol("sfPreviousFields"));
    previous.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        amount(get_field_by_symbol("sfTakerGets"), usd, 70),
    );
    previous.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        amount(get_field_by_symbol("sfTakerPays"), eur, 140),
    );

    let mut final_fields = STObject::new(get_field_by_symbol("sfFinalFields"));
    final_fields.set_field_amount(
        get_field_by_symbol("sfTakerGets"),
        amount(get_field_by_symbol("sfTakerGets"), usd, 40),
    );
    final_fields.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        amount(get_field_by_symbol("sfTakerPays"), eur, 80),
    );
    final_fields.set_field_u32(get_field_by_symbol("sfSequence"), 9);
    if let Some(domain) = domain {
        final_fields.set_field_h256(get_field_by_symbol("sfDomainID"), domain);
    }

    let mut node = STObject::new(get_field_by_symbol("sfModifiedNode"));
    node.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::Offer.code(),
    );
    node.set_field_object(get_field_by_symbol("sfPreviousFields"), previous);
    node.set_field_object(get_field_by_symbol("sfFinalFields"), final_fields);

    let mut affected = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    affected.push_back(node);

    let mut object = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    object.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    object.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 1);
    object.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected);
    TxMeta::from_stobject(Uint256::from_array([0x41; 32]), 90, object)
}

#[derive(Clone)]
struct TestSource {
    current: Option<LedgerLookupLedger>,
    closed: Option<LedgerLookupLedger>,
    validated: Option<LedgerLookupLedger>,
    by_seq: BTreeMap<u32, LedgerLookupLedger>,
    by_hash: BTreeMap<Uint256, LedgerLookupLedger>,
    valid_ledger_index: u32,
    validated_age: Duration,
    snapshots: BTreeMap<u32, BookChangesLedger>,
}

impl LedgerLookupSource for TestSource {
    fn get_ledger_by_hash(&self, hash: Uint256) -> Option<LedgerLookupLedger> {
        self.by_hash.get(&hash).copied()
    }

    fn get_ledger_by_seq(&self, seq: u32) -> Option<LedgerLookupLedger> {
        self.by_seq.get(&seq).copied()
    }

    fn get_current_ledger(&self) -> Option<LedgerLookupLedger> {
        self.current
    }

    fn get_closed_ledger(&self) -> Option<LedgerLookupLedger> {
        self.closed
    }

    fn get_validated_ledger(&self) -> Option<LedgerLookupLedger> {
        self.validated
    }

    fn get_valid_ledger_index(&self) -> u32 {
        self.valid_ledger_index
    }

    fn get_validated_ledger_age(&self) -> Duration {
        self.validated_age
    }

    fn is_validated(&self, ledger: &LedgerLookupLedger) -> bool {
        self.validated == Some(*ledger)
    }
}

impl BookChangesSource for TestSource {
    fn book_changes_ledger(&self, ledger: LedgerLookupLedger) -> Option<BookChangesLedger> {
        self.snapshots.get(&ledger.seq).cloned()
    }
}

fn source() -> TestSource {
    let current = LedgerLookupLedger {
        hash: Uint256::from_array([0x11; 32]),
        seq: 200,
        open: true,
    };
    let validated = LedgerLookupLedger {
        hash: Uint256::from_array([0x22; 32]),
        seq: 199,
        open: false,
    };

    let txn = BookChangesTransaction {
        txn: payment_tx(1),
        meta: offer_change_meta(Some(Uint256::from_array([0x55; 32]))),
    };

    TestSource {
        current: Some(current),
        closed: Some(validated),
        validated: Some(validated),
        by_seq: BTreeMap::from([(199, validated)]),
        by_hash: BTreeMap::from([(validated.hash, validated)]),
        valid_ledger_index: 200,
        validated_age: Duration::from_secs(30),
        snapshots: BTreeMap::from([
            (
                200,
                BookChangesLedger {
                    ledger_time: 777,
                    transactions: vec![txn.clone()],
                },
            ),
            (
                199,
                BookChangesLedger {
                    ledger_time: 666,
                    transactions: vec![txn],
                },
            ),
        ]),
    }
}

#[test]
fn book_changes_defaults_to_current_ledger_shape() {
    let source = source();
    let params = object([]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );

    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(200))
    );
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        result.get("type"),
        Some(&JsonValue::String("bookChanges".to_owned()))
    );
    assert_eq!(result.get("ledger_time"), Some(&JsonValue::Unsigned(777)));
}

#[test]
fn book_changes_preserves_domain_and_validated_flag() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );

    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
    let JsonValue::Array(changes) = result.get("changes").expect("changes should be present")
    else {
        panic!("changes must be an array");
    };
    assert_eq!(changes.len(), 1);
    let JsonValue::Object(change) = &changes[0] else {
        panic!("change entry must be an object");
    };
    assert_eq!(
        change.get("domain"),
        Some(&JsonValue::String(
            Uint256::from_array([0x55; 32]).to_string()
        ))
    );
}

#[test]
fn book_changes_change_entry_has_expected_fields() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Array(changes) = result.get("changes").expect("changes") else {
        panic!("changes must be an array");
    };
    assert_eq!(changes.len(), 1);

    let JsonValue::Object(change) = &changes[0] else {
        panic!("change must be an object");
    };
    // Each change should have currency_a, currency_b, volume_a, volume_b, high, low, open, close
    assert!(change.contains_key("currency_a"), "should have currency_a");
    assert!(change.contains_key("currency_b"), "should have currency_b");
    assert!(change.contains_key("volume_a"), "should have volume_a");
    assert!(change.contains_key("volume_b"), "should have volume_b");
    assert!(change.contains_key("high"), "should have high");
    assert!(change.contains_key("low"), "should have low");
    assert!(change.contains_key("open"), "should have open");
    assert!(change.contains_key("close"), "should have close");
}

#[test]
fn book_changes_ledger_not_found() {
    let source = source();
    let params = object([("ledger_index", JsonValue::Unsigned(999))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert!(result.contains_key("error"));
}

#[test]
fn book_changes_response_structure_validated() {
    let source = source();
    let params = object([("ledger_index", JsonValue::String("validated".to_owned()))]);

    let result = do_book_changes(
        &BookChangesRequest {
            params: &params,
            api_version: 2,
            role: RpcRole::Guest,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("validated"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        result.get("ledger_hash"),
        Some(&JsonValue::String(
            Uint256::from_array([0x22; 32]).to_string()
        ))
    );
    assert_eq!(result.get("ledger_index"), Some(&JsonValue::Unsigned(199)));
    assert_eq!(
        result.get("type"),
        Some(&JsonValue::String("bookChanges".to_owned()))
    );
    assert_eq!(result.get("ledger_time"), Some(&JsonValue::Unsigned(666)));
    assert!(result.contains_key("changes"));
}

fn memory_node_store(label: &str) -> app::SHAMapStoreNodeStore {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "quaxar-rpc-book-changes-{label}-{}-{unique}",
        std::process::id()
    ));
    let mut config = basics::basic_config::BasicConfig::new();
    config.set_legacy("database_path", dir.join("sql").to_string_lossy());
    let node_db = config.section_mut("node_db");
    node_db.set("type", "Memory");
    node_db.set("path", dir.join("node").to_string_lossy());
    app::bootstrap_shamap_store(
        &config,
        false,
        128,
        1,
        8,
        64,
        2,
        &nodestore::ManagerImp::new(),
        std::sync::Arc::new(nodestore::DummyScheduler) as std::sync::Arc<dyn nodestore::Scheduler>,
        std::sync::Arc::new(nodestore::NullJournal),
    )
    .expect("node store bootstrap")
    .node_store
}

/// A ledger whose transaction map holds one offer-changing transaction, with
/// the map's nodes in `node_store` and only its root in memory: the shape of
/// an older ledger loaded from disk.
fn store_backed_offer_ledger(node_store: &app::SHAMapStoreNodeStore) -> ledger::Ledger {
    use shamap::sync::{SHAMapType, SyncState, SyncTree};

    let seq = 90;
    let tx = payment_tx(1);
    let meta = offer_change_meta(None);
    let mut payload = protocol::Serializer::new(0);
    payload.add_vl(tx.get_serializer().data());
    payload.add_vl(meta.get_as_object().get_serializer().data());
    let mut tx_tree = shamap::mutation::MutableTree::new(seq);
    tx_tree
        .add_item(
            shamap::tree_node::SHAMapNodeType::TransactionMd,
            shamap::item::SHAMapItem::new(tx.get_transaction_id(), payload.data().to_vec()),
        )
        .expect("TransactionMd leaf should insert");
    let header = ledger::LedgerHeader {
        seq,
        close_time: 666,
        tx_hash: tx_tree.root().get_hash(),
        ..ledger::LedgerHeader::default()
    };
    let mut full = ledger::Ledger::from_maps(
        header.clone(),
        SyncTree::new_with_type(SHAMapType::State, true, seq),
        SyncTree::from_root_with_type(
            tx_tree.root(),
            SHAMapType::Transaction,
            true,
            seq,
            SyncState::Modifying,
        ),
    );
    let writer = node_store.clone();
    full.set_node_writer(std::sync::Arc::new(
        move |object_type: ledger::LedgerNodeObjectType,
              hash: Uint256,
              data: Vec<u8>,
              ledger_seq: u32| {
            let object_type = match object_type {
                ledger::LedgerNodeObjectType::AccountNode => nodestore::NodeObjectType::AccountNode,
                ledger::LedgerNodeObjectType::TransactionNode => {
                    nodestore::NodeObjectType::TransactionNode
                }
            };
            match &writer {
                app::SHAMapStoreNodeStore::Single(db) => db
                    .store(object_type, data, hash, ledger_seq)
                    .expect("test node store write should succeed"),
            }
        },
    ));
    full.flush_tx_map_to_store();

    let root = full.tx_map().root();
    let sparse_root = shamap::nodes::tree_node::SHAMapTreeNode::make_from_wire(
        &root.serialize_for_wire().expect("root should serialize"),
    )
    .expect("root should decode from wire shape")
    .expect("root wire must contain a node");
    sparse_root.set_hash(root.get_hash());
    let mut ledger = ledger::Ledger::from_maps(
        header,
        SyncTree::new_with_type(SHAMapType::State, true, seq),
        SyncTree::from_root_with_type(
            sparse_root,
            SHAMapType::Transaction,
            true,
            seq,
            SyncState::Immutable,
        ),
    );
    ledger.set_immutable(true);
    ledger
}

/// rippled `computeBookChanges` walks every transaction of the ledger. An
/// older ledger's transaction map is mostly on disk; quaxar walked only the
/// in-memory nodes, saw no transactions and answered with empty `changes`
/// (mainnet ledger 108548865, 2026-11-24). It must read through the node
/// store.
#[test]
fn book_changes_reads_transactions_through_the_node_store() {
    let node_store = memory_node_store("store-backed");
    let ledger = std::sync::Arc::new(store_backed_offer_ledger(&node_store));
    assert!(
        ledger.tx_snapshot().is_err(),
        "fixture: the transaction map must not be readable from memory alone"
    );
    let mut app = app::ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    app.attach_node_store(Some(node_store));
    app.note_validated_ledger_for_sync(std::sync::Arc::clone(&ledger));

    let source = rpc::ApplicationServerInfo::new(&app);
    let snapshot = source
        .book_changes_ledger(LedgerLookupLedger {
            hash: *ledger.header().hash.as_uint256(),
            seq: ledger.header().seq,
            open: false,
        })
        .expect("the ledger's transactions should be readable");
    assert_eq!(snapshot.ledger_time, 666);
    assert_eq!(snapshot.transactions.len(), 1, "the offer change must be seen");
}
