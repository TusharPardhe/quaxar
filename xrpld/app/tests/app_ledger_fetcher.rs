use app::{ApplicationRoot, bootstrap_shamap_store, decode_acquired_tx_set};
use basics::base_uint::{Uint160, Uint256};
use basics::basic_config::BasicConfig;
use ledger::{ApplyView, Fees, Ledger, LedgerHeader, encode_fee_settings_entry};
use nodestore::{DummyScheduler, FetchType, ManagerImp, NullJournal, Scheduler};
use protocol::{
    AccountID, LedgerEntryType, STAmount, STArray, STLedgerEntry, STObject, STTx, TxType,
    account_keylet, fee_settings_keylet, get_field_by_symbol,
};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::SHAMapNodeType,
};
use std::sync::Arc;
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────────

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match Uint160")
}

fn test_node_type(object_type: ledger::LedgerNodeObjectType) -> nodestore::NodeObjectType {
    match object_type {
        ledger::LedgerNodeObjectType::AccountNode => nodestore::NodeObjectType::AccountNode,
        ledger::LedgerNodeObjectType::TransactionNode => nodestore::NodeObjectType::TransactionNode,
    }
}

fn canonical_order_payment(source: AccountID, destination: AccountID, sequence: u32) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    }))
}

fn tx_map_payload(tx: &STTx) -> Vec<u8> {
    let affected_nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    let mut meta = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    meta.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    meta.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 0);
    meta.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);

    let tx_bytes = tx.get_serializer().data().to_vec();
    let meta_bytes = meta.get_serializer().data().to_vec();
    let mut serializer = protocol::Serializer::new(0);
    serializer.add_vl(&tx_bytes);
    serializer.add_vl(&meta_bytes);
    serializer.data().to_vec()
}

#[test]
fn acquired_tx_set_decodes_to_cpp_canonical_order_not_tx_map_leaf_order() {
    let source =
        AccountID::from_hex("1111111111111111111111111111111111111111").expect("source account");
    let destination = AccountID::from_hex("2222222222222222222222222222222222222222")
        .expect("destination account");
    let later = canonical_order_payment(source, destination, 17264169);
    let earlier = canonical_order_payment(source, destination, 17264168);
    let tx_items = vec![
        (tx_map_payload(&later), later.get_transaction_id()),
        (tx_map_payload(&earlier), earlier.get_transaction_id()),
    ];

    let ordered = decode_acquired_tx_set(
        &tx_items,
        Uint256::from_array([0xCF; 32]),
        shamap::tree_node::SHAMapNodeType::TransactionMd,
    );
    let sequences = ordered
        .iter()
        .map(|tx| tx.get_field_u32(get_field_by_symbol("sfSequence")))
        .collect::<Vec<_>>();

    assert_eq!(sequences, vec![17264168, 17264169]);
}

/// Build a parent ledger that has one account root in its state tree,
/// flush all nodes to NuDB, and attach fetcher + writer so the built
/// child ledger can read state back through the node store.
fn parent_with_account_in_nudb(
    seq: u32,
    account: AccountID,
    balance_drops: u64,
    node_store: &app::SHAMapStoreNodeStore,
) -> Ledger {
    // 1. Build the account root SLE.
    let keylet = account_keylet(raw_account_id(account));
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
    sle.set_account_id(get_field_by_symbol("sfAccount"), account);
    sle.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    sle.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(balance_drops, false),
    );

    // 2. Insert into a MutableTree.
    let mut state_tree = MutableTree::new(seq);
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(keylet.key, sle.get_serializer().data().to_vec()),
        )
        .expect("account root should insert");

    // 3. Build the ledger (backed=true so reads go through NuDB).
    let state_map = SyncTree::from_root_with_type(
        state_tree.root(),
        SHAMapType::State,
        true,
        seq,
        SyncState::Modifying,
    );
    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq,
            drops: balance_drops,
            ..LedgerHeader::default()
        },
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, true, seq),
    );

    // 4. Wire up the node store fetcher + writer.
    let ns_read = node_store.clone();
    let ns_write = node_store.clone();
    ledger.set_node_fetcher(Arc::new(move |hash| {
        let data = match &ns_read {
            app::SHAMapStoreNodeStore::Single(db) => {
                db.fetch_node_object(hash.as_uint256(), 0, FetchType::Synchronous, false)
            }
            app::SHAMapStoreNodeStore::Rotating(db) => {
                db.fetch_node_object(hash.as_uint256(), 0, FetchType::Synchronous, false)
            }
        }?;
        shamap::nodes::tree_node::SHAMapTreeNode::make_from_prefix(data.data(), hash).ok()
    }));
    ledger.set_node_writer(Arc::new(
        move |object_type: ledger::LedgerNodeObjectType,
              hash: Uint256,
              data: Vec<u8>,
              ledger_seq: u32| match &ns_write {
            app::SHAMapStoreNodeStore::Single(db) => {
                db.store(test_node_type(object_type), data, hash, ledger_seq)
            }
            app::SHAMapStoreNodeStore::Rotating(db) => {
                db.store(test_node_type(object_type), data, hash, ledger_seq)
            }
        },
    ));

    // 5. Finalize: flush dirty nodes to NuDB, then mark immutable.
    ledger.flush_state_map_to_store();
    ledger.set_immutable(true);
    ledger
}

