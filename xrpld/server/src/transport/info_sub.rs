//! InfoSub ported from `xrpl/server/InfoSub.h/the reference source`.
//!
//! Base class for subscription tracking — assigns unique sequence IDs,
//! manages subscription lifecycle (accounts, ledgers, books, transactions),
//! delivers JSON events to subscribers.

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use protocol::{AccountID, JsonValue};

/// Global sequence ID generator for InfoSub instances.
static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);

fn assign_id() -> u64 {
    NEXT_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Trait for InfoSub request objects (path_find create/close/status).
pub trait InfoSubRequest: Send + Sync {
    fn do_close(&self) -> JsonValue;
    fn do_status(&self, params: &JsonValue) -> JsonValue;
}

/// Trait representing the subscription source (SubscriptionManager).
/// On InfoSub drop, unsubscribes from all feeds.
pub trait InfoSubSource: Send + Sync {
    fn unsub_transactions(&self, seq: u64);
    fn unsub_rt_transactions(&self, seq: u64);
    fn unsub_ledger(&self, seq: u64);
    fn unsub_manifests(&self, seq: u64);
    fn unsub_server(&self, seq: u64);
    fn unsub_validations(&self, seq: u64);
    fn unsub_peer_status(&self, seq: u64);
    fn unsub_consensus(&self, seq: u64);
    fn unsub_book_changes(&self, seq: u64);
    fn unsub_account_internal(&self, seq: u64, accounts: &HashSet<AccountID>, real_time: bool);
    fn unsub_account_history_internal(&self, seq: u64, account: &AccountID, history_only: bool);
}

/// Manages a client's subscription to data feeds.
///
/// Each InfoSub gets a unique sequence ID. On drop, it unsubscribes from
/// all feeds it was subscribed to via the Source.
pub struct InfoSub {
    seq: u64,
    api_version: u32,
    source: Arc<dyn InfoSubSource>,
    inner: Mutex<InfoSubInner>,
}

struct InfoSubInner {
    real_time_subscriptions: HashSet<AccountID>,
    normal_subscriptions: HashSet<AccountID>,
    account_history_subscriptions: HashSet<AccountID>,
    request: Option<Arc<dyn InfoSubRequest>>,
}

impl InfoSub {
    pub fn new(source: Arc<dyn InfoSubSource>) -> Self {
        Self {
            seq: assign_id(),
            api_version: 0,
            source,
            inner: Mutex::new(InfoSubInner {
                real_time_subscriptions: HashSet::new(),
                normal_subscriptions: HashSet::new(),
                account_history_subscriptions: HashSet::new(),
                request: None,
            }),
        }
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn api_version(&self) -> u32 {
        self.api_version
    }

    pub fn set_api_version(&mut self, version: u32) {
        self.api_version = version;
    }

    pub fn insert_sub_account_info(&self, account: AccountID, real_time: bool) {
        let mut inner = self.inner.lock().unwrap();
        if real_time {
            inner.real_time_subscriptions.insert(account);
        } else {
            inner.normal_subscriptions.insert(account);
        }
    }

    pub fn delete_sub_account_info(&self, account: &AccountID, real_time: bool) {
        let mut inner = self.inner.lock().unwrap();
        if real_time {
            inner.real_time_subscriptions.remove(account);
        } else {
            inner.normal_subscriptions.remove(account);
        }
    }

    /// Returns false if already subscribed to this account.
    pub fn insert_sub_account_history(&self, account: AccountID) -> bool {
        let mut inner = self.inner.lock().unwrap();
        inner.account_history_subscriptions.insert(account)
    }

    pub fn delete_sub_account_history(&self, account: &AccountID) {
        let mut inner = self.inner.lock().unwrap();
        inner.account_history_subscriptions.remove(account);
    }

    pub fn clear_request(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.request = None;
    }

    pub fn set_request(&self, req: Arc<dyn InfoSubRequest>) {
        let mut inner = self.inner.lock().unwrap();
        inner.request = Some(req);
    }

    pub fn get_request(&self) -> Option<Arc<dyn InfoSubRequest>> {
        let inner = self.inner.lock().unwrap();
        inner.request.clone()
    }

    /// Called when there's nothing to send. Default no-op.
    pub fn on_send_empty(&self) {}
}

impl Drop for InfoSub {
    fn drop(&mut self) {
        // Unsubscribe from all global feeds
        self.source.unsub_transactions(self.seq);
        self.source.unsub_rt_transactions(self.seq);
        self.source.unsub_ledger(self.seq);
        self.source.unsub_manifests(self.seq);
        self.source.unsub_server(self.seq);
        self.source.unsub_validations(self.seq);
        self.source.unsub_peer_status(self.seq);
        self.source.unsub_consensus(self.seq);
        self.source.unsub_book_changes(self.seq);

        let inner = self.inner.lock().unwrap();

        // Unsubscribe account feeds using internal method (won't call back)
        if !inner.real_time_subscriptions.is_empty() {
            self.source
                .unsub_account_internal(self.seq, &inner.real_time_subscriptions, true);
        }
        if !inner.normal_subscriptions.is_empty() {
            self.source
                .unsub_account_internal(self.seq, &inner.normal_subscriptions, false);
        }
        for account in &inner.account_history_subscriptions {
            self.source
                .unsub_account_history_internal(self.seq, account, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    struct MockSource {
        unsub_count: AtomicU32,
    }

    impl InfoSubSource for MockSource {
        fn unsub_transactions(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_rt_transactions(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_ledger(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_manifests(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_server(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_validations(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_peer_status(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_consensus(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_book_changes(&self, _: u64) {
            self.unsub_count.fetch_add(1, Ordering::Relaxed);
        }
        fn unsub_account_internal(&self, _: u64, _: &HashSet<AccountID>, _: bool) {}
        fn unsub_account_history_internal(&self, _: u64, _: &AccountID, _: bool) {}
    }

    #[test]
    fn unique_seq_ids() {
        let src: Arc<dyn InfoSubSource> = Arc::new(MockSource {
            unsub_count: AtomicU32::new(0),
        });
        let a = InfoSub::new(src.clone());
        let b = InfoSub::new(src.clone());
        assert_ne!(a.seq(), b.seq());
    }

    #[test]
    fn drop_unsubscribes() {
        let src = Arc::new(MockSource {
            unsub_count: AtomicU32::new(0),
        });
        let src_trait: Arc<dyn InfoSubSource> = src.clone();
        {
            let _sub = InfoSub::new(src_trait);
        }
        // 9 global unsub calls on drop
        assert_eq!(src.unsub_count.load(Ordering::Relaxed), 9);
    }
}
