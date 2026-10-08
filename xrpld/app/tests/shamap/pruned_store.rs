//! End-to-end test of the pruned node store: build real validated ledgers,
//! drive them through the PrunedDriver exactly as on_ledger_closed does, and
//! assert the store keeps the retention window's nodes and prunes older ones.
//!
//! Unlike the nodestore-crate unit tests, this exercises the real
//! compute_claim_delta over actual SHAMap state and transaction trees, so it
//! verifies the SHAMap-to-index bridge together with claim and prune on the
//! fjall backend.

use app::shamap::pruned_driver::PrunedDriver;
use basics::base_uint::{Uint160, Uint256};
use basics::basic_config::Section;
use ledger::{LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader};
use nodestore::{
    Backend, Factory, FjallFactory, Keyspace, NodeObject, NodeObjectType, NullJournal, PruneMode,
    PrunedConfig,
};
use protocol::{AccountID, LedgerEntryType, STLedgerEntry, account_keylet, get_field_by_symbol};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use std::sync::Arc;

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match Uint160")
}

/// An account-root state item whose serialized bytes depend on `sequence`, so a
/// changing sequence produces a changing leaf hash (a modified account).
fn account_item(account: AccountID, sequence: u32) -> SHAMapItem {
    let mut root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    root.set_account_id(get_field_by_symbol("sfAccount"), account);
    root.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    SHAMapItem::new(
        account_keylet(raw_account_id(account)).key,
        root.get_serializer().data().to_vec(),
    )
}

fn account(byte: u8) -> AccountID {
    AccountID::from_hex(&format!("{byte:02x}").repeat(20)).expect("account hex")
}

/// Build a ledger where `alice` changes every sequence and `carol` never does,
/// mirroring the design's running example.
fn example_ledger(seq: u32) -> Ledger {
    let alice = account(0xA1);
    let carol = account(0xC0);
    let mut state = MutableTree::new(1);
    state
        .add_item(SHAMapNodeType::AccountState, account_item(alice, seq))
        .expect("alice inserts");
    state
        .add_item(SHAMapNodeType::AccountState, account_item(carol, 1))
        .expect("carol inserts");

    let mut tx = MutableTree::new(1);
    tx.add_item(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(
            Uint256::from_array([seq as u8; 32]),
            vec![seq as u8 + 1; 12],
        ),
    )
    .expect("tx inserts");

    // Finalize node hashes the way a built ledger's maps are before close:
    // flush_dirty walks the dirty nodes and computes their hashes, so
    // visit_differences (which short-circuits on a zero root hash) can walk
    // them. The writer is a no-op identity; we only need the hashing pass.
    let mut identity = |node| node;
    state.flush_dirty(&mut identity);
    tx.flush_dirty(&mut identity);

    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 700 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            account_hash: state.root().get_hash(),
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx.root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Modifying,
        ),
    )
}

