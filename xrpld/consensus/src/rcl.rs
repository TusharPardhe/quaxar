//! Concrete RCL (Ripple Consensus Ledger) types re-exported at the crate
//! root, ported from `RCLCxLedger.h` and `RCLCxTx.h`.
//!
//! These are thin, ledger/SHAMap-backed instantiations of the generic
//! [`crate::ConsensusTx`], [`crate::ConsensusTxSet`], and
//! [`crate::ConsensusLedger`] traits used throughout the `app` crate's
//! consensus wiring (`xrpld/app/src/consensus/rcl_consensus.rs` and its
//! sibling modules). They live here rather than in `app` because the
//! `app` crate's `ConsensusRunner`/`AppConsensusRuntime` types refer to
//! them as `consensus::RclCxTx` / `consensus::RclCxLedger` directly.

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

/// A plain, wire/queue-facing transaction identifier. Constructed directly
/// as a struct literal at call sites (e.g. `RclCxTx { id: item.key() }`),
/// matching the shape `network_ops_runtime.rs`'s `handle_map_complete`
/// requires for the `got_tx_set(Vec<RclCxTx>)` transit path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RclCxTx {
    pub id: Uint256,
}

impl crate::ConsensusTx for RclCxTx {
    type Id = Uint256;

    fn tx_id(&self) -> Uint256 {
        self.id
    }
}

/// A SHAMap-backed transaction, carrying the full serialized `STTx` blob.
/// This is the working-set element type used by [`RclTxSet`] during
/// consensus (as opposed to [`RclCxTx`], which is just an id used for
/// wire/queue transit).
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

impl crate::ConsensusTx for RclCxTxRef {
    type Id = Uint256;

    fn tx_id(&self) -> Uint256 {
        self.id()
    }
}

/// A SHAMap-backed transaction set, providing the copy-on-write insert/
/// erase/compare semantics `Consensus<Adaptor>` needs while building and
/// comparing peer positions. Ported from `RCLTxSet`.
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

    /// Returns every item stored in this transaction set. Matches the
    /// reference's `for (auto const& item : *result.txns.map)` iteration in
    /// `RCLConsensus::Adaptor::doAccept`, used to build the `CanonicalTXSet`
    /// (`retriableTxs`) that is actually applied to build the new ledger.
    /// This is the one true source of "what did consensus agree to include"
    /// -- it must be read directly from this already-captured set, not by
    /// re-querying any mutable, concurrently-reset open ledger view.
    pub fn all_items(&self) -> Vec<Arc<SHAMapItem>> {
        let map = StorageTree::from_loaded_root(
            self.root.clone(),
            1,
            self.backed,
            self.ledger_seq,
            Arc::clone(&self.cache),
        );
        let mut items = Vec::new();
        map.visit_leaves(&mut |_| None, &mut |item| {
            items.push(Arc::new(item.clone()))
        })
        .expect("loaded consensus tx set should not need fetches");
        items
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

    pub fn to_sync_tree(&self) -> shamap::sync::SyncTree {
        shamap::sync::SyncTree::from_root_with_type(
            self.root.clone(),
            shamap::sync::SHAMapType::Transaction,
            self.backed,
            self.ledger_seq,
            shamap::sync::SyncState::Modifying,
        )
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
        let root = self.map.root();
        root.update_hash_deep();
        RclTxSet {
            root,
            cache: self.cache,
            backed: self.backed,
            ledger_seq: self.ledger_seq,
        }
    }
}

impl crate::ConsensusTxSet for RclTxSet {
    type Id = Uint256;
    type Tx = RclCxTxRef;

    fn exists(&self, tx_id: &Uint256) -> bool {
        RclTxSet::exists(self, *tx_id)
    }

    fn find(&self, tx_id: &Uint256) -> Option<RclCxTxRef> {
        RclTxSet::find(self, *tx_id).map(|item| RclCxTxRef::new((*item).clone()))
    }

    fn id(&self) -> Uint256 {
        RclTxSet::id(self)
    }

    fn compare(&self, other: &Self) -> BTreeMap<Uint256, bool> {
        RclTxSet::compare(self, other)
    }

    fn insert(&mut self, tx: RclCxTxRef) -> bool {
        let mut editable = self.mutable_view();
        let inserted = editable.insert(&tx);
        *self = editable.freeze();
        inserted
    }

    fn erase(&mut self, tx_id: &Uint256) -> bool {
        let mut editable = self.mutable_view();
        let erased = editable.erase(*tx_id);
        *self = editable.freeze();
        erased
    }
}

/// A ledger snapshot suitable for the consensus algorithm's `ConsensusLedger`
/// bound. Wraps a real `Arc<ledger::Ledger>`. Ported from `RCLCxLedger`.
#[derive(Debug, Clone)]
pub struct RclCxLedger {
    ledger: Arc<Ledger>,
}

impl RclCxLedger {
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

impl crate::ConsensusLedger for RclCxLedger {
    type Id = Uint256;
    type Seq = u32;

    fn id(&self) -> Uint256 {
        RclCxLedger::id(self)
    }

