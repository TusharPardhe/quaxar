use app::{AppLedgerMaster, AppLoadedLedgerRuntime, SqliteSHAMapStoreRelational};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use ledger::{Ledger, LedgerHeader, LedgerMasterConfig, calculate_ledger_hash};
use overlay::TmGetLedger;
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use xrpld_core::{DatabaseCon, LEDGER_DB_INIT};

fn sample_loaded_ledger(seq: u32) -> Ledger {
    let mut state_tree = MutableTree::new(seq);
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(Uint256::from_u64(11), vec![1; 48]),
        )
        .expect("state leaf should insert");

    let mut tx_tree = MutableTree::new(seq);
    tx_tree
        .add_item(
            SHAMapNodeType::TransactionNm,
            SHAMapItem::new(Uint256::from_u64(22), vec![2; 32]),
        )
        .expect("tx leaf should insert");

    let state_root = state_tree.root();
    let tx_root = tx_tree.root();
    let header = LedgerHeader {
        seq,
        drops: 100,
        parent_hash: SHAMapHash::new(Uint256::from_u64(9)),
        account_hash: state_root.get_hash(),
        tx_hash: tx_root.get_hash(),
        close_time: 500 + seq,
        parent_close_time: 499 + seq,
        close_time_resolution: 10,
        close_flags: 0,
        ..LedgerHeader::default()
    };
    let header = LedgerHeader {
        hash: calculate_ledger_hash(&header),
        ..header
    };

    Ledger::from_maps(
        header,
        SyncTree::from_root_with_type(
            state_root,
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx_root,
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Modifying,
        ),
    )
}

fn sample_immutable_closed_ledger(seq: u32) -> Ledger {
    let header = LedgerHeader {
        seq,
        drops: 100,
        parent_hash: SHAMapHash::new(Uint256::from_u64(9)),
        account_hash: SHAMapHash::new(Uint256::from_u64(10)),
        tx_hash: SHAMapHash::new(Uint256::from_u64(11)),
        close_time: 500 + seq,
        parent_close_time: 499 + seq,
        close_time_resolution: 10,
        close_flags: 0,
        ..LedgerHeader::default()
    };
    Ledger::from_header_hashes(LedgerHeader {
        hash: calculate_ledger_hash(&header),
        ..header
    })
}

fn insert_sql_ledger(db: &DatabaseCon, seq: u32, hash: SHAMapHash, parent_hash: SHAMapHash) {
    db.get_session()
        .execute(
            "INSERT INTO Ledgers (LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                hash.as_uint256().to_string(),
                i64::from(seq),
                parent_hash.as_uint256().to_string(),
                100_i64,
                i64::from(500 + seq),
                i64::from(499 + seq),
                10_i64,
                0_i64,
                SHAMapHash::new(Uint256::from_u64(u64::from(seq) + 1)).as_uint256().to_string(),
                SHAMapHash::new(Uint256::from_u64(u64::from(seq) + 2)).as_uint256().to_string(),
            ],
        )
        .expect("ledger insert should succeed");
}

#[test]
fn runtime_resolves_closed_ledger_by_hash_and_seq_without_storage() {
    let ledger = Arc::new(sample_immutable_closed_ledger(77));
    let ledger_master = Arc::new(AppLedgerMaster::new(
        MonotonicClock::default(),
        LedgerMasterConfig::default(),
    ));
    ledger_master.set_closed_ledger(Arc::clone(&ledger));
    let runtime = AppLoadedLedgerRuntime::with_ledger_master(ledger_master);

    let by_hash = runtime
        .resolve_request_ledger(&TmGetLedger {
            itype: 0,
            ltype: None,
            ledger_hash: Some(ledger.header().hash.as_uint256().data().to_vec()),
            ledger_seq: Some(ledger.header().seq),
            node_i_ds: Vec::new(),
            request_cookie: None,
            query_type: None,
            query_depth: None,
        })
        .expect("hash lookup should not error");
    assert_eq!(
        by_hash.as_ref().map(|ledger| ledger.header().hash),
        Some(ledger.header().hash)
    );

    let by_seq = runtime
        .resolve_request_ledger(&TmGetLedger {
            itype: 0,
            ltype: None,
            ledger_hash: None,
            ledger_seq: Some(ledger.header().seq),
            node_i_ds: Vec::new(),
            request_cookie: None,
            query_type: None,
            query_depth: None,
        })
        .expect("sequence lookup should not error");
    assert_eq!(
        by_seq.as_ref().map(|ledger| ledger.header().seq),
        Some(ledger.header().seq)
    );
}

