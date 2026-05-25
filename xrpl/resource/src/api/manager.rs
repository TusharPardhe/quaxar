use crate::{
    charge::Charge,
    fees::{
        DROP_THRESHOLD, FEE_DROP, FEE_LOG_AS_DEBUG, FEE_LOG_AS_INFO, FEE_LOG_AS_WARN, FEE_WARNING,
        MINIMUM_GOSSIP_BALANCE, WARNING_THRESHOLD,
    },
    gossip::{Gossip, GossipItem, PublicKey},
    import::{Import, ImportItem},
    logic::{
        Disposition, JournalLevel, Kind, ResourceClock, ResourceCollector, ResourceJournal,
        ResourceMeter, SECONDS_UNTIL_EXPIRATION,
    },
    types::{
        Entry, GOSSIP_EXPIRATION_SECONDS, Key, ResourceState, normalize_inbound_address,
        normalize_unlimited_address,
    },
};
use basics::chrono::SystemStopwatch;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Clone)]
pub struct ResourceManager {
    inner: Arc<ResourceManagerInner>,
}

impl ResourceManager {
    pub fn new_with_clock(
        collector: Arc<dyn ResourceCollector>,
        journal: Arc<dyn ResourceJournal>,
        clock: Arc<dyn ResourceClock>,
    ) -> Self {
        Self {
            inner: Arc::new(ResourceManagerInner::new(collector, journal, clock)),
        }
    }

    pub fn start(&self) {
        self.inner.start();
    }

    pub fn stop(&self) {
        self.inner.stop();
    }

    pub fn new_inbound_endpoint(&self, address: SocketAddr) -> Consumer {
        self.new_endpoint(Kind::Inbound, normalize_inbound_address(address))
    }

    pub fn new_inbound_endpoint_with_proxy(
        &self,
        address: SocketAddr,
        proxy: bool,
        forwarded_for: &str,
    ) -> Consumer {
        if !proxy {
            return self.new_inbound_endpoint(address);
        }

        match IpAddr::from_str(forwarded_for) {
            Ok(ip) => self.new_endpoint(Kind::Inbound, SocketAddr::new(ip, 0)),
            Err(error) => {
                self.inner.journal.log(
                    JournalLevel::Warn,
                    &format!(
                        "forwarded for ({forwarded_for}) from proxy {address} doesn't convert to IP endpoint: {error}",
                    ),
                );
                self.new_inbound_endpoint(address)
            }
        }
    }

    pub fn new_outbound_endpoint(&self, address: SocketAddr) -> Consumer {
        self.new_endpoint(Kind::Outbound, address)
    }

    pub fn new_unlimited_endpoint(&self, address: SocketAddr) -> Consumer {
        self.new_endpoint(Kind::Unlimited, normalize_unlimited_address(address))
    }

    pub fn export_consumers(&self) -> Gossip {
        self.inner.with_state_mut(|state| {
            let now = self.inner.clock.now();
            let mut gossip = Gossip::default();
            gossip.items.reserve(state.inbound.len());
            for key in state.inbound.iter().copied() {
                if let Some(entry) = state.entries.get_mut(&key) {
                    let balance = entry.balance(now);
                    if balance >= MINIMUM_GOSSIP_BALANCE {
                        gossip.items.push(GossipItem::new(balance, key.address));
                    }
                }
            }
            gossip
        })
    }

    pub fn import_consumers(&self, origin: impl Into<String>, gossip: Gossip) {
        let origin = origin.into();
        let now = self.inner.clock.now();

        let previous = self.inner.with_state_mut(|state| {
            let mut next = Import::new(now + GOSSIP_EXPIRATION_SECONDS);
            for gossip_item in gossip.items {
                let key = self.inner.create_or_acquire_key_locked(
                    state,
                    Key::new(
                        Kind::Inbound,
                        normalize_inbound_address(gossip_item.address),
                    ),
                    now,
                );
                if let Some(entry) = state.entries.get_mut(&key) {
                    entry.remote_balance += gossip_item.balance;
                }
                next.items.push(ImportItem {
                    balance: gossip_item.balance,
                    key,
                });
            }

            let previous = state.imports.insert(origin.clone(), next);
            if let Some(previous) = &previous {
                for item in &previous.items {
                    if let Some(entry) = state.entries.get_mut(&item.key) {
                        entry.remote_balance -= item.balance;
                    }
                }
            }
            previous
        });

        if let Some(previous) = previous {
            for item in previous.items {
                self.release_key(item.key);
            }
        }
    }

