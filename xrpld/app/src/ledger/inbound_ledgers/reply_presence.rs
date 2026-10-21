//! Off-owner NodeStore presence check for peer `TMLedgerData` replies.
//!
//! rippled processes each reply on a `JtLedgerData` job: `addKnownNode`
//! descends to the node's position and `fetchNodeNT` resolves each step from
//! the TreeNodeCache or, failing that, the NodeStore. A received node that is
//! already on disk therefore attaches as a duplicate: it is neither counted as
//! progress nor written again.
//!
//! Quaxar's coordinator owner must never block on the NodeStore, so its reply
//! path consults only resident data. A reply node that is on disk but not
//! cached was treated as new and written again. On a cold mainnet catch-up
//! almost every reply node is in that state, which kept the single owner busy
//! re-writing the tree (2026-10-20: ~12k duplicate writes/s, flat store size).
//!
//! This stage runs rippled's NodeStore lookup off the owner: when a reply is
//! admitted, a bounded worker pool looks up each node that is not cached and,
//! if the NodeStore has it, installs the stored node in the shared
//! TreeNodeCache marked persisted. The owner's unchanged reply path then finds
//! it resident and durable (rippled's duplicate outcome). The owner remains
//! the only writer of acquisition state, and the decision order stays cache,
//! NodeStore, fetch pack, network.
//!
//! The stage is best effort and side-effect free with respect to ledger
//! contents: it only adds canonical, hash-verified stored nodes to a cache.
//! If the queue is full or the owner reaches a packet first, the owner simply
//! behaves as before. NodeStore reads use the ordinary watched fetch, so a
//! node read here is protected from a concurrent prune exactly like any other
//! read.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::MonotonicClock;
use shamap::nodes::tree_node::SHAMapTreeNode;
use shamap::tree_node_cache::TreeNodeCache;

use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

/// Queued reply batches. A full queue drops the warm-up, never the reply.
const QUEUE_CAPACITY: usize = 256;

/// Counters exposed for diagnostics.
#[derive(Debug, Default)]
pub(crate) struct ReplyPresenceStats {
    pub(crate) batches: AtomicU64,
    pub(crate) dropped_batches: AtomicU64,
    pub(crate) nodes_checked: AtomicU64,
    pub(crate) already_cached: AtomicU64,
    pub(crate) found_on_disk: AtomicU64,
    pub(crate) not_on_disk: AtomicU64,
}

struct Job {
    nodes: Vec<Vec<u8>>,
    ledger_seq: u32,
}

/// Bounded worker pool performing the off-owner presence lookup.
pub(crate) struct ReplyPresenceWarmer {
    tx: Mutex<Option<mpsc::SyncSender<Job>>>,
    stats: Arc<ReplyPresenceStats>,
}

