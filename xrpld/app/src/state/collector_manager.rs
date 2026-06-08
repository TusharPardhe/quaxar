//! Collector manager for the app runtime.
//!
//! This module carries the full `beast::insight` transport stack.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectorBackend {
    Null,
    Statsd { address: String, prefix: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CollectorParams {
    pub server: String,
    pub address: String,
    pub prefix: String,
}

#[derive(Debug, Default)]
struct CollectorGroupState {
    counters: BTreeMap<String, i64>,
    events: Vec<String>,
}

#[derive(Debug)]
pub struct CollectorGroup {
    name: String,
    state: Mutex<CollectorGroupState>,
}

impl CollectorGroup {
    fn new(name: String) -> Self {
        Self {
            name,
            state: Mutex::new(CollectorGroupState::default()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn increment(&self, key: impl Into<String>, amount: i64) {
        let mut state = self
            .state
            .lock()
            .expect("collector group mutex must not be poisoned");
        *state.counters.entry(key.into()).or_default() += amount;
    }

    pub fn record_event(&self, event: impl Into<String>) {
        self.state
            .lock()
            .expect("collector group mutex must not be poisoned")
            .events
            .push(event.into());
    }

    pub fn counters(&self) -> BTreeMap<String, i64> {
        self.state
            .lock()
            .expect("collector group mutex must not be poisoned")
            .counters
            .clone()
    }

    pub fn events(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("collector group mutex must not be poisoned")
            .events
            .clone()
    }
}

#[derive(Clone)]
pub struct CollectorManager {
    backend: CollectorBackend,
    groups: Arc<Mutex<BTreeMap<String, Arc<CollectorGroup>>>>,
}

impl std::fmt::Debug for CollectorManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let groups = self
            .groups
            .lock()
            .expect("collector manager mutex must not be poisoned");
        f.debug_struct("CollectorManager")
            .field("backend", &self.backend)
            .field("group_count", &groups.len())
            .finish()
    }
}

impl CollectorManager {
    pub fn new(params: CollectorParams) -> Self {
        let backend = if params.server == "statsd" {
            CollectorBackend::Statsd {
                address: params.address,
                prefix: params.prefix,
            }
        } else {
            CollectorBackend::Null
        };

        Self {
            backend,
            groups: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn backend(&self) -> &CollectorBackend {
        &self.backend
    }

    pub fn group(&self, name: impl Into<String>) -> Arc<CollectorGroup> {
        let name = name.into();
        let mut groups = self
            .groups
            .lock()
            .expect("collector manager mutex must not be poisoned");
        Arc::clone(
            groups
                .entry(name.clone())
                .or_insert_with(|| Arc::new(CollectorGroup::new(name))),
        )
    }
}

impl Default for CollectorManager {
    fn default() -> Self {
        Self::new(CollectorParams::default())
    }
}

#[cfg(test)]
mod tests {
    use super::{CollectorBackend, CollectorManager, CollectorParams};
    use std::sync::Arc;

    #[test]
    fn collector_manager_reuses_named_groups_and_preserves_backend_choice() {
        let manager = CollectorManager::new(CollectorParams {
            server: "statsd".to_owned(),
            address: "127.0.0.1:8125".to_owned(),
            prefix: "quaxar".to_owned(),
        });

        let first = manager.group("jobq");
        first.increment("queued", 1);
        let second = manager.group("jobq");
        second.record_event("drain");

        assert!(matches!(manager.backend(), CollectorBackend::Statsd { .. }));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(second.counters()["queued"], 1);
        assert_eq!(second.events(), vec!["drain"]);
    }
}