    pub fn get_json(&self) -> Value {
        self.get_json_with_threshold(WARNING_THRESHOLD)
    }

    pub fn get_json_with_threshold(&self, threshold: i64) -> Value {
        self.inner.with_state_mut(|state| {
            let now = self.inner.clock.now();
            let mut root = Map::new();
            for (label, list) in [
                ("inbound", &state.inbound),
                ("outbound", &state.outbound),
                ("admin", &state.admin),
            ] {
                for key in list {
                    if let Some(entry) = state.entries.get_mut(key) {
                        let local_balance = entry.local_balance.value(now);
                        if local_balance + entry.remote_balance >= threshold {
                            root.insert(
                                entry.to_string(),
                                json!({
                                    "local": local_balance,
                                    "remote": entry.remote_balance,
                                    "type": label,
                                }),
                            );
                        }
                    }
                }
            }
            Value::Object(root)
        })
    }

    pub fn on_write(&self) -> Value {
        self.inner.with_state_mut(|state| {
            let now = self.inner.clock.now();
            let inbound = state.inbound.clone();
            let outbound = state.outbound.clone();
            let admin = state.admin.clone();
            let inactive = state.inactive.clone();
            let mut root = Map::new();
            root.insert("inbound".to_string(), write_list(state, &inbound, now));
            root.insert("outbound".to_string(), write_list(state, &outbound, now));
            root.insert("admin".to_string(), write_list(state, &admin, now));
            root.insert("inactive".to_string(), write_list(state, &inactive, now));
            Value::Object(root)
        })
    }

    pub fn periodic_activity(&self) {
        let expired_imports = self.inner.with_state_mut(|state| {
            let now = self.inner.clock.now();
            while let Some(key) = state.inactive.front().copied() {
                let Some(entry) = state.entries.get(&key) else {
                    state.inactive.pop_front();
                    continue;
                };
                let Some(when_expires) = entry.when_expires else {
                    break;
                };
                if when_expires > now {
                    break;
                }
                let entry_string = entry.to_string();
                self.inner
                    .journal
                    .log(JournalLevel::Debug, &format!("Expired {entry_string}"));
                state.inactive.pop_front();
                state.entries.remove(&key);
            }

            let mut expired = Vec::new();
            let origins: Vec<String> = state
                .imports
                .iter()
                .filter_map(|(origin, import)| {
                    (import.when_expires <= now).then_some(origin.clone())
                })
                .collect();
            for origin in origins {
                if let Some(import) = state.imports.remove(&origin) {
                    for item in &import.items {
                        if let Some(entry) = state.entries.get_mut(&item.key) {
                            entry.remote_balance -= item.balance;
                        }
                    }
                    expired.push(import);
                }
            }

            expired
        });

        for import in expired_imports {
            for item in import.items {
                self.release_key(item.key);
            }
        }
    }

    fn new_endpoint(&self, kind: Kind, address: SocketAddr) -> Consumer {
        let key = Key::new(kind, address);
        let entry_string = self.inner.with_state_mut(|state| {
            let now = self.inner.clock.now();
            let _ = self.inner.create_or_acquire_key_locked(state, key, now);
            self.inner.entry_string_locked(state, key)
        });

        let label = match kind {
            Kind::Inbound => "New inbound endpoint",
            Kind::Outbound => "New outbound endpoint",
            Kind::Unlimited => "New unlimited endpoint",
        };
        self.inner
            .journal
            .log(JournalLevel::Debug, &format!("{label} {entry_string}"));

        Consumer {
            inner: Some(Arc::clone(&self.inner)),
            key: Some(key),
        }
    }

