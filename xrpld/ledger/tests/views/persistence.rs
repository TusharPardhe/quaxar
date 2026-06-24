use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::intrusive_pointer::{SharedIntrusive, make_shared_intrusive};
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::ManualClock;
use ledger::{
    Ledger, LedgerConfig, LedgerHeader, LedgerInfoProvider, LedgerPersistence,
    LedgerPersistenceJobType, LedgerPersistenceRuntime, NullLedgerJournal, calculate_ledger_hash,
    get_latest_ledger, load_by_hash, load_by_index, load_ledger_helper, pend_save_validated,
};
use shamap::family::{
    NullFullBelowCache, NullMissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher,
};
use shamap::item::SHAMapItem;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::{SHAMapNodeType, SHAMapTreeNode};
use shamap::tree_node_cache::TreeNodeCache;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use time::Duration;

fn sample_hash(fill: u8) -> SHAMapHash {
    SHAMapHash::new(Uint256::from_array([fill; 32]))
}

fn state_leaf(fill: u8) -> SharedIntrusive<SHAMapTreeNode> {
    make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::AccountState,
        SHAMapItem::new(Uint256::from_array([fill; 32]), vec![fill; 12]),
        0,
    ))
}

fn immutable_ledger(seq: u32, fill: u8) -> Arc<Ledger> {
    let root = state_leaf(fill);
    let mut header = LedgerHeader {
        seq,
        account_hash: root.get_hash(),
        parent_hash: sample_hash(fill.wrapping_add(1)),
        close_time: seq + 10,
        close_time_resolution: 30,
        ..LedgerHeader::default()
    };
    header.hash = calculate_ledger_hash(&header);
    let mut ledger = Ledger::from_maps(
        header,
        SyncTree::from_root_with_type(root, SHAMapType::State, true, seq, SyncState::Modifying),
        SyncTree::new_with_type(SHAMapType::Transaction, true, seq),
    );
    ledger.set_immutable(true);
    Arc::new(ledger)
}

#[derive(Debug, Default)]
struct RecordingFetcher {
    nodes: HashMap<SHAMapHash, SharedIntrusive<SHAMapTreeNode>>,
}

impl SHAMapNodeFetcher for RecordingFetcher {
    fn fetch_node(&self, hash: SHAMapHash) -> Option<SharedIntrusive<SHAMapTreeNode>> {
        self.nodes.get(&hash).cloned()
    }
}

fn family_with_nodes(
    label: &'static str,
    nodes: HashMap<SHAMapHash, SharedIntrusive<SHAMapTreeNode>>,
) -> SHAMapFamily<
    ManualClock,
    HardenedHashBuilder,
    NullFullBelowCache,
    RecordingFetcher,
    NullMissingNodeReporter,
> {
    SHAMapFamily::new(
        Arc::new(TreeNodeCache::new(
            label,
            8,
            Duration::seconds(1),
            ManualClock::new(0),
        )),
        NullFullBelowCache::new(1),
        RecordingFetcher { nodes },
        NullMissingNodeReporter,
    )
}

#[derive(Debug, Default)]
struct RecordingProvider {
    by_index: HashMap<u32, LedgerHeader>,
    by_hash: HashMap<SHAMapHash, LedgerHeader>,
    newest: Option<LedgerHeader>,
}

impl LedgerInfoProvider for RecordingProvider {
    fn get_ledger_info_by_index(&self, ledger_index: u32) -> Option<LedgerHeader> {
        self.by_index.get(&ledger_index).copied()
    }

    fn get_ledger_info_by_hash(&self, ledger_hash: SHAMapHash) -> Option<LedgerHeader> {
        self.by_hash.get(&ledger_hash).copied()
    }

    fn get_newest_ledger_info(&self) -> Option<LedgerHeader> {
        self.newest
    }
}

#[derive(Default)]
struct RecordingPersistenceRuntime {
    saved_hashes: Mutex<Vec<SHAMapHash>>,
    finished: Mutex<Vec<u32>>,
    pending: Mutex<Vec<u32>>,
    save_calls: Mutex<Vec<(u32, bool)>>,
    queued_jobs: Mutex<Vec<(LedgerPersistenceJobType, String)>>,
    allow_enqueue: bool,
}

impl LedgerPersistenceRuntime for RecordingPersistenceRuntime {
    fn mark_saved(&self, hash: SHAMapHash) -> bool {
        let mut hashes = self.saved_hashes.lock().expect("saved_hashes mutex");
        if hashes.contains(&hash) {
            return false;
        }
        hashes.push(hash);
        true
    }

