//! Owner-level `HashRouter` compatibility port for the current `xrpl/core` migration.
//!
//! This keeps the the reference implementation owner contract visible:
//! - monotonic internal clock ownership,
//! - mutex-owned suppression state,
//! - hardened hashed storage rather than ordered-map stand-ins,
//! - create-on-read and touch-on-access semantics around the landed entry
//!   rules.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use basics::basic_config::{BasicConfig, Section, set};
use basics::{base_uint::Uint256, unordered_containers::HardenedHashMap};

use crate::{
    HashRouterEntry, HashRouterFlags, PeerShortId, any,
    hash_router_flags::{
        HASH_ROUTER_DEFAULT_HOLD_TIME, HASH_ROUTER_DEFAULT_RELAY_TIME, validate_hash_router_setup,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashRouterSetup {
    pub hold_time: Duration,
    pub relay_time: Duration,
}

impl Default for HashRouterSetup {
    fn default() -> Self {
        Self {
            hold_time: HASH_ROUTER_DEFAULT_HOLD_TIME,
            relay_time: HASH_ROUTER_DEFAULT_RELAY_TIME,
        }
    }
}

impl HashRouterSetup {
    pub fn new(hold_time: Duration, relay_time: Duration) -> Result<Self, &'static str> {
        validate_hash_router_setup(hold_time, relay_time)?;
        Ok(Self {
            hold_time,
            relay_time,
        })
    }

    pub fn from_section(section: &Section) -> Result<Self, String> {
        let mut setup = Self::default();
        let mut hold_time = 0i32;
        let mut relay_time = 0i32;

        if set(&mut hold_time, "hold_time", section) {
            if hold_time < 12 {
                return Err(
                    "HashRouter hold time must be at least 12 seconds (the approximate validation time for three ledgers)."
                        .to_owned(),
                );
            }
            setup.hold_time = Duration::from_secs(
                u64::try_from(hold_time).expect("validated hold time must fit u64"),
            );
        }

        if set(&mut relay_time, "relay_time", section) {
            if relay_time < 8 {
                return Err(
                    "HashRouter relay time must be at least 8 seconds (the approximate validation time for two ledgers)."
                        .to_owned(),
                );
            }
            setup.relay_time = Duration::from_secs(
                u64::try_from(relay_time).expect("validated relay time must fit u64"),
            );
        }

        validate_hash_router_setup(setup.hold_time, setup.relay_time)
            .map_err(std::borrow::ToOwned::to_owned)?;
        Ok(setup)
    }

    pub fn from_basic_config(config: &BasicConfig) -> Result<Self, String> {
        Self::from_section(config.section("hashrouter"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AgedEntry {
    entry: HashRouterEntry,
    touched_at: Duration,
}

#[derive(Debug, Default)]
struct HashRouterState {
    suppression_map: HardenedHashMap<Uint256, AgedEntry>,
}

impl HashRouterState {
    fn entry_count(&self) -> usize {
        self.suppression_map.len()
    }

    fn expire(&mut self, now: Duration, hold_time: Duration) {
        self.suppression_map
            .retain(|_, aged| aged.touched_at + hold_time > now);
    }

    fn emplace(
        &mut self,
        key: Uint256,
        now: Duration,
        hold_time: Duration,
    ) -> (&mut HashRouterEntry, bool) {
        if self.suppression_map.contains_key(&key) {
            let aged = self
                .suppression_map
                .get_mut(&key)
                .expect("entry should exist after contains_key");
            aged.touched_at = now;
            return (&mut aged.entry, false);
        }

        self.expire(now, hold_time);

        let aged = self
            .suppression_map
            .entry(key)
            .or_insert_with(|| AgedEntry {
                entry: HashRouterEntry::default(),
                touched_at: now,
            });
        aged.touched_at = now;
        (&mut aged.entry, true)
    }
}

pub trait HashRouterClock: Send + Sync {
    fn now(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemHashRouterClock {
    start: Instant,
}

impl SystemHashRouterClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemHashRouterClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HashRouterClock for SystemHashRouterClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
}

pub struct HashRouter {
    setup: HashRouterSetup,
    clock: Arc<dyn HashRouterClock>,
    state: Mutex<HashRouterState>,
}

impl Default for HashRouter {
    fn default() -> Self {
        Self::new(HashRouterSetup::default())
    }
}

impl HashRouter {
    pub fn new(setup: HashRouterSetup) -> Self {
        Self::with_clock(setup, Arc::new(SystemHashRouterClock::default()))
    }

    pub fn with_clock(setup: HashRouterSetup, clock: Arc<dyn HashRouterClock>) -> Self {
        Self {
            setup,
            clock,
            state: Mutex::new(HashRouterState::default()),
        }
    }

    pub const fn setup(&self) -> HashRouterSetup {
        self.setup
    }

    pub fn entry_count(&self) -> usize {
        self.lock_state().entry_count()
    }

    fn lock_state(&self) -> MutexGuard<'_, HashRouterState> {
        self.state.lock().expect("hash router mutex poisoned")
    }

    fn now(&self) -> Duration {
        self.clock.now()
    }

    pub fn add_suppression(&self, key: Uint256) {
        let now = self.now();
        let hold_time = self.setup.hold_time;
        let _ = self.lock_state().emplace(key, now, hold_time);
    }

    pub fn add_suppression_peer(&self, key: Uint256, peer: PeerShortId) -> bool {
        self.add_suppression_peer_with_status(key, peer).0
    }

    pub fn add_suppression_peer_with_status(
        &self,
        key: Uint256,
        peer: PeerShortId,
    ) -> (bool, Option<Duration>) {
        let now = self.now();
        let hold_time = self.setup.hold_time;
        let mut state = self.lock_state();
        let (entry, created) = state.emplace(key, now, hold_time);
        entry.add_peer(peer);
        (created, entry.relayed())
    }

    pub fn add_suppression_peer_and_get_flags(
        &self,
        key: Uint256,
        peer: PeerShortId,
    ) -> (bool, HashRouterFlags) {
        let now = self.now();
        let hold_time = self.setup.hold_time;
        let mut state = self.lock_state();
        let (entry, created) = state.emplace(key, now, hold_time);
        entry.add_peer(peer);
        (created, entry.flags())
    }

    pub fn should_process(
        &self,
        key: Uint256,
        peer: PeerShortId,
        tx_interval: Duration,
    ) -> (bool, HashRouterFlags) {
        let now = self.now();
        let hold_time = self.setup.hold_time;
        let mut state = self.lock_state();
        let (entry, _) = state.emplace(key, now, hold_time);
        entry.add_peer(peer);
        let flags = entry.flags();
        (entry.should_process(now, tx_interval), flags)
    }

    pub fn set_flags(&self, key: Uint256, flags: HashRouterFlags) -> bool {
        assert!(any(flags), "HashRouter::setFlags requires non-zero input");

        let now = self.now();
        let hold_time = self.setup.hold_time;
        let mut state = self.lock_state();
        let (entry, _) = state.emplace(key, now, hold_time);
        entry.set_flags(flags)
    }

    pub fn get_flags(&self, key: Uint256) -> HashRouterFlags {
        let now = self.now();
        let hold_time = self.setup.hold_time;
        self.lock_state().emplace(key, now, hold_time).0.flags()
    }

    pub fn should_relay(&self, key: Uint256) -> Option<BTreeSet<PeerShortId>> {
        let now = self.now();
        let hold_time = self.setup.hold_time;
        let relay_time = self.setup.relay_time;
        let mut state = self.lock_state();
        let (entry, _) = state.emplace(key, now, hold_time);
        if !entry.should_relay(now, relay_time) {
            return None;
        }
        Some(entry.release_peer_set())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use basics::base_uint::Uint256;

    use super::{HashRouter, HashRouterClock, HashRouterSetup};
    use crate::HashRouterFlags;

    #[derive(Debug)]
    struct ManualHashRouterClock {
        now: Mutex<Duration>,
    }

    impl ManualHashRouterClock {
        fn new(now: Duration) -> Self {
            Self {
                now: Mutex::new(now),
            }
        }

        fn advance(&self, delta: Duration) {
            let mut now = self.now.lock().expect("manual hash-router clock poisoned");
            *now += delta;
        }
    }

    impl HashRouterClock for ManualHashRouterClock {
        fn now(&self) -> Duration {
            *self.now.lock().expect("manual hash-router clock poisoned")
        }
    }

    fn key(value: u64) -> Uint256 {
        Uint256::from_u64(value)
    }

    fn router_with_clock(
        setup: HashRouterSetup,
        start: Duration,
    ) -> (HashRouter, Arc<ManualHashRouterClock>) {
        let clock = Arc::new(ManualHashRouterClock::new(start));
        (HashRouter::with_clock(setup, clock.clone()), clock)
    }

    #[test]
    fn add_suppression_creates_entry_and_get_flags_reads_same_entry() {
        let (router, _) = router_with_clock(HashRouterSetup::default(), Duration::from_secs(5));
        router.add_suppression(key(1));

        assert_eq!(router.entry_count(), 1);
        assert_eq!(router.get_flags(key(1)), HashRouterFlags::UNDEFINED);
        assert_eq!(router.entry_count(), 1);
    }

    #[test]
    fn add_suppression_peer_with_status_matches_creation_and_relay_status() {
        let (router, clock) = router_with_clock(HashRouterSetup::default(), Duration::from_secs(1));

        let (created, relayed) = router.add_suppression_peer_with_status(key(1), 7);
        assert!(created);
        assert_eq!(relayed, None);

        clock.advance(Duration::from_secs(1));
        assert_eq!(router.should_relay(key(1)), Some(BTreeSet::from([7])));

        clock.advance(Duration::from_secs(1));
        let (created_again, relayed_again) = router.add_suppression_peer_with_status(key(1), 9);
        assert!(!created_again);
        assert_eq!(relayed_again, Some(Duration::from_secs(2)));
    }

    #[test]
    fn add_suppression_peer_and_get_flags_shape() {
        let (router, _) = router_with_clock(HashRouterSetup::default(), Duration::from_secs(1));
        assert!(router.set_flags(key(5), HashRouterFlags::BAD | HashRouterFlags::PRIVATE1));

        let (created, flags) = router.add_suppression_peer_and_get_flags(key(5), 11);
        assert!(!created);
        assert_eq!(flags, HashRouterFlags::BAD | HashRouterFlags::PRIVATE1);
    }

    #[test]
    fn should_process_adds_peer_returns_flags_and_respects_interval() {
        let (router, clock) = router_with_clock(HashRouterSetup::default(), Duration::from_secs(1));
        assert!(router.set_flags(key(8), HashRouterFlags::TRUSTED));

        clock.advance(Duration::from_secs(4));
        let (first, flags) = router.should_process(key(8), 3, Duration::from_secs(10));
        assert!(first);
        assert_eq!(flags, HashRouterFlags::TRUSTED);

        clock.advance(Duration::from_secs(9));
        let (second, second_flags) = router.should_process(key(8), 4, Duration::from_secs(10));
        assert!(!second);
        assert_eq!(second_flags, HashRouterFlags::TRUSTED);

        clock.advance(Duration::from_secs(1));
        let (third, _) = router.should_process(key(8), 5, Duration::from_secs(10));
        assert!(third);
    }

    #[test]
    fn should_relay_uses_setup_relay_time_and_clears_peer_set() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(300),
                relay_time: Duration::from_secs(30),
            },
            Duration::from_secs(1),
        );
        router.add_suppression_peer(key(2), 13);
        clock.advance(Duration::from_secs(1));
        router.add_suppression_peer(key(2), 17);

        clock.advance(Duration::from_secs(8));
        assert_eq!(router.should_relay(key(2)), Some(BTreeSet::from([13, 17])));
        clock.advance(Duration::from_secs(29));
        assert_eq!(router.should_relay(key(2)), None);
        clock.advance(Duration::from_secs(1));
        assert_eq!(router.should_relay(key(2)), Some(BTreeSet::new()));
    }

    #[test]
    fn emplace_expires_old_entries_before_inserting_new_ones() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(10),
                relay_time: Duration::from_secs(30),
            },
            Duration::from_secs(0),
        );
        router.add_suppression(key(1));
        clock.advance(Duration::from_secs(9));
        router.add_suppression(key(2));
        assert_eq!(router.entry_count(), 2);

        clock.advance(Duration::from_secs(2));
        router.add_suppression(key(3));
        assert_eq!(router.entry_count(), 2);
        assert_eq!(router.get_flags(key(2)), HashRouterFlags::UNDEFINED);
        assert_eq!(router.get_flags(key(3)), HashRouterFlags::UNDEFINED);
    }

    #[test]
    fn existing_entry_touch_prevents_expiry_on_later_insert() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(10),
                relay_time: Duration::from_secs(30),
            },
            Duration::from_secs(0),
        );
        router.add_suppression(key(1));
        clock.advance(Duration::from_secs(9));
        let _ = router.get_flags(key(1));
        clock.advance(Duration::from_secs(2));
        router.add_suppression(key(2));

        assert_eq!(router.entry_count(), 2);
        assert_eq!(router.get_flags(key(1)), HashRouterFlags::UNDEFINED);
        assert_eq!(router.get_flags(key(2)), HashRouterFlags::UNDEFINED);
    }

    #[test]
    fn non_expiration_sequence_matches_internal_clock_owner() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(2),
                relay_time: Duration::from_secs(1),
            },
            Duration::from_secs(0),
        );

        let key1 = Uint256::from_u64(HashRouterFlags::PRIVATE1.bits().into());
        let key2 = Uint256::from_u64(HashRouterFlags::PRIVATE2.bits().into());
        let key3 = Uint256::from_u64(HashRouterFlags::PRIVATE3.bits().into());

        router.set_flags(key1, HashRouterFlags::PRIVATE1);
        assert_eq!(router.get_flags(key1), HashRouterFlags::PRIVATE1);
        router.set_flags(key2, HashRouterFlags::PRIVATE2);
        assert_eq!(router.get_flags(key2), HashRouterFlags::PRIVATE2);

        clock.advance(Duration::from_secs(1));
        assert_eq!(router.get_flags(key1), HashRouterFlags::PRIVATE1);

        clock.advance(Duration::from_secs(1));
        router.set_flags(key3, HashRouterFlags::PRIVATE3);
        assert_eq!(router.get_flags(key1), HashRouterFlags::PRIVATE1);
        assert_eq!(router.get_flags(key2), HashRouterFlags::UNDEFINED);
    }

    #[test]
    fn expiration_sequence_matches_internal_clock_owner() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(2),
                relay_time: Duration::from_secs(1),
            },
            Duration::from_secs(0),
        );

        let key1 = Uint256::from_u64(HashRouterFlags::PRIVATE1.bits().into());
        let key2 = Uint256::from_u64(HashRouterFlags::PRIVATE2.bits().into());
        let key3 = Uint256::from_u64(HashRouterFlags::PRIVATE3.bits().into());
        let key4 = Uint256::from_u64(HashRouterFlags::PRIVATE4.bits().into());

        assert!(router.set_flags(key1, HashRouterFlags::BAD));
        assert_eq!(router.get_flags(key1), HashRouterFlags::BAD);

        clock.advance(Duration::from_secs(1));
        assert!(router.set_flags(key2, HashRouterFlags::PRIVATE5));
        assert_eq!(router.get_flags(key1), HashRouterFlags::BAD);
        assert_eq!(router.get_flags(key2), HashRouterFlags::PRIVATE5);

        clock.advance(Duration::from_secs(1));
        assert_eq!(router.get_flags(key2), HashRouterFlags::PRIVATE5);

        clock.advance(Duration::from_secs(1));
        assert!(router.set_flags(key3, HashRouterFlags::BAD));
        assert_eq!(router.get_flags(key1), HashRouterFlags::UNDEFINED);
        assert_eq!(router.get_flags(key2), HashRouterFlags::PRIVATE5);
        assert_eq!(router.get_flags(key3), HashRouterFlags::BAD);

        clock.advance(Duration::from_secs(1));
        assert!(router.set_flags(key1, HashRouterFlags::SAVED));
        assert_eq!(router.get_flags(key1), HashRouterFlags::SAVED);
        assert_eq!(router.get_flags(key2), HashRouterFlags::PRIVATE5);
        assert_eq!(router.get_flags(key3), HashRouterFlags::BAD);

        clock.advance(Duration::from_secs(2));
        assert!(router.set_flags(key4, HashRouterFlags::TRUSTED));
        assert_eq!(router.get_flags(key1), HashRouterFlags::UNDEFINED);
        assert_eq!(router.get_flags(key2), HashRouterFlags::UNDEFINED);
        assert_eq!(router.get_flags(key3), HashRouterFlags::UNDEFINED);
        assert_eq!(router.get_flags(key4), HashRouterFlags::TRUSTED);
    }

    #[test]
    fn suppression_sequence_matches_internal_clock_owner() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(2),
                relay_time: Duration::from_secs(1),
            },
            Duration::from_secs(0),
        );

        let key1 = Uint256::from_u64(1);
        let key2 = Uint256::from_u64(2);
        let key3 = Uint256::from_u64(3);
        let key4 = Uint256::from_u64(4);

        router.add_suppression(key1);
        assert!(router.add_suppression_peer(key2, 15));
        let (created, flags) = router.add_suppression_peer_and_get_flags(key3, 20);
        assert!(created);
        assert_eq!(flags, HashRouterFlags::UNDEFINED);

        clock.advance(Duration::from_secs(1));
        assert!(!router.add_suppression_peer(key1, 2));
        assert!(!router.add_suppression_peer(key2, 3));
        let (created_again, flags_again) = router.add_suppression_peer_and_get_flags(key3, 4);
        assert!(!created_again);
        assert_eq!(flags_again, HashRouterFlags::UNDEFINED);
        assert!(router.add_suppression_peer(key4, 5));
    }

    #[test]
    fn relay_sequence_matches_internal_clock_owner() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(50),
                relay_time: Duration::from_secs(1),
            },
            Duration::from_secs(0),
        );
        let key1 = Uint256::from_u64(1);

        assert_eq!(router.should_relay(key1), Some(BTreeSet::new()));
        router.add_suppression_peer(key1, 1);
        router.add_suppression_peer(key1, 3);
        router.add_suppression_peer(key1, 5);
        assert_eq!(router.should_relay(key1), None);

        clock.advance(Duration::from_secs(1));
        assert_eq!(router.should_relay(key1), Some(BTreeSet::from([1, 3, 5])));
        router.add_suppression_peer(key1, 2);
        router.add_suppression_peer(key1, 4);
        assert_eq!(router.should_relay(key1), None);

        clock.advance(Duration::from_secs(1));
        assert_eq!(router.should_relay(key1), Some(BTreeSet::from([2, 4])));
        clock.advance(Duration::from_secs(1));
        assert_eq!(router.should_relay(key1), Some(BTreeSet::new()));
    }

    #[test]
    fn process_sequence_matches_internal_clock_owner() {
        let (router, clock) = router_with_clock(
            HashRouterSetup {
                hold_time: Duration::from_secs(5),
                relay_time: Duration::from_secs(1),
            },
            Duration::from_secs(0),
        );
        let key = Uint256::from_u64(1);

        let (first, flags) = router.should_process(key, 1, Duration::from_secs(1));
        assert!(first);
        assert_eq!(flags, HashRouterFlags::UNDEFINED);

        let (second, second_flags) = router.should_process(key, 1, Duration::from_secs(1));
        assert!(!second);
        assert_eq!(second_flags, HashRouterFlags::UNDEFINED);

        clock.advance(Duration::from_secs(2));
        let (third, third_flags) = router.should_process(key, 1, Duration::from_secs(1));
        assert!(third);
        assert_eq!(third_flags, HashRouterFlags::UNDEFINED);
    }

    #[test]
    fn hash_router_setup_new_defaults_and_validation() {
        let default_setup = HashRouterSetup::default();
        assert_eq!(default_setup.hold_time, Duration::from_secs(300));
        assert_eq!(default_setup.relay_time, Duration::from_secs(30));

        let custom_setup = HashRouterSetup::new(Duration::from_secs(600), Duration::from_secs(15))
            .expect("expected valid setup");
        assert_eq!(custom_setup.hold_time, Duration::from_secs(600));
        assert_eq!(custom_setup.relay_time, Duration::from_secs(15));
    }

    #[test]
    fn hash_router_setup_new_rejects_invalid_inputs_with_cpp_messages() {
        assert_eq!(
            HashRouterSetup::new(Duration::from_secs(60), Duration::from_secs(120)),
            Err("HashRouter relay time must be less than or equal to hold time")
        );
        assert_eq!(
            HashRouterSetup::new(Duration::from_secs(10), Duration::from_secs(120)),
            Err(
                "HashRouter hold time must be at least 12 seconds (the approximate validation time for three ledgers)."
            )
        );
        assert_eq!(
            HashRouterSetup::new(Duration::from_secs(500), Duration::from_secs(6)),
            Err(
                "HashRouter relay time must be at least 8 seconds (the approximate validation time for two ledgers)."
            )
        );
    }
}