    fn release_key(&self, key: Key) {
        let entry_string = self.inner.release_key(key);
        if let Some(entry_string) = entry_string {
            self.inner
                .journal
                .log(JournalLevel::Debug, &format!("Inactive {entry_string}"));
        }
    }
}

impl Drop for ResourceManager {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.stop();
        }
    }
}

#[derive(Default)]
pub struct Consumer {
    inner: Option<Arc<ResourceManagerInner>>,
    key: Option<Key>,
}

impl Clone for Consumer {
    fn clone(&self) -> Self {
        let Some(inner) = &self.inner else {
            return Self::default();
        };
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        let now = inner.clock.now();
        inner.with_state_mut(|state| {
            inner.create_or_acquire_key_locked(state, key, now);
        });

        Self {
            inner: Some(Arc::clone(inner)),
            key: Some(key),
        }
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        let (Some(inner), Some(key)) = (&self.inner, self.key) else {
            return;
        };
        let now = inner.clock.now();
        let entry_string = inner.with_state_mut(|state| inner.release_key_locked(state, key, now));
        if let Some(entry_string) = entry_string {
            inner
                .journal
                .log(JournalLevel::Debug, &format!("Inactive {entry_string}"));
        }
    }
}

impl fmt::Display for Consumer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string())
    }
}

impl Consumer {
    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        let Some(inner) = &self.inner else {
            return "(none)".to_owned();
        };
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        inner.with_state_mut(|state| inner.entry_string_locked(state, key))
    }

    pub fn is_unlimited(&self) -> bool {
        let Some(inner) = &self.inner else {
            return false;
        };
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        inner.with_state_mut(|state| {
            state
                .entries
                .get(&key)
                .is_some_and(|entry| entry.is_unlimited())
        })
    }

    pub fn disposition(&self) -> Disposition {
        let Some(inner) = &self.inner else {
            return Disposition::Ok;
        };
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        let fee = Charge::new(0, "");
        inner.with_state_mut(|state| inner.charge_locked_state(state, key, &fee, None, true))
    }

    pub fn charge(&self, fee: Charge) -> Disposition {
        self.charge_with_context(fee, "")
    }

    pub fn charge_with_context(&self, fee: Charge, context: impl Into<String>) -> Disposition {
        let Some(inner) = &self.inner else {
            return Disposition::Ok;
        };
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        let context = context.into();
        let context = (!context.is_empty()).then_some(context);
        inner.with_state_mut(|state| {
            inner.charge_locked_state(state, key, &fee, context.as_deref(), false)
        })
    }

    pub fn warn(&self) -> bool {
        let inner = self
            .inner
            .as_ref()
            .expect("resource consumer must be initialized before warning");
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        inner.with_state_mut(|state| inner.warn_locked(state, key))
    }

    pub fn disconnect(&self, journal: &dyn ResourceJournal) -> bool {
        let inner = self
            .inner
            .as_ref()
            .expect("resource consumer must be initialized before disconnect");
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        let (disconnected, entry_string) = inner.with_state_mut(|state| {
            let disconnected = inner.disconnect_locked(state, key);
            let entry_string = disconnected.then(|| inner.entry_string_locked(state, key));
            (disconnected, entry_string)
        });
        if let Some(entry_string) = entry_string {
            journal.log(
                JournalLevel::Debug,
                &format!("disconnecting {entry_string}"),
            );
        }
        disconnected
    }

    pub fn disconnect_with_manager_journal(&self) -> bool {
        let inner = self
            .inner
            .as_ref()
            .expect("resource consumer must be initialized before disconnect");
        self.disconnect(inner.journal.as_ref())
    }

    pub fn balance(&self) -> i64 {
        let inner = self
            .inner
            .as_ref()
            .expect("resource consumer must be initialized before checking balance");
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        inner.with_state_mut(|state| inner.balance_locked(state, key))
    }

    pub fn set_public_key(&self, public_key: PublicKey) {
        let inner = self
            .inner
            .as_ref()
            .expect("resource consumer must be initialized before setting public key");
        let key = self
            .key
            .expect("resource consumer key must exist with inner state");
        inner.with_state_mut(|state| {
            inner.set_public_key_locked(state, key, public_key);
        });
    }

    pub fn elevate(&self, name: impl Into<String>) {
        // The the reference implementation oracle declares `Consumer::elevate(...)` but ships no
        // implementation or backing named-endpoint entry kind to mirror yet.
        // Keep the surface present without inventing non-oracle behavior.
        let _ = name.into();
    }
}