    fn start_work(&self, seq: u32) -> bool {
        self.pending.lock().expect("pending mutex").push(seq);
        true
    }

    fn finish_work(&self, seq: u32) {
        self.finished.lock().expect("finished mutex").push(seq);
    }

    fn should_work(&self, _seq: u32, _is_synchronous: bool) -> bool {
        true
    }

    fn pending(&self, seq: u32) -> bool {
        self.pending.lock().expect("pending mutex").contains(&seq)
    }

    fn save_validated_ledger(&self, ledger: Arc<Ledger>, is_current: bool) -> bool {
        self.save_calls
            .lock()
            .expect("save_calls mutex")
            .push((ledger.header().seq, is_current));
        true
    }

    fn enqueue_job(
        &self,
        job_type: LedgerPersistenceJobType,
        job_name: String,
        job: ledger::LedgerPersistenceJob,
    ) -> bool {
        self.queued_jobs
            .lock()
            .expect("queued_jobs mutex")
            .push((job_type, job_name));
        if self.allow_enqueue {
            job();
            true
        } else {
            false
        }
    }
}

#[test]
fn pend_save_validated_queues_current_save() {
    let runtime = Arc::new(RecordingPersistenceRuntime {
        allow_enqueue: true,
        ..RecordingPersistenceRuntime::default()
    });
    let persistence = LedgerPersistence::new(runtime.clone());
    let ledger = immutable_ledger(40, 0x31);

    assert!(persistence.pend_save_validated(ledger, false, true));

    let queued = runtime.queued_jobs.lock().expect("queued_jobs mutex");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].0, LedgerPersistenceJobType::PubLedger);
    assert_eq!(
        runtime
            .save_calls
            .lock()
            .expect("save_calls mutex")
            .as_slice(),
        &[(40, true)]
    );
}

#[test]
fn pend_save_validated_falls_back_to_synchronous_save_when_enqueue_rejects() {
    let runtime = Arc::new(RecordingPersistenceRuntime::default());
    let ledger = immutable_ledger(41, 0x32);

    assert!(pend_save_validated(runtime.clone(), ledger, false, false));

    assert_eq!(
        runtime
            .save_calls
            .lock()
            .expect("save_calls mutex")
            .as_slice(),
        &[(41, false)]
    );
    assert_eq!(
        runtime.finished.lock().expect("finished mutex").as_slice(),
        &[41]
    );
}

#[test]
fn persistence_load_helpers_reuse_current_ledger_loading_paths() {
    let leaf = state_leaf(0x41);
    let mut header = LedgerHeader {
        seq: 42,
        account_hash: leaf.get_hash(),
        parent_hash: sample_hash(0x42),
        close_time: 100,
        close_time_resolution: 30,
        ..LedgerHeader::default()
    };
    header.hash = calculate_ledger_hash(&header);
    let provider = RecordingProvider {
        by_index: HashMap::from([(42, header)]),
        by_hash: HashMap::from([(header.hash, header)]),
        newest: Some(header),
    };
    let family = family_with_nodes("persistence-load", HashMap::from([(leaf.get_hash(), leaf)]));

    let loaded = load_ledger_helper(
        header,
        true,
        &NullLedgerJournal,
        &LedgerConfig::default(),
        &family,
    )
    .expect("helper load should not error")
    .expect("helper load should return ledger");
    assert_eq!(loaded.header().seq, 42);

    let by_index = load_by_index(
        42,
        true,
        &NullLedgerJournal,
        &LedgerConfig::default(),
        &family,
        &provider,
    )
    .expect("load by index should not error")
    .expect("load by index should return ledger");
    assert_eq!(by_index.header().hash, header.hash);

    let by_hash = load_by_hash(
        header.hash,
        true,
        &NullLedgerJournal,
        &LedgerConfig::default(),
        &family,
        &provider,
    )
    .expect("load by hash should not error")
    .expect("load by hash should return ledger");
    assert_eq!(by_hash.header().seq, 42);

    let (latest, seq, hash) = get_latest_ledger(
        &NullLedgerJournal,
        &LedgerConfig::default(),
        &family,
        &provider,
    )
    .expect("latest load should not error");
    assert_eq!(seq, 42);
    assert_eq!(hash, header.hash);
    assert_eq!(latest.expect("latest ledger").header().hash, header.hash);
}
