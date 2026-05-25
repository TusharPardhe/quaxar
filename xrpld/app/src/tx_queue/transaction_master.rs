//! Narrow `xrpld/app/ledger/TransactionMaster.*` owner port.
//!
//! This preserves the real cache-owner behavior the current Rust substrate can
//! support:
//! - cache-backed transaction canonicalization,
//! - `inLedger(...)` status promotion on cached transactions,
//! - SHAMap item decode for transaction and transaction+meta nodes,
//! - and explicit callback seams for the reference `fetch(...)` overloads above the
//!   still-missing database owner.
//!
//! It intentionally does not claim full `Application` / relational database
//! parity because those owners are not ported yet.

use crate::{TransStatus, Transaction};
use basics::base_uint::Uint256;
use basics::range_set::ClosedInterval;
use basics::tagged_cache::{CacheClock, MonotonicClock, TaggedCache};
use protocol::{STTx, SerialIter, TxMeta, TxSearched};
use shamap::item::SHAMapItem;
use shamap::tree_node::SHAMapNodeType;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use time::Duration;

pub type SharedTransaction = Arc<Mutex<Transaction>>;
pub type TransactionLoadResult = (SharedTransaction, Option<TxMeta>);

#[derive(Debug)]
pub enum TransactionFetchResult {
    Found(TransactionLoadResult),
    NotFound(TxSearched),
}

#[derive(Debug)]
pub struct TransactionMaster<C = MonotonicClock>
where
    C: CacheClock,
{
    cache: TaggedCache<Uint256, Mutex<Transaction>, C>,
}

impl TransactionMaster<MonotonicClock> {
    pub fn new() -> Self {
        Self::new_with_clock(MonotonicClock::default())
    }
}

impl Default for TransactionMaster<MonotonicClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> TransactionMaster<C>
where
    C: CacheClock,
{
    pub fn new_with_clock(clock: C) -> Self {
        Self {
            cache: TaggedCache::new("TransactionCache", 65_536, Duration::minutes(30), clock),
        }
    }

    pub fn fetch_from_cache(&self, txn_id: &Uint256) -> Option<SharedTransaction> {
        self.cache.fetch(txn_id)
    }

    pub fn fetch<E>(
        &self,
        txn_id: Uint256,
        load: impl FnOnce() -> Result<TransactionFetchResult, E>,
    ) -> Result<TransactionFetchResult, E> {
        if let Some(txn) = self.fetch_from_cache(&txn_id) {
            let validated = txn
                .lock()
                .expect("transaction mutex must not be poisoned")
                .is_validated();
            if !validated {
                return Ok(TransactionFetchResult::Found((txn, None)));
            }
        }

        match load()? {
            TransactionFetchResult::Found((mut txn, meta)) => {
                self.cache.canonicalize_replace_client(&txn_id, &mut txn);
                Ok(TransactionFetchResult::Found((txn, meta)))
            }
            TransactionFetchResult::NotFound(searched) => {
                Ok(TransactionFetchResult::NotFound(searched))
            }
        }
    }

    pub fn fetch_in_range<E>(
        &self,
        txn_id: Uint256,
        range: ClosedInterval<u32>,
        load: impl FnOnce(ClosedInterval<u32>) -> Result<TransactionFetchResult, E>,
    ) -> Result<TransactionFetchResult, E> {
        self.fetch(txn_id, || load(range))
    }

    pub fn fetch_with_loader<E>(
        &self,
        txn_id: Uint256,
        load: impl FnOnce() -> Result<Option<TransactionLoadResult>, E>,
    ) -> Result<Option<TransactionLoadResult>, E> {
        match self.fetch(txn_id, || {
            Ok(match load()? {
                Some(result) => TransactionFetchResult::Found(result),
                None => TransactionFetchResult::NotFound(TxSearched::Unknown),
            })
        })? {
            TransactionFetchResult::Found(result) => Ok(Some(result)),
            TransactionFetchResult::NotFound(_) => Ok(None),
        }
    }

    pub fn fetch_from_shamap_item(
        &self,
        item: &SHAMapItem,
        node_type: SHAMapNodeType,
        commit_ledger: u32,
    ) -> Result<Option<Arc<STTx>>, String> {
        if let Some(txn) = self.fetch_from_cache(&item.key()) {
            let mut cached = txn.lock().expect("transaction mutex must not be poisoned");
            if commit_ledger != 0 {
                cached.set_status_with_ledger(TransStatus::COMMITTED, commit_ledger, None, None);
            }
            return Ok(Some(Arc::clone(cached.get_s_transaction())));
        }

        match node_type {
            SHAMapNodeType::TransactionNm => parse_sttx(item.data()).map(Some),
            SHAMapNodeType::TransactionMd => {
                let mut serial = SerialIter::new(item.data());
                let blob =
                    catch_unwind(AssertUnwindSafe(|| serial.get_vl())).map_err(|payload| {
                        unwind_message(payload).unwrap_or_else(|| {
                            "failed to parse transaction-with-meta payload".to_string()
                        })
                    })?;
                parse_sttx(&blob).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub fn in_ledger(
        &self,
        hash: Uint256,
        ledger: u32,
        txn_seq: Option<u32>,
        network_id: Option<u32>,
    ) -> bool {
        let Some(txn) = self.fetch_from_cache(&hash) else {
            return false;
        };

        txn.lock()
            .expect("transaction mutex must not be poisoned")
            .set_status_with_ledger(TransStatus::COMMITTED, ledger, txn_seq, network_id);
        true
    }

    pub fn canonicalize(&self, txn: &mut SharedTransaction) {
        let txn_id = txn
            .lock()
            .expect("transaction mutex must not be poisoned")
            .get_id();
        if !txn_id.is_zero() {
            self.cache.canonicalize_replace_client(&txn_id, txn);
        }
    }

    pub fn sweep(&self) {
        self.cache.sweep();
    }

    pub fn get_cache(&self) -> &TaggedCache<Uint256, Mutex<Transaction>, C> {
        &self.cache
    }

    pub fn cache_keys(&self) -> Vec<Uint256> {
        self.cache.get_keys()
    }
}

fn parse_sttx(bytes: &[u8]) -> Result<Arc<STTx>, String> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut serial = SerialIter::new(bytes);
        Arc::new(STTx::from_serial_iter(&mut serial))
    }))
    .map_err(|payload| unwind_message(payload).unwrap_or_else(|| "failed to parse STTx".into()))
}

fn unwind_message(payload: Box<dyn std::any::Any + Send>) -> Option<String> {
    match payload.downcast::<String>() {
        Ok(message) => Some(*message),
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => Some((*message).to_string()),
            Err(_) => None,
        },
    }
}