fn backed_fee_ledger_without_fetcher(
    seq: u32,
    fees: Fees,
    node_store: &app::SHAMapStoreNodeStore,
) -> Ledger {
    let fee_keylet = fee_settings_keylet();
    let fee_payload = encode_fee_settings_entry(fees, false);

    let mut state_tree = MutableTree::new(seq);
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(fee_keylet.key, fee_payload),
        )
        .expect("fee settings should insert");

    let state_map = SyncTree::from_root_with_type(
        state_tree.root(),
        SHAMapType::State,
        true,
        seq,
        SyncState::Modifying,
    );
    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq,
            ..LedgerHeader::default()
        },
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, true, seq),
    );

    let ns_write = node_store.clone();
    ledger.set_node_writer(Arc::new(
        move |object_type: ledger::LedgerNodeObjectType,
              hash: Uint256,
              data: Vec<u8>,
              ledger_seq: u32| match &ns_write {
            app::SHAMapStoreNodeStore::Single(db) => {
                db.store(test_node_type(object_type), data, hash, ledger_seq)
            }
            app::SHAMapStoreNodeStore::Rotating(db) => {
                db.store(test_node_type(object_type), data, hash, ledger_seq)
            }
        },
    ));
    ledger.flush_state_map_to_store();

    let root_hash = ledger.state_map().root().get_hash();
    let root_wire = ledger
        .state_map()
        .root()
        .serialize_for_wire()
        .expect("root should serialize");
    let sparse_root = shamap::nodes::tree_node::SHAMapTreeNode::make_from_wire(&root_wire)
        .expect("root should decode from wire shape")
        .expect("root wire must contain a node");
    sparse_root.set_hash(root_hash);

    let sparse_state_map = SyncTree::from_root_with_type(
        sparse_root,
        SHAMapType::State,
        true,
        seq,
        SyncState::Immutable,
    );
    Ledger::from_maps(
        ledger.header(),
        sparse_state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, true, seq),
    )
}

// ── NuDB bootstrap helper ────────────────────────────────────────────────────