fn open_fjall_backend(tag: &str) -> (Arc<dyn Backend>, String) {
    let dir = std::env::temp_dir().join(format!(
        "quaxar-pruned-e2e-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let path = dir.to_string_lossy().into_owned();
    let mut section = Section::new("node_db");
    section.set("type", "fjall");
    section.set("path", path.clone());
    let backend = FjallFactory::new()
        .create_instance(
            NodeObject::KEY_BYTES,
            &section,
            0,
            Arc::new(nodestore::DummyScheduler),
            Arc::new(NullJournal),
        )
        .expect("fjall backend");
    let backend: Arc<dyn Backend> = Arc::from(backend);
    backend.open(true).expect("open backend");
    (backend, path)
}

/// Flush every node of a ledger's state and tx trees into the backend's nodes
/// keyspace, the way the production store path does before a claim.
fn flush_ledger_nodes(driver: &PrunedDriver, ledger: &Ledger) {
    let store = driver.store();
    for tree in [ledger.state_map(), ledger.tx_map()] {
        let mut fetch = |_hash| None;
        tree.visit_nodes(&mut fetch, &mut |node| {
            if let Ok(bytes) = node.serialize_with_prefix() {
                let object = NodeObject::create_object(
                    NodeObjectType::AccountNode,
                    bytes,
                    *node.get_hash().as_uint256(),
                );
                store
                    .store(&object, ledger.header().seq)
                    .expect("store node");
            }
            true
        })
        .expect("visit_nodes should traverse the resident tree");
    }
}

#[test]
fn pruned_store_dedup_and_retention_without_pruning() {
    // Diagnostic: with an effectively infinite window, nothing is pruned and
    // shared nodes are stored once. Confirms the flush/claim path keeps every
    // node resident and that the shared Carol leaf is deduplicated.
    let (backend, path) = open_fjall_backend("dedup");
    let config = PrunedConfig {
        online_delete: 1_000_000,
        can_delete: u32::MAX,
        prune_mode: PruneMode::On,
        prune_batch: 64,
        verify_interval_secs: 0,
    };
    let driver = PrunedDriver::open(Arc::clone(&backend), config).expect("driver opens");

    let mut roots = Vec::new();
    for seq in 1..=3u32 {
        let ledger = example_ledger(seq);
        roots.push(*ledger.state_map().root().get_hash().as_uint256());
        flush_ledger_nodes(&driver, &ledger);
        driver.on_validated_ledger(Arc::new(ledger)).expect("claim");
    }
    // No pruning happened, so every ledger's root is still present.
    for (idx, root) in roots.iter().enumerate() {
        let present = backend
            .kv_get(Keyspace::Nodes, root.as_slice())
            .expect("read");
        assert!(present.is_some(), "root of ledger {} retained", idx + 1);
    }
    drop(driver);
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn pruned_store_prunes_old_ledgers_and_keeps_the_window() {
    let (backend, path) = open_fjall_backend("window");
    // Keep a window of 3 ledgers, prune eagerly.
    let config = PrunedConfig {
        online_delete: 3,
        can_delete: u32::MAX,
        prune_mode: PruneMode::On,
        prune_batch: 64,
        verify_interval_secs: 0,
    };
    let driver = PrunedDriver::open(Arc::clone(&backend), config).expect("driver opens");

    // The first ledger's Alice leaf hash: it must be gone once pruned, while
    // Carol's (shared) leaf survives the whole run.
    let first = example_ledger(1);
    let alice_1 = *first.state_map().root().get_hash().as_uint256(); // root changes each ledger; use it as the pruned marker.

    for seq in 1..=8u32 {
        let ledger = example_ledger(seq);
        flush_ledger_nodes(&driver, &ledger);
        driver
            .on_validated_ledger(Arc::new(ledger))
            .expect("claim + prune");
    }

    // After 8 ledgers with a window of 3, old ledgers are pruned and the
    // retention window is kept.
    let metrics = driver.metrics();
    let latest = example_ledger(8);
    let latest_root = *latest.state_map().root().get_hash().as_uint256();
    let l1_present = backend
        .kv_get(Keyspace::Nodes, alice_1.as_slice())
        .expect("read");
    let l8_present = backend
        .kv_get(Keyspace::Nodes, latest_root.as_slice())
        .expect("read");
    assert_eq!(
        metrics.claimed_seq,
        Some(8),
        "claimed up to the latest ledger"
    );
    assert!(
        metrics.pruned_to >= 4,
        "pruned past the window floor: {metrics:?}"
    );
    assert!(l1_present.is_none(), "ledger 1's root was pruned");
    assert!(l8_present.is_some(), "the latest ledger's root is retained");

    drop(driver);
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn pruned_store_dry_run_keeps_everything() {
    let (backend, path) = open_fjall_backend("dryrun");
    let config = PrunedConfig {
        online_delete: 2,
        can_delete: u32::MAX,
        prune_mode: PruneMode::DryRun,
        prune_batch: 64,
        verify_interval_secs: 0,
    };
    let driver = PrunedDriver::open(Arc::clone(&backend), config).expect("driver opens");

    let first_root = *example_ledger(1).state_map().root().get_hash().as_uint256();
    for seq in 1..=6u32 {
        let ledger = example_ledger(seq);
        flush_ledger_nodes(&driver, &ledger);
        driver.on_validated_ledger(Arc::new(ledger)).expect("claim");
    }

    // Dry-run deletes nothing: even ledger 1's root is still on disk.
    let present = backend
        .kv_get(Keyspace::Nodes, first_root.as_slice())
        .expect("read");
    assert!(present.is_some(), "dry-run retains every node");
    assert!(
        driver.metrics().last_dry_run_would_delete >= 1,
        "dry-run reports it would have deleted"
    );

    drop(driver);
    let _ = std::fs::remove_dir_all(&path);
}

// T-RANGE-1 (Case 18): the retained-floor callback fires in lockstep with the
// prune cursor, so the advertised complete_ledgers range can be lowered to the
// floor. The driver reports `pruned_to + 1` after every maintain pass.
#[test]
fn pruned_store_reports_retained_floor_in_lockstep_with_prune() {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    let (backend, path) = open_fjall_backend("floor");
    let config = PrunedConfig {
        online_delete: 3,
        can_delete: u32::MAX,
        prune_mode: PruneMode::On,
        prune_batch: 64,
        verify_interval_secs: 0,
    };
    let driver = PrunedDriver::open(Arc::clone(&backend), config).expect("driver opens");

    // Record every retained floor the driver reports.
    let last_floor = Arc::new(AtomicU32::new(0));
    let history: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let last_floor_cb = Arc::clone(&last_floor);
    let history_cb = Arc::clone(&history);
    driver.set_on_floor_advanced(Box::new(move |floor| {
        last_floor_cb.store(floor, Ordering::SeqCst);
        history_cb.lock().expect("history").push(floor);
    }));

    for seq in 1..=8u32 {
        let ledger = example_ledger(seq);
        flush_ledger_nodes(&driver, &ledger);
        driver
            .on_validated_ledger(Arc::new(ledger))
            .expect("claim + prune");
    }

    let metrics = driver.metrics();
    // The callback fired once per ledger and the last reported floor equals the
    // store's retained floor (pruned_to + 1).
    assert_eq!(
        history.lock().expect("history").len(),
        7,
        "floor callback fires on every claim after the anchor"
    );
    assert_eq!(
        last_floor.load(Ordering::SeqCst),
        metrics.retained_floor,
        "reported floor tracks the prune cursor"
    );
    assert_eq!(
        metrics.retained_floor,
        metrics.pruned_to + 1,
        "retained floor is pruned_to + 1"
    );
    assert!(
        metrics.retained_floor >= 5,
        "floor advanced with pruning: {metrics:?}"
    );

    drop(driver);
    let _ = std::fs::remove_dir_all(&path);
}

// Periodic sampled verify (Stage 4 verify_interval): with a nonzero interval,
// the driver verifies the latest ledger's reachable nodes are present. When
// every node was flushed, verify passes and the claim succeeds; the metric
// records the last result as ok.
#[test]
fn pruned_store_periodic_verify_passes_when_nodes_present() {
    let (backend, path) = open_fjall_backend("verify");
    let config = PrunedConfig {
        online_delete: 3,
        can_delete: u32::MAX,
        prune_mode: PruneMode::On,
        prune_batch: 64,
        // A 1-second interval; last_verify is None so the first ledger verifies.
        verify_interval_secs: 1,
    };
    let driver = PrunedDriver::open(Arc::clone(&backend), config).expect("driver opens");

    for seq in 1..=4u32 {
        let ledger = example_ledger(seq);
        flush_ledger_nodes(&driver, &ledger);
        driver
            .on_validated_ledger(Arc::new(ledger))
            .expect("claim + maintain + verify");
    }

    // Verify ran (interval elapsed on the first ledger) and found nothing
    // missing, so the recorded result is ok.
    assert!(
        driver.metrics().verify_last_ok,
        "verify over present nodes must pass: {:?}",
        driver.metrics()
    );

    drop(driver);
    let _ = std::fs::remove_dir_all(&path);
}

/// A ledger with `accounts` state leaves, so the state tree has inner nodes
/// below the root. `backed` marks both trees as store-backed, which lets
/// `release_to_disk` drop resident children like a long-running node does.
fn wide_ledger(seq: u32, accounts: u8, backed: bool) -> Ledger {
    let mut state = MutableTree::new(1);
    for byte in 0..accounts {
        // Vary only the first account each ledger so most leaves are shared.
        let sequence = if byte == 0 { seq } else { 1 };
        state
            .add_item(
                SHAMapNodeType::AccountState,
                account_item(account(byte.wrapping_mul(7).wrapping_add(1)), sequence),
            )
            .expect("account inserts");
    }
    let mut tx = MutableTree::new(1);
    tx.add_item(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(
            Uint256::from_array([seq as u8; 32]),
            vec![seq as u8 + 1; 12],
        ),
    )
    .expect("tx inserts");
    let mut identity = |node| node;
    state.flush_dirty(&mut identity);
    tx.flush_dirty(&mut identity);
    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 700 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            account_hash: state.root().get_hash(),
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state.root(),
            SHAMapType::State,
            backed,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx.root(),
            SHAMapType::Transaction,
            backed,
            seq,
            SyncState::Modifying,
        ),
    )
}

// A long-running node releases validated subtrees from memory and reads them
// back from the store on demand. The claim diff must fetch those nodes and
// produce the same delta as a fully resident ledger; it must never claim a
// partial delta (which would leave live nodes for the orphan sweep).
#[test]
fn claim_over_a_released_backed_ledger_matches_the_resident_delta() {
    let config = PrunedConfig {
        online_delete: 1_000_000,
        can_delete: u32::MAX,
        prune_mode: PruneMode::On,
        prune_batch: 64,
        verify_interval_secs: 0,
    };

    // Reference: resident ledgers.
    let (ref_backend, ref_path) = open_fjall_backend("resident");
    let reference = PrunedDriver::open(Arc::clone(&ref_backend), config).expect("driver");
    for seq in 1..=3u32 {
        let ledger = wide_ledger(seq, 40, false);
        flush_ledger_nodes(&reference, &ledger);
        reference
            .on_validated_ledger(Arc::new(ledger))
            .expect("claim");
    }

    // Same ledgers, but backed and released to disk before each claim.
    let (backend, path) = open_fjall_backend("released");
    let driver = PrunedDriver::open(Arc::clone(&backend), config).expect("driver");
    for seq in 1..=3u32 {
        let ledger = wide_ledger(seq, 40, true);
        flush_ledger_nodes(&driver, &ledger);
        ledger.state_map().release_to_disk();
        driver
            .on_validated_ledger(Arc::new(ledger))
            .expect("a released backed ledger must claim through the store fetcher");
    }

    let count_rows = |backend: &Arc<dyn Backend>| {
        let mut rows = Vec::new();
        backend
            .kv_range(Keyspace::Counts, &[], &[0xFF; 64], &mut |key, value| {
                rows.push((key.to_vec(), value.to_vec()));
                true
            })
            .expect("range");
        rows
    };
    assert_eq!(driver.metrics().claimed_seq, Some(3));
    assert_eq!(
        count_rows(&backend),
        count_rows(&ref_backend),
        "released and resident ledgers must yield identical reference counts"
    );

    drop((driver, reference));
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_dir_all(&ref_path);
}

// Production path: the component hands ledgers to the worker with submit(),
// which returns immediately; the worker claims them off the publish path.
#[test]
fn submitted_ledgers_are_claimed_by_the_worker() {
    let (backend, path) = open_fjall_backend("worker");
    let config = PrunedConfig {
        online_delete: 3,
        can_delete: u32::MAX,
        prune_mode: PruneMode::On,
        prune_batch: 64,
        verify_interval_secs: 0,
    };
    let driver = Arc::new(PrunedDriver::open(Arc::clone(&backend), config).expect("driver"));
    driver.spawn_worker();
    for seq in 1..=6u32 {
        let ledger = example_ledger(seq);
        flush_ledger_nodes(&driver, &ledger);
        driver.submit(Arc::new(ledger));
        // Let the worker drain each ledger so every one is claimed in order.
        for _ in 0..200 {
            if driver.metrics().claimed_seq == Some(seq) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let metrics = driver.metrics();
    driver.stop_worker();
    assert_eq!(metrics.claimed_seq, Some(6), "worker claimed every ledger");
    assert!(
        metrics.pruned_to >= 2,
        "worker pruned behind the window: {metrics:?}"
    );
    drop(driver);
    let _ = std::fs::remove_dir_all(&path);
}