#[test]
fn runtime_rejects_hash_lookup_when_requested_sequence_mismatches() {
    let ledger = Arc::new(sample_immutable_closed_ledger(88));
    let ledger_master = Arc::new(AppLedgerMaster::new(
        MonotonicClock::default(),
        LedgerMasterConfig::default(),
    ));
    ledger_master.set_closed_ledger(Arc::clone(&ledger));
    let runtime = AppLoadedLedgerRuntime::with_ledger_master(ledger_master);

    let resolved = runtime
        .resolve_request_ledger(&TmGetLedger {
            itype: 0,
            ltype: None,
            ledger_hash: Some(ledger.header().hash.as_uint256().data().to_vec()),
            ledger_seq: Some(ledger.header().seq + 1),
            node_i_ds: Vec::new(),
            request_cookie: None,
            query_type: None,
            query_depth: None,
        })
        .expect("lookup should not error");
    assert!(resolved.is_none());
}

#[test]
fn runtime_builds_base_and_node_replies_from_loaded_ledger() {
    let ledger = sample_loaded_ledger(99);
    let runtime = AppLoadedLedgerRuntime::with_ledger_master(Arc::new(AppLedgerMaster::new(
        MonotonicClock::default(),
        LedgerMasterConfig::default(),
    )));

    let base_nodes = runtime.build_base_reply_nodes(&ledger);
    assert_eq!(base_nodes.len(), 3);
    assert!(base_nodes.iter().all(|node| node.nodeid.is_none()));

    let root_node_id = shamap::node_id::SHAMapNodeId::default().get_raw_string();
    let tx_nodes = runtime
        .build_shamap_reply_nodes(
            &ledger,
            &TmGetLedger {
                itype: 1,
                ltype: None,
                ledger_hash: Some(ledger.header().hash.as_uint256().data().to_vec()),
                ledger_seq: Some(ledger.header().seq),
                node_i_ds: vec![root_node_id.clone()],
                request_cookie: None,
                query_type: None,
                query_depth: Some(1),
            },
            false,
        )
        .expect("tx map reply should build");
    assert!(!tx_nodes.is_empty());
    assert!(tx_nodes.iter().all(|node| node.nodeid.is_some()));

    let state_nodes = runtime
        .build_shamap_reply_nodes(
            &ledger,
            &TmGetLedger {
                itype: 2,
                ltype: None,
                ledger_hash: Some(ledger.header().hash.as_uint256().data().to_vec()),
                ledger_seq: Some(ledger.header().seq),
                node_i_ds: vec![root_node_id],
                request_cookie: None,
                query_type: None,
                query_depth: Some(1),
            },
            true,
        )
        .expect("state map reply should build");
    assert!(!state_nodes.is_empty());
    assert!(state_nodes.iter().all(|node| node.nodeid.is_some()));
}

#[test]
fn runtime_exposes_history_hash_queries_from_relational_storage() {
    let temp = TempDir::new().expect("tempdir");
    let ledger_db = Arc::new(
        DatabaseCon::new_at_path(temp.path(), "ledger.db", &[], LEDGER_DB_INIT).expect("ledger db"),
    );
    let relational = Arc::new(SqliteSHAMapStoreRelational::new(
        Arc::clone(&ledger_db),
        None,
        false,
        100,
        Duration::from_secs(0),
    ));

    let hash_9 = SHAMapHash::new(Uint256::from_u64(900));
    let hash_10 = SHAMapHash::new(Uint256::from_u64(1_000));
    let hash_11 = SHAMapHash::new(Uint256::from_u64(1_100));
    insert_sql_ledger(
        &ledger_db,
        9,
        hash_9,
        SHAMapHash::new(Uint256::from_u64(800)),
    );
    insert_sql_ledger(&ledger_db, 10, hash_10, hash_9);
    insert_sql_ledger(&ledger_db, 11, hash_11, hash_10);

    let runtime = AppLoadedLedgerRuntime::with_sources(
        Arc::new(AppLedgerMaster::new(
            MonotonicClock::default(),
            LedgerMasterConfig::default(),
        )),
        Some(relational),
        None,
    );

    assert_eq!(runtime.earliest_ledger_seq(), 9);
    assert_eq!(runtime.get_hash_by_index(10), Some(hash_10));

    let pairs = runtime.get_hash_pairs_by_index(9, 11);
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[1].0, 10);
    assert_eq!(pairs[1].1.ledger_hash, hash_10);
    assert_eq!(pairs[2].1.parent_hash, hash_10);
}