struct ResourceManagerInner {
    clock: Arc<dyn ResourceClock>,
    journal: Arc<dyn ResourceJournal>,
    warn_meter: Arc<dyn ResourceMeter>,
    drop_meter: Arc<dyn ResourceMeter>,
    state: Mutex<ResourceState>,
    stop: Mutex<bool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    condvar: Condvar,
}

impl ResourceManagerInner {
    fn new(
        collector: Arc<dyn ResourceCollector>,
        journal: Arc<dyn ResourceJournal>,
        clock: Arc<dyn ResourceClock>,
    ) -> Self {
        Self {
            warn_meter: collector.make_meter("warn"),
            drop_meter: collector.make_meter("drop"),
            clock,
            journal,
            state: Mutex::new(ResourceState::new()),
            stop: Mutex::new(false),
            thread: Mutex::new(None),
            condvar: Condvar::new(),
        }
    }

    fn with_state_mut<R>(&self, f: impl FnOnce(&mut ResourceState) -> R) -> R {
        let mut state = self
            .state
            .lock()
            .expect("resource state mutex must not be poisoned");
        f(&mut state)
    }

    fn start(self: &Arc<Self>) {
        let mut thread_guard = self
            .thread
            .lock()
            .expect("resource thread mutex must not be poisoned");
        if thread_guard.is_some() {
            return;
        }
        *self
            .stop
            .lock()
            .expect("resource stop mutex must not be poisoned") = false;

        let inner = Arc::clone(self);
        let handle = thread::Builder::new()
            .name("Resource::Mngr".to_owned())
            .spawn(move || inner.run())
            .expect("resource manager thread should start");
        *thread_guard = Some(handle);
    }

    fn stop(&self) {
        {
            let mut stop = self
                .stop
                .lock()
                .expect("resource stop mutex must not be poisoned");
            if *stop {
                // A previous stop already happened.
            } else {
                *stop = true;
                self.condvar.notify_all();
            }
        }

        if let Some(handle) = self
            .thread
            .lock()
            .expect("resource thread mutex must not be poisoned")
            .take()
        {
            let _ = handle.join();
        }
    }

    fn run(self: Arc<Self>) {
        loop {
            self.periodic_activity();

            let stop_guard = self
                .stop
                .lock()
                .expect("resource stop mutex must not be poisoned");
            if *stop_guard {
                break;
            }
            let (stop_guard, _) = self
                .condvar
                .wait_timeout(stop_guard, std::time::Duration::from_secs(1))
                .expect("resource condvar wait should succeed");
            if *stop_guard {
                break;
            }
        }
    }

    fn create_or_acquire_key_locked(
        &self,
        state: &mut ResourceState,
        key: Key,
        now: Instant,
    ) -> Key {
        let was_new = !state.entries.contains_key(&key);
        let entry = state
            .entries
            .entry(key)
            .or_insert_with(|| Entry::new(key, now));
        let was_inactive = entry.refcount == 0;
        entry.refcount += 1;
        if was_inactive {
            entry.when_expires = None;
            remove_key(&mut state.inactive, key);
            list_for_kind(state, key.kind).push_back(key);
        } else if was_new {
            list_for_kind(state, key.kind).push_back(key);
        }
        key
    }

