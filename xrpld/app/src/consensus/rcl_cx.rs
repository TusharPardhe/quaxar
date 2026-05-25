//! Thin app-owned ports of `RCLCxLedger.h` and `RCLCxTx.h`.

use std::collections::BTreeMap;
use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::tagged_cache::MonotonicClock;
use ledger::{Ledger, LedgerFill, LedgerFillOptions, get_close_agree, get_json};
use protocol::{JsonValue, STTx, serialize_blob};
use shamap::compare::{Delta, compare};
use shamap::item::SHAMapItem;
use shamap::storage::StorageTree;
use shamap::tree_node::SHAMapNodeType;
use shamap::tree_node_cache::TreeNodeCache;
use time::Duration;

pub type RclTxSetSharedCache = Arc<TreeNodeCache<MonotonicClock, HardenedHashBuilder>>;

#[derive(Debug, Clone)]
pub struct RclCxLedgerRef {
    ledger: Arc<Ledger>,
}

impl RclCxLedgerRef {
    pub fn new(ledger: Arc<Ledger>) -> Self {
        Self { ledger }
    }

    pub fn seq(&self) -> u32 {
        self.ledger.header().seq
    }

    pub fn id(&self) -> Uint256 {
        *self.ledger.header().hash.as_uint256()
    }

    pub fn parent_id(&self) -> Uint256 {
        *self.ledger.header().parent_hash.as_uint256()
    }

    pub fn close_time_resolution(&self) -> Duration {
        Duration::seconds(i64::from(self.ledger.header().close_time_resolution))
    }

    pub fn close_agree(&self) -> bool {
        get_close_agree(&self.ledger.header())
    }

    pub fn close_time(&self) -> basics::chrono::NetClockTimePoint {
        basics::chrono::NetClockTimePoint::new(self.ledger.header().close_time)
    }

    pub fn parent_close_time(&self) -> basics::chrono::NetClockTimePoint {
        basics::chrono::NetClockTimePoint::new(self.ledger.header().parent_close_time)
    }

    pub fn get_json(&self) -> JsonValue {
        get_json(&LedgerFill::new(
            self.ledger.as_ref(),
            LedgerFillOptions::default(),
        ))
        .expect("loaded consensus ledger should render to JSON")
    }