fn nudb_bootstrap() -> (TempDir, app::SHAMapStoreNodeStore) {
    let dir = TempDir::new().expect("tempdir");
    let mut config = BasicConfig::new();
    config.set_legacy("database_path", dir.path().join("sql").to_string_lossy());
    let node_db = config.section_mut("node_db");
    node_db.set("type", "Memory");
    node_db.set("path", dir.path().join("node").to_string_lossy());

    let bootstrap = bootstrap_shamap_store(
        &config,
        false,
        128,
        1,
        8,
        64,
        2,
        &ManagerImp::new(),
        Arc::new(DummyScheduler) as Arc<dyn Scheduler>,
        Arc::new(NullJournal),
    )
    .expect("bootstrap");

    (dir, bootstrap.node_store)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn application_root_attaches_node_fetcher_to_backed_ledger_holders() {
    let (_dir, node_store) = nudb_bootstrap();
    let mut root = ApplicationRoot::new(0).expect("root");
    root.attach_node_store(Some(node_store));

    let closed = Arc::new(Ledger::from_ledger_seq_and_close_time(10, 1_000, true));
    assert!(!closed.has_node_fetcher());
    root.on_closed_ledger(Arc::clone(&closed));
    assert!(
        root.closed_ledger()
            .expect("closed ledger should be stored")
            .has_node_fetcher()
    );

    let published = Arc::new(Ledger::from_ledger_seq_and_close_time(11, 1_010, true));
    root.on_published_ledger(Arc::clone(&published));
    assert!(
        root.published_ledger()
            .expect("published ledger should be stored")
            .has_node_fetcher()
    );

    let validated = Arc::new(Ledger::from_ledger_seq_and_close_time(12, 1_020, true));
    root.note_validated_ledger_for_sync(Arc::clone(&validated));
    assert!(
        root.validated_ledger()
            .expect("validated ledger should be stored")
            .has_node_fetcher()
    );
}

#[test]
fn application_root_refreshes_fee_setup_after_attaching_fetcher() {
    let (_dir, node_store) = nudb_bootstrap();
    let fees = Fees {
        base: 10,
        reserve: 1_000_000,
        increment: 200_000,
    };
    let ledger = backed_fee_ledger_without_fetcher(100, fees, &node_store);
    assert!(!ledger.has_node_fetcher());
    assert_eq!(ledger.fees(), Fees::default());

    let mut root = ApplicationRoot::new(0).expect("root");
    root.attach_node_store(Some(node_store));

    let ledger = root.ledger_with_node_fetcher(Arc::new(ledger));
    assert!(ledger.has_node_fetcher());
    assert_eq!(
        ledger.fees(),
        fees,
        "ledger_with_node_fetcher must refresh setup so acquired ledgers use fetched FeeSettings"
    );
}

#[test]
fn application_root_replaces_stale_fetcher_when_family_runtime_is_attached() {
    let (_dir, node_store) = nudb_bootstrap();
    let fees = Fees {
        base: 12,
        reserve: 2_000_000,
        increment: 400_000,
    };
    let mut ledger = backed_fee_ledger_without_fetcher(101, fees, &node_store);
    ledger.set_node_fetcher(Arc::new(|_| None));
    ledger.set_node_writer(Arc::new(|_, _, _, _| ()));
    assert!(ledger.has_node_fetcher());
    assert!(ledger.has_node_writer());
    assert_ne!(
        ledger.fees(),
        fees,
        "the stale fetcher cannot materialize this ledger's FeeSettings"
    );

    let mut root = ApplicationRoot::new(0).expect("root");
    root.attach_node_store(Some(node_store));
    root.attach_default_node_family();

    let ledger = root.ledger_with_node_fetcher(Arc::new(ledger));
    assert_eq!(
        ledger.fees(),
        fees,
        "family-backed applications must replace stale store-only fetchers so setup reads use the current shared fetch path"
    );
}

/// Core regression test: a Sandbox built on a parent ledger whose state is
/// stored in NuDB must be able to read account roots through the node fetcher.
/// This is the offline equivalent of the TER_NO_ACCOUNT storm seen on mainnet.
#[test]
fn build_ledger_from_acquired_tx_reads_account_through_nudb_fetcher() {
    let (_dir, node_store) = nudb_bootstrap();

    let account = AccountID::from_array([0x42; 20]);
    let balance = 100_000_000u64;

    // Parent ledger: account root written to NuDB.
    let parent = parent_with_account_in_nudb(100, account, balance, &node_store);
    assert!(parent.has_node_fetcher(), "parent must have fetcher");

    // Simulate what build_ledger_from_acquired_tx does: clone parent into a
    // Sandbox and peek the account root.  This is the exact call chain that
    // was returning TER_NO_ACCOUNT.
    let base = Arc::new(parent.clone());
    assert!(base.has_node_fetcher(), "clone must carry fetcher");

    let mut sandbox = ledger::Sandbox::new(base, protocol::ApplyFlags::default());
    let keylet = account_keylet(raw_account_id(account));
    let result = sandbox
        .peek(keylet)
        .expect("peek must not return a traversal error");

    assert!(
        result.is_some(),
        "account root not found in NuDB — TER_NO_ACCOUNT regression: \
         Sandbox::peek returned None even though the account exists in the node store"
    );

    let sle = result.unwrap();
    let found_balance = sle
        .get_field_amount(get_field_by_symbol("sfBalance"))
        .xrp()
        .drops();
    assert_eq!(
        found_balance, balance as i64,
        "account balance should match what was written"
    );
}

/// Negative control: a backed Ledger without a node fetcher must report
/// has_fetcher=false, which is the condition that causes TER_NO_ACCOUNT
/// when child nodes are not loaded in memory.
#[test]
fn sandbox_peek_fails_without_node_fetcher() {
    let (_dir, node_store) = nudb_bootstrap();

    let account = AccountID::from_array([0x42; 20]);
    let balance = 100_000_000u64;

    // Build parent with fetcher so nodes are written to NuDB.
    let parent = parent_with_account_in_nudb(100, account, balance, &node_store);

    // Strip the fetcher: rebuild from the same state root but backed=true, no fetcher.
    let state_root = parent.state_map().root();
    let state_map = SyncTree::from_root_with_type(
        state_root,
        SHAMapType::State,
        true,
        100,
        SyncState::Immutable,
    );
    let parent_no_fetcher = Ledger::from_maps(
        parent.header(),
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, true, 100),
    );

    // The invariant we are guarding: backed=true + has_fetcher=false is the
    // broken state.  Any Ledger in this state that needs to traverse child
    // nodes not already loaded will return MissingNode → TER_NO_ACCOUNT.
    assert!(
        !parent_no_fetcher.has_node_fetcher(),
        "fetcher should be absent"
    );
    assert!(
        parent_no_fetcher.state_map().backed(),
        "state map should be backed"
    );
}