    fn release_key_locked(
        &self,
        state: &mut ResourceState,
        key: Key,
        now: Instant,
    ) -> Option<String> {
        let (entry_string, became_inactive) = {
            let entry = state.entries.get_mut(&key)?;
            if entry.refcount == 0 {
                return None;
            }
            entry.refcount -= 1;
            let entry_string = entry.to_string();
            let became_inactive = entry.refcount == 0;
            if became_inactive {
                entry.when_expires = Some(now + SECONDS_UNTIL_EXPIRATION);
            }
            (entry_string, became_inactive)
        };

        if became_inactive {
            remove_key(list_for_kind(state, key.kind), key);
            state.inactive.push_back(key);
            Some(entry_string)
        } else {
            None
        }
    }

    fn entry_string_locked(&self, state: &mut ResourceState, key: Key) -> String {
        state
            .entries
            .get(&key)
            .expect("resource entry must exist")
            .to_string()
    }

    fn balance_locked(&self, state: &mut ResourceState, key: Key) -> i64 {
        let now = self.clock.now();
        state
            .entries
            .get_mut(&key)
            .expect("resource entry must exist")
            .balance(now)
    }

    fn set_public_key_locked(&self, state: &mut ResourceState, key: Key, public_key: PublicKey) {
        state
            .entries
            .get_mut(&key)
            .expect("resource entry must exist")
            .public_key = Some(public_key);
    }

    fn charge_locked_state(
        &self,
        state: &mut ResourceState,
        key: Key,
        fee: &Charge,
        context: Option<&str>,
        allow_unlimited: bool,
    ) -> Disposition {
        let now = self.clock.now();
        let entry = state
            .entries
            .get_mut(&key)
            .expect("resource entry must exist");
        if !allow_unlimited && entry.is_unlimited() {
            return Disposition::Ok;
        }

        let level = match fee.cost() {
            cost if cost >= FEE_LOG_AS_WARN => JournalLevel::Warn,
            cost if cost >= FEE_LOG_AS_INFO => JournalLevel::Info,
            cost if cost >= FEE_LOG_AS_DEBUG => JournalLevel::Debug,
            _ => JournalLevel::Trace,
        };
        let context_suffix = context
            .filter(|context| !context.is_empty())
            .map(|context| format!(" ({context})"))
            .unwrap_or_default();
        self.journal.log(
            level,
            &format!(
                "Charging {} for {}{}",
                entry.to_string(),
                fee,
                context_suffix
            ),
        );

        disposition(entry.add(i64::from(fee.cost()), now))
    }

    fn warn_locked(&self, state: &mut ResourceState, key: Key) -> bool {
        let now = self.clock.now();
        let entry_string = {
            let entry = state
                .entries
                .get_mut(&key)
                .expect("resource entry must exist");
            if entry.is_unlimited() {
                return false;
            }

            if entry.balance(now) >= WARNING_THRESHOLD && entry.last_warning_time != Some(now) {
                entry.last_warning_time = Some(now);
                entry.to_string()
            } else {
                return false;
            }
        };

        let _ = self.charge_locked_state(state, key, &FEE_WARNING, None, true);
        self.journal.log(
            JournalLevel::Info,
            &format!("Load warning: {}", entry_string),
        );
        self.warn_meter.increment();
        true
    }

    fn disconnect_locked(&self, state: &mut ResourceState, key: Key) -> bool {
        let now = self.clock.now();
        let (entry_string, balance) = {
            let entry = state
                .entries
                .get_mut(&key)
                .expect("resource entry must exist");
            if entry.is_unlimited() {
                return false;
            }

            let balance = entry.balance(now);
            if balance >= DROP_THRESHOLD {
                (entry.to_string(), balance)
            } else {
                return false;
            }
        };

        self.journal.log(
            JournalLevel::Warn,
            &format!(
                "Consumer entry {} dropped with balance {balance} at or above drop threshold {DROP_THRESHOLD}",
                entry_string
            ),
        );
        let _ = self.charge_locked_state(state, key, &FEE_DROP, None, true);
        self.drop_meter.increment();
        true
    }

