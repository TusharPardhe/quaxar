//! Rust `CachedView` port using the shared `TaggedCache` seam.

use std::sync::{Arc, Mutex, OnceLock};

use basics::base_uint::Uint256;
use basics::counted_object::Counter;
use basics::tagged_cache::{MonotonicClock, TaggedCache};
use basics::unordered_containers::HardenedHashMap;
use time::Duration;

use crate::cached_sles::CachedSles;
use crate::read_view::{DigestAwareReadView, ReadView, ReadViewTx, ViewError};
use crate::{Fees, LedgerHeader};

type DigestMap = HardenedHashMap<Uint256, Uint256>;

fn hit_counter() -> &'static Counter {
    static COUNTER: OnceLock<Counter> = OnceLock::new();
    COUNTER.get_or_init(|| Counter::new("CachedView::hit"))
}

fn hit_expired_counter() -> &'static Counter {
    static COUNTER: OnceLock<Counter> = OnceLock::new();
    COUNTER.get_or_init(|| Counter::new("CachedView::hitExpired"))
}

fn miss_counter() -> &'static Counter {
    static COUNTER: OnceLock<Counter> = OnceLock::new();
    COUNTER.get_or_init(|| Counter::new("CachedView::miss"))
}

#[derive(Debug)]
pub struct CachedView<B, C = MonotonicClock> {
    base: Arc<B>,
    cache: Arc<CachedSles<C>>,
    digest_map: Mutex<DigestMap>,
}

impl<B> CachedView<B, MonotonicClock>
where
    B: DigestAwareReadView,
{
    pub fn new(base: Arc<B>) -> Self {
        Self::with_cache(
            base,
            Arc::new(TaggedCache::new(
                "CachedView",
                65_536,
                Duration::seconds(300),
                MonotonicClock::default(),
            )),
        )
    }
}

impl<B, C> CachedView<B, C>
where
    B: DigestAwareReadView,
    C: basics::tagged_cache::CacheClock,
{
    pub fn with_cache(base: Arc<B>, cache: Arc<CachedSles<C>>) -> Self {
        Self {
            base,
            cache,
            digest_map: Mutex::new(DigestMap::default()),
        }
    }

    pub fn base(&self) -> &Arc<B> {
        &self.base
    }

    pub fn cache(&self) -> &Arc<CachedSles<C>> {
        &self.cache
    }
}

impl<B, C> ReadView for CachedView<B, C>
where
    B: DigestAwareReadView + std::fmt::Debug,
    C: basics::tagged_cache::CacheClock + std::fmt::Debug,
{
    fn open(&self) -> bool {
        self.base.open()
    }

    fn header(&self) -> LedgerHeader {
        self.base.header()
    }

    fn fees(&self) -> Fees {
        self.base.fees()
    }

    fn rules(&self) -> protocol::Rules {
        self.base.rules()
    }

    fn exists(&self, keylet: protocol::Keylet) -> Result<bool, ViewError> {
        Ok(self.read(keylet)?.is_some())
    }

    fn succ(&self, key: Uint256, last: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
        self.base.succ(key, last)
    }

    fn read(
        &self,
        keylet: protocol::Keylet,
    ) -> Result<Option<Arc<protocol::STLedgerEntry>>, ViewError> {
        let mut cache_hit = false;
        let mut base_read = false;

        let digest = if let Some(digest) = {
            let map = self
                .digest_map
                .lock()
                .expect("CachedView digest mutex must not be poisoned");
            map.get(&keylet.key).copied()
        } {
            cache_hit = true;
            Some(digest)
        } else {
            self.base.digest(keylet.key)?
        };

        let Some(digest) = digest else {
            return Ok(None);
        };

        let mut sle = self.cache.fetch(&digest);
        if sle.is_none() {
            base_read = true;
            sle = self.base.read(keylet)?;
            if let Some(sle_ref) = sle.as_ref() {
                self.cache.canonicalize_replace_cache(&digest, sle_ref);
            }
        }

        if cache_hit && base_read {
            hit_expired_counter().increment();
        } else if cache_hit {
            hit_counter().increment();
        } else {
            miss_counter().increment();
        }

        if !cache_hit {
            let mut map = self
                .digest_map
                .lock()
                .expect("CachedView digest mutex must not be poisoned");
            map.insert(keylet.key, digest);
        }

        Ok(sle.filter(|sle| keylet.check_ledger_entry(sle.get_type(), *sle.key())))
    }

    fn sles(&self) -> Result<Vec<Arc<protocol::STLedgerEntry>>, ViewError> {
        self.base.sles()
    }

    fn tx_exists(&self, key: Uint256) -> Result<bool, ViewError> {
        self.base.tx_exists(key)
    }

    fn tx_read(&self, key: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
        self.base.tx_read(key)
    }

    fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
        self.base.txs()
    }

    fn balance_hook_iou(
        &self,
        account: protocol::AccountID,
        issuer: protocol::AccountID,
        amount: protocol::STAmount,
    ) -> protocol::STAmount {
        self.base.balance_hook_iou(account, issuer, amount)
    }

    fn balance_hook_mpt(
        &self,
        account: protocol::AccountID,
        issue: protocol::MPTIssue,
        amount: i64,
    ) -> protocol::STAmount {
        self.base.balance_hook_mpt(account, issue, amount)
    }

    fn balance_hook_self_issue_mpt(
        &self,
        issue: protocol::MPTIssue,
        amount: i64,
    ) -> protocol::STAmount {
        self.base.balance_hook_self_issue_mpt(issue, amount)
    }

    fn owner_count_hook(&self, account: protocol::AccountID, count: u32) -> u32 {
        self.base.owner_count_hook(account, count)
    }
}

impl<B, C> DigestAwareReadView for CachedView<B, C>
where
    B: DigestAwareReadView + std::fmt::Debug,
    C: basics::tagged_cache::CacheClock + std::fmt::Debug,
{
    fn digest(&self, key: Uint256) -> Result<Option<Uint256>, ViewError> {
        self.base.digest(key)
    }
}