impl ReplyPresenceWarmer {
    pub(crate) fn start(
        tree_cache: Arc<TreeNodeCache<MonotonicClock>>,
        node_store: SHAMapStoreNodeStore,
        threads: usize,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::sync_channel::<Job>(QUEUE_CAPACITY);
        let rx = Arc::new(Mutex::new(rx));
        let stats = Arc::new(ReplyPresenceStats::default());
        for index in 0..threads.max(1) {
            let rx = Arc::clone(&rx);
            let cache = Arc::clone(&tree_cache);
            let store = node_store.clone();
            let stats = Arc::clone(&stats);
            let _ = std::thread::Builder::new()
                .name(format!("quaxar-reply-{index}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let Ok(guard) = rx.lock() else { return };
                            match guard.recv() {
                                Ok(job) => job,
                                Err(_) => return,
                            }
                        };
                        warm_nodes(&cache, &store, &stats, &job.nodes, job.ledger_seq);
                    }
                });
        }
        Arc::new(Self {
            tx: Mutex::new(Some(tx)),
            stats,
        })
    }

    /// Queue a reply's wire nodes for the presence lookup. Never blocks.
    pub(crate) fn submit(&self, nodes: Vec<Vec<u8>>, ledger_seq: u32) {
        if nodes.is_empty() {
            return;
        }
        let Ok(guard) = self.tx.lock() else { return };
        let Some(tx) = guard.as_ref() else { return };
        match tx.try_send(Job { nodes, ledger_seq }) {
            Ok(()) => {
                self.stats.batches.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.stats.dropped_batches.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn stats(&self) -> &ReplyPresenceStats {
        &self.stats
    }

    /// Stop accepting work; workers exit once the queue drains.
    pub(crate) fn stop(&self) {
        if let Ok(mut guard) = self.tx.lock() {
            guard.take();
        }
    }
}

/// Look up each wire node in the cache, then the NodeStore; install stored
/// nodes in the cache marked persisted. Pure with respect to ledger state.
pub(crate) fn warm_nodes(
    cache: &TreeNodeCache<MonotonicClock>,
    store: &SHAMapStoreNodeStore,
    stats: &ReplyPresenceStats,
    nodes: &[Vec<u8>],
    ledger_seq: u32,
) {
    for raw in nodes {
        stats.nodes_checked.fetch_add(1, Ordering::Relaxed);
        let Ok(Some(decoded)) = SHAMapTreeNode::make_from_wire(raw) else {
            continue;
        };
        let hash: SHAMapHash = decoded.get_hash();
        if cache.fetch(hash.as_uint256()).is_some() {
            stats.already_cached.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let object = match store {
            SHAMapStoreNodeStore::Single(database) => database.fetch_node_object(
                hash.as_uint256(),
                ledger_seq,
                nodestore::FetchType::Async,
                false,
            ),
        };
        let Some(object) = object else {
            stats.not_on_disk.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        // Install the stored form (hash-checked against the request) so the
        // cache holds exactly the durable bytes, as a NodeStore read would.
        let Ok(mut node) = SHAMapTreeNode::make_from_prefix(object.data(), hash) else {
            continue;
        };
        if node.get_hash() != hash {
            continue;
        }
        node.mark_persisted();
        cache.canonicalize_replace_client(hash.as_uint256(), &mut node);
        stats.found_on_disk.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use basics::base_uint::Uint256;
    use shamap::item::SHAMapItem;
    use shamap::nodes::tree_node::SHAMapNodeType;
    use time::Duration;

    pub(crate) fn memory_store(path: &str) -> SHAMapStoreNodeStore {
        use basics::basic_config::Section;
        use nodestore::{Manager as _, ManagerImp, NullJournal};
        let mut config = Section::new("node_db");
        config.set("type", "Memory");
        config.set("path", path);
        SHAMapStoreNodeStore::Single(
            ManagerImp::new()
                .make_database(0, Arc::new(nodestore::DummyScheduler), 1, &config, Arc::new(NullJournal))
                .expect("memory node store"),
        )
    }

    pub(crate) fn store_node(store: &SHAMapStoreNodeStore, node: &SHAMapTreeNode) {
        let SHAMapStoreNodeStore::Single(db) = store;
        db.store(
            nodestore::NodeObjectType::AccountNode,
            node.serialize_with_prefix().expect("prefix"),
            *node.get_hash().as_uint256(),
            1,
        )
        .expect("store");
    }

    fn leaf(fill: u8) -> basics::intrusive_pointer::SharedIntrusive<SHAMapTreeNode> {
        SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(Uint256::from_array([fill; 32]), vec![fill; 40]),
            0,
        )
    }

    #[test]
    fn on_disk_reply_node_is_cached_persisted_missing_one_is_not() {
        let cache = TreeNodeCache::new("reply-presence-test", 64, Duration::seconds(60), MonotonicClock::default());
        let store = memory_store("reply-presence-1");
        let stats = ReplyPresenceStats::default();
        let on_disk = leaf(0x31);
        let missing = leaf(0x32);
        store_node(&store, &on_disk);

        let wire = vec![
            on_disk.serialize_for_wire().unwrap(),
            missing.serialize_for_wire().unwrap(),
            vec![0xFF, 0x00], // undecodable: ignored
        ];
        warm_nodes(&cache, &store, &stats, &wire, 1);

        let cached = cache.fetch(on_disk.get_hash().as_uint256()).expect("on-disk node cached");
        assert!(cached.is_persisted(), "stored node is durable");
        assert!(cache.fetch(missing.get_hash().as_uint256()).is_none(), "missing node not cached");
        assert_eq!(stats.found_on_disk.load(Ordering::Relaxed), 1);
        assert_eq!(stats.not_on_disk.load(Ordering::Relaxed), 1);

        // A second pass sees it cached and does nothing else.
        warm_nodes(&cache, &store, &stats, &wire[..1], 1);
        assert_eq!(stats.already_cached.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn already_cached_non_persisted_node_is_left_untouched() {
        let cache = TreeNodeCache::new("reply-presence-test2", 64, Duration::seconds(60), MonotonicClock::default());
        let store = memory_store("reply-presence-2");
        let stats = ReplyPresenceStats::default();
        let mut node = leaf(0x33);
        store_node(&store, &node);
        cache.canonicalize_replace_client(node.get_hash().as_uint256(), &mut node);
        warm_nodes(&cache, &store, &stats, &[node.serialize_for_wire().unwrap()], 1);
        assert!(!cache.fetch(node.get_hash().as_uint256()).unwrap().is_persisted(),
            "a node already owned by the owner path keeps its provenance");
        assert_eq!(stats.found_on_disk.load(Ordering::Relaxed), 0);
    }
}
