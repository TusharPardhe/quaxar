//! Narrow `BookListeners` port from `xrpl/ledger/BookListeners.*`.

use basics::unordered_containers::{HashMap, HashSet};
use protocol::{JsonValue, MultiApiJson};
use std::sync::{Arc, Mutex, Weak};

pub trait BookListenerSubscriber: Send + Sync + 'static {
    fn seq(&self) -> u64;
    fn api_version(&self) -> u32;
    fn send(&self, json: &JsonValue, broadcast: bool);
}

#[derive(Default)]
pub struct BookListeners {
    listeners: Mutex<HashMap<u64, Weak<dyn BookListenerSubscriber>>>,
}

impl std::fmt::Debug for BookListeners {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BookListeners").finish_non_exhaustive()
    }
}

impl BookListeners {
    pub fn add_subscriber(&self, subscriber: Arc<dyn BookListenerSubscriber>) {
        self.listeners
            .lock()
            .expect("book listeners mutex must not be poisoned")
            .insert(subscriber.seq(), Arc::downgrade(&subscriber));
    }

    pub fn remove_subscriber(&self, seq: u64) {
        self.listeners
            .lock()
            .expect("book listeners mutex must not be poisoned")
            .remove(&seq);
    }

    pub fn publish(&self, json: &MultiApiJson, have_published: &mut HashSet<u64>) {
        let mut listeners = self
            .listeners
            .lock()
            .expect("book listeners mutex must not be poisoned");
        listeners.retain(|seq, weak| {
            let Some(subscriber) = weak.upgrade() else {
                return false;
            };

            if have_published.insert(*seq) {
                json.visit(subscriber.api_version(), |value| {
                    subscriber.send(value, true);
                });
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use protocol::JsonValue;

    use super::{BookListenerSubscriber, BookListeners};

    #[derive(Debug)]
    struct TestSubscriber {
        seq: u64,
        api_version: u32,
        seen: Mutex<Vec<JsonValue>>,
    }

    impl BookListenerSubscriber for TestSubscriber {
        fn seq(&self) -> u64 {
            self.seq
        }

        fn api_version(&self) -> u32 {
            self.api_version
        }

        fn send(&self, json: &JsonValue, _broadcast: bool) {
            self.seen
                .lock()
                .expect("seen mutex must not be poisoned")
                .push(json.clone());
        }
    }

    #[test]
    fn publish_deduplicates_by_subscriber_seq_and_prunes_expired() {
        let listeners = BookListeners::default();
        let subscriber = Arc::new(TestSubscriber {
            seq: 1,
            api_version: 2,
            seen: Mutex::new(Vec::new()),
        });
        listeners.add_subscriber(subscriber.clone());

        let mut json = protocol::MultiApiJson::new(JsonValue::Object(BTreeMap::new()));
        json.visit_mut(2, |value| {
            let JsonValue::Object(object) = value else {
                panic!("value should be object");
            };
            object.insert("ledger_index".to_owned(), JsonValue::Unsigned(5));
        });

        let mut have_published = basics::unordered_containers::HashSet::default();
        listeners.publish(&json, &mut have_published);
        listeners.publish(&json, &mut have_published);

        assert_eq!(
            *subscriber
                .seen
                .lock()
                .expect("seen mutex must not be poisoned"),
            vec![JsonValue::Object(BTreeMap::from([(
                "ledger_index".to_owned(),
                JsonValue::Unsigned(5)
            )]))]
        );

        drop(subscriber);
        let mut second_seen = basics::unordered_containers::HashSet::default();
        listeners.publish(&json, &mut second_seen);
    }
}