    pub fn ledger(&self) -> Arc<Ledger> {
        Arc::clone(&self.ledger)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RclCxTxRef {
    item: Arc<SHAMapItem>,
}

impl RclCxTxRef {
    pub fn new(item: SHAMapItem) -> Self {
        Self {
            item: Arc::new(item),
        }
    }

    pub fn from_transaction(tx: &STTx) -> Self {
        Self::new(SHAMapItem::new(tx.get_transaction_id(), serialize_blob(tx)))
    }

    pub fn id(&self) -> Uint256 {
        self.item.key()
    }

    pub fn item(&self) -> Arc<SHAMapItem> {
        Arc::clone(&self.item)
    }
}

#[derive(Debug, Clone)]
pub struct RclTxSet {
    root: basics::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>,
    cache: RclTxSetSharedCache,
    backed: bool,
    ledger_seq: u32,
}

#[derive(Debug, Clone)]
pub struct RclTxSetMutable {
    map: StorageTree<MonotonicClock, HardenedHashBuilder>,
    cache: RclTxSetSharedCache,
    backed: bool,
    ledger_seq: u32,
}

impl RclTxSet {
    pub fn new(cache: RclTxSetSharedCache, ledger_seq: u32) -> Self {
        let map = StorageTree::new(1, false, ledger_seq, Arc::clone(&cache));
        Self {
            root: map.root(),
            cache,
            backed: false,
            ledger_seq,
        }
    }

    pub fn from_parts(
        root: basics::intrusive_pointer::SharedIntrusive<shamap::tree_node::SHAMapTreeNode>,
        cache: RclTxSetSharedCache,
        backed: bool,
        ledger_seq: u32,
    ) -> Self {
        Self {
            root,
            cache,
            backed,
            ledger_seq,
        }
    }

    pub fn mutable_view(&self) -> RclTxSetMutable {
        let owner_cowid = self.root.cowid().max(1);
        let next_cowid = owner_cowid + 1;
        let base = StorageTree::from_loaded_root(
            self.root.clone(),
            owner_cowid,
            self.backed,
            self.ledger_seq,
            Arc::clone(&self.cache),
        );
        RclTxSetMutable {
            map: base.mutable_snapshot(next_cowid),
            cache: Arc::clone(&self.cache),
            backed: self.backed,
            ledger_seq: self.ledger_seq,
        }
    }

    pub fn exists(&self, entry: Uint256) -> bool {
        let map = StorageTree::from_loaded_root(
            self.root.clone(),
            1,
            self.backed,
            self.ledger_seq,
            Arc::clone(&self.cache),
        );
        map.has_item(entry, &mut |_| None)
            .expect("loaded consensus tx set should not need fetches")
    }

    pub fn find(&self, entry: Uint256) -> Option<Arc<SHAMapItem>> {
        let map = StorageTree::from_loaded_root(
            self.root.clone(),
            1,
            self.backed,
            self.ledger_seq,
            Arc::clone(&self.cache),
        );
        map.peek_item(entry, &mut |_| None)
            .expect("loaded consensus tx set should not need fetches")
            .map(Arc::new)
    }

    pub fn id(&self) -> Uint256 {
        *self.root.get_hash().as_uint256()
    }

    pub fn compare(&self, other: &Self) -> BTreeMap<Uint256, bool> {
        let mut delta = Delta::new();
        let _ = compare(
            &self.root,
            &other.root,
            self.backed,
            &mut |_| None,
            other.backed,
            &mut |_| None,
            &mut delta,
            65_536,
        )
        .expect("loaded consensus tx sets should compare without fetches");

        delta
            .into_iter()
            .map(|(key, (left, right))| {
                assert!(
                    (left.is_some() && right.is_none()) || (left.is_none() && right.is_some()),
                    "xrpl::RCLTxSet::compare : either side is set"
                );
                (key, left.is_some())
            })
            .collect()
    }
}

impl RclTxSetMutable {
    pub fn insert(&mut self, tx: &RclCxTxRef) -> bool {
        self.map
            .add_item(SHAMapNodeType::TransactionNm, (*tx.item()).clone())
            .expect("loaded consensus tx set insert should not need fetches")
    }

    pub fn erase(&mut self, entry: Uint256) -> bool {
        self.map
            .delete_item(entry)
            .expect("loaded consensus tx set erase should not need fetches")
    }

    pub fn freeze(mut self) -> RclTxSet {
        self.map.unshare();
        RclTxSet {
            root: self.map.root(),
            cache: self.cache,
            backed: self.backed,
            ledger_seq: self.ledger_seq,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RclCxLedgerRef, RclCxTxRef, RclTxSet, RclTxSetSharedCache};
    use std::sync::Arc;

    use basics::tagged_cache::MonotonicClock;
    use ledger::Ledger;
    use protocol::{STAmount, STTx, TxType, get_field_by_symbol};
    use shamap::tree_node_cache::TreeNodeCache;
    use time::Duration;

    fn cache() -> RclTxSetSharedCache {
        Arc::new(TreeNodeCache::<MonotonicClock>::new(
            "RclTxSetTest",
            32,
            Duration::minutes(5),
            MonotonicClock::default(),
        ))
    }

    fn payment(fill: u8) -> STTx {
        STTx::new(TxType::PAYMENT, |tx| {
            tx.set_field_u32(get_field_by_symbol("sfSequence"), u32::from(fill));
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(u64::from(fill), false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
        })
    }

    #[test]
    fn rcl_cx_ledger_reads_header_fields_and_json() {
        let ledger = Arc::new(Ledger::from_ledger_seq_and_close_time(912, 345, true));
        let wrapped = RclCxLedgerRef::new(Arc::clone(&ledger));

        assert_eq!(wrapped.seq(), 912);
        assert_eq!(wrapped.id(), *ledger.header().hash.as_uint256());
        assert_eq!(
            wrapped.parent_id(),
            *ledger.header().parent_hash.as_uint256()
        );
        let protocol::JsonValue::Object(object) = wrapped.get_json() else {
            panic!("ledger json must be an object");
        };
        assert!(object.contains_key("ledger_index"));
    }

    #[test]
    fn rcl_tx_set_insert_erase_exists_find_and_compare() {
        let cache = cache();
        let tx1 = RclCxTxRef::from_transaction(&payment(1));
        let tx2 = RclCxTxRef::from_transaction(&payment(2));
        let tx3 = RclCxTxRef::from_transaction(&payment(3));

        let base = RclTxSet::new(Arc::clone(&cache), 1);
        let mut editable = base.mutable_view();
        assert!(editable.insert(&tx1));
        assert!(editable.insert(&tx2));
        let left = editable.freeze();

        assert!(left.exists(tx1.id()));
        assert!(left.find(tx2.id()).is_some());

        let mut editable = left.mutable_view();
        assert!(editable.erase(tx1.id()));
        assert!(editable.insert(&tx3));
        let right = editable.freeze();

        let diff = right.compare(&left);
        assert_eq!(diff.get(&tx1.id()), Some(&false));
        assert_eq!(diff.get(&tx3.id()), Some(&true));
        assert_eq!(left.find(tx3.id()), None);
        assert!(!right.exists(tx1.id()));
    }
}