    fn seq(&self) -> u32 {
        RclCxLedger::seq(self)
    }

    fn close_time_resolution(&self) -> std::time::Duration {
        let secs = RclCxLedger::close_time_resolution(self)
            .whole_seconds()
            .max(0);
        std::time::Duration::from_secs(secs as u64)
    }

    fn close_agree(&self) -> bool {
        RclCxLedger::close_agree(self)
    }

    fn close_time(&self) -> basics::chrono::NetClockTimePoint {
        RclCxLedger::close_time(self)
    }

    fn parent_close_time(&self) -> basics::chrono::NetClockTimePoint {
        RclCxLedger::parent_close_time(self)
    }
}

impl Default for RclCxLedger {
    fn default() -> Self {
        Self::new(Arc::new(Ledger::from_ledger_seq_and_close_time(
            0, 0, false,
        )))
    }
}

/// Alias for [`crate::rcl_support::ValidationsAdaptor`], used as
/// the trait bound for [`RclValidations`]. Kept as a distinct name (rather
/// than requiring callers to spell out the full `rcl_support` path) since
/// `xrpld/app`'s `negative_unl_vote.rs` refers to it as
/// `consensus::RclValidationsAdapter`.
pub use crate::rcl_support::ValidationsAdaptor as RclValidationsAdapter;

/// Alias for [`crate::rcl_support::ValStatus`], used at the
/// crate root as `consensus::ValidationStatus` by
/// `network_ops_validation_runtime.rs`.
pub use crate::rcl_support::ValStatus as ValidationStatus;

/// A thin ergonomic wrapper around [`crate::rcl_support::Validations`]
/// exposing a couple of `&mut self`-shaped convenience methods that
/// `xrpld/app`'s `negative_unl_vote.rs` expects (mirroring the reference's
/// `RCLValidations` type alias, which in the C++ code is simply
/// `Validations<RCLValidationsAdaptor>` used directly -- Rust's ownership
/// rules make a `&mut self` call site read more naturally here even though
/// the underlying tracker is fully `&self`-based internally via its own
/// mutex).
///
/// The two convenience methods below are pinned to concrete `u32`
/// sequence numbers and `Uint256` ledger ids rather than `A`'s generic
/// associated types: this matches the RCL instantiation exactly (the only
/// one that exists), and lets `negative_unl_vote.rs` -- which is written
/// against concrete rippled-domain types, not generic algorithm types --
/// call them without threading associated-type bounds through its own
/// generic `NegativeUNLVoteValidations` trait.
pub struct RclValidations<A: RclValidationsAdapter> {
    inner: crate::rcl_support::Validations<A>,
}

impl<A> RclValidations<A>
where
    A: RclValidationsAdapter,
    A::Ledger: crate::model::TrieLedger<Seq = u32, Id = Uint256>,
{
    pub fn new(parms: crate::rcl_support::ValidationParms, adaptor: A) -> Self {
        Self {
            inner: crate::rcl_support::Validations::new(parms, adaptor),
        }
    }

    pub fn inner(&self) -> &crate::rcl_support::Validations<A> {
        &self.inner
    }

    /// Protect the `[low, high)` sequence range from expiry. Matches
    /// `Validations::setSeqToKeep`.
    pub fn set_seq_to_keep(&mut self, low: u32, high: u32) {
        self.inner.set_seq_to_keep(low, high);
    }

    /// The signer public keys of trusted, full validations for `ledger_id`
    /// at sequence `seq`. Built from `Validations::get_trusted_for_ledger`
    /// by mapping each wrapped validation through its `NodeKey`. Used by
    /// `negative_unl_vote.rs`'s reliability score table.
    pub fn trusted_for_ledger_by_sequence(
        &mut self,
        ledger_id: Uint256,
        seq: u32,
    ) -> Vec<<A::Validation as crate::rcl_support::ValidationT>::NodeKey>
    where
        <A::Validation as crate::rcl_support::ValidationT>::Wrapped: AsValidationKey<A>,
    {
        self.inner
            .get_trusted_for_ledger(&ledger_id, seq)
            .into_iter()
            .map(|wrapped| wrapped.node_key())
            .collect()
    }
}

/// Extracts the signer key from a wrapped validation. Implemented for
/// `Arc<protocol::STValidation>` in `xrpld/app`'s `rcl_validation.rs` (kept
/// here as a trait rather than a hard dependency on `protocol::STValidation`
/// so this crate does not need to know about the concrete wrapped type,
/// consistent with the rest of this crate's adaptor-parameterized design).
pub trait AsValidationKey<A: RclValidationsAdapter> {
    fn node_key(&self) -> <A::Validation as crate::rcl_support::ValidationT>::NodeKey;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rcl_cx_ledger_default_is_genesis() {
        let ledger = RclCxLedger::default();
        assert_eq!(ledger.seq(), 0);
    }

    #[test]
    fn rcl_cx_tx_id_matches_field() {
        let id = Uint256::from_array([9; 32]);
        let tx = RclCxTx { id };
        assert_eq!(crate::ConsensusTx::tx_id(&tx), id);
    }
}