    fn periodic_activity(&self) {
        let expired_imports = self.with_state_mut(|state| {
            let now = self.clock.now();

            while let Some(key) = state.inactive.front().copied() {
                let Some(entry) = state.entries.get(&key) else {
                    state.inactive.pop_front();
                    continue;
                };
                let Some(when_expires) = entry.when_expires else {
                    break;
                };
                if when_expires > now {
                    break;
                }

                self.journal.log(
                    JournalLevel::Debug,
                    &format!("Expired {}", entry.to_string()),
                );
                state.inactive.pop_front();
                state.entries.remove(&key);
            }

            let mut expired = Vec::new();
            let origins: Vec<String> = state
                .imports
                .iter()
                .filter_map(|(origin, import)| {
                    (import.when_expires <= now).then_some(origin.clone())
                })
                .collect();
            for origin in origins {
                if let Some(import) = state.imports.remove(&origin) {
                    for item in &import.items {
                        if let Some(entry) = state.entries.get_mut(&item.key) {
                            entry.remote_balance -= item.balance;
                        }
                    }
                    expired.push(import);
                }
            }

            expired
        });

        for import in expired_imports {
            for item in import.items {
                self.release_key(item.key);
            }
        }
    }

    fn release_key(&self, key: Key) -> Option<String> {
        self.with_state_mut(|state| {
            let now = self.clock.now();
            self.release_key_locked(state, key, now)
        })
    }
}

fn disposition(balance: i64) -> Disposition {
    if balance >= DROP_THRESHOLD {
        Disposition::Drop
    } else if balance >= WARNING_THRESHOLD {
        Disposition::Warn
    } else {
        Disposition::Ok
    }
}

fn list_for_kind(state: &mut ResourceState, kind: Kind) -> &mut VecDeque<Key> {
    match kind {
        Kind::Inbound => &mut state.inbound,
        Kind::Outbound => &mut state.outbound,
        Kind::Unlimited => &mut state.admin,
    }
}

fn remove_key(list: &mut VecDeque<Key>, key: Key) {
    if let Some(index) = list.iter().position(|candidate| *candidate == key) {
        let _ = list.remove(index);
    }
}

fn write_list(state: &mut ResourceState, list: &VecDeque<Key>, now: Instant) -> Value {
    let mut items = Vec::new();
    for key in list {
        if let Some(entry) = state.entries.get_mut(key) {
            let mut object = Map::new();
            if entry.refcount != 0 {
                object.insert("count".to_string(), json!(entry.refcount));
            }
            object.insert("name".to_string(), json!(entry.to_string()));
            object.insert("balance".to_string(), json!(entry.balance(now)));
            if entry.remote_balance != 0 {
                object.insert("remote_balance".to_string(), json!(entry.remote_balance));
            }
            items.push(Value::Object(object));
        }
    }
    Value::Array(items)
}

pub fn make_manager(
    collector: Arc<dyn ResourceCollector>,
    journal: Arc<dyn ResourceJournal>,
) -> ResourceManager {
    let manager = ResourceManager::new_with_clock(collector, journal, Arc::new(SystemStopwatch));
    manager.start();
    manager
}

#[cfg(test)]
mod tests {
    use crate::types::DecayingSample;
    use std::time::{Duration, Instant};

    #[test]
    fn decaying_sample_applies_integer_decay() {
        let start = Instant::now();
        let mut sample = DecayingSample::<32>::new(start);

        assert_eq!(sample.add(32, start), 1);
        assert_eq!(sample.value(start), 1);

        let later = start + Duration::from_secs(1);
        assert_eq!(sample.value(later), 0);
    }

    #[test]
    fn decaying_sample_resets_after_large_gap() {
        let start = Instant::now();
        let mut sample = DecayingSample::<32>::new(start);
        let later = start + Duration::from_secs(200);

        assert_eq!(sample.add(10_000, start), 312);
        assert_eq!(sample.value(later), 0);
    }
}
