use basics::base_uint::Uint256;
use basics::basic_config::{BasicConfig, Section};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;
use xrpl_core::{HashRouter, HashRouterClock, HashRouterFlags, HashRouterSetup};

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
        let mut now = self.now.lock().expect("manual clock mutex poisoned");
        *now += delta;
    }
}

impl HashRouterClock for ManualHashRouterClock {
    fn now(&self) -> Duration {
        *self.now.lock().expect("manual clock mutex poisoned")
    }
}

#[test]
fn hash_router_setup_from_section_setup_hash_router_contract() {
    let mut defaults = Section::new("hashrouter");
    let default_setup = HashRouterSetup::from_section(&defaults).expect("default setup");
    assert_eq!(default_setup, HashRouterSetup::default());

    defaults.set("hold_time", "600");
    defaults.set("relay_time", "15");
    let custom = HashRouterSetup::from_section(&defaults).expect("custom setup");
    assert_eq!(custom.hold_time, Duration::from_secs(600));
    assert_eq!(custom.relay_time, Duration::from_secs(15));

    let mut bad_hold = Section::new("hashrouter");
    bad_hold.set("hold_time", "11");
    assert_eq!(
        HashRouterSetup::from_section(&bad_hold),
        Err("HashRouter hold time must be at least 12 seconds (the approximate validation time for three ledgers).".to_owned())
    );

    let mut bad_relay = Section::new("hashrouter");
    bad_relay.set("relay_time", "7");
    assert_eq!(
        HashRouterSetup::from_section(&bad_relay),
        Err("HashRouter relay time must be at least 8 seconds (the approximate validation time for two ledgers).".to_owned())
    );

    let mut bad_order = Section::new("hashrouter");
    bad_order.set("hold_time", "30");
    bad_order.set("relay_time", "31");
    assert_eq!(
        HashRouterSetup::from_section(&bad_order),
        Err("HashRouter relay time must be less than or equal to hold time".to_owned())
    );

    let mut garbage = Section::new("hashrouter");
    garbage.set("hold_time", "alice");
    garbage.set("relay_time", "bob");
    assert_eq!(
        HashRouterSetup::from_section(&garbage)
            .expect("garbage values should fall back to defaults"),
        HashRouterSetup::default()
    );

    let mut equal_times = Section::new("hashrouter");
    equal_times.set("hold_time", "30");
    equal_times.set("relay_time", "30");
    assert_eq!(
        HashRouterSetup::from_section(&equal_times)
            .expect("equal hold and relay should be accepted"),
        HashRouterSetup {
            hold_time: Duration::from_secs(30),
            relay_time: Duration::from_secs(30),
        }
    );
}

#[test]
fn hash_router_setup_from_basic_config_setup_hash_router_contract() {
    let mut config = BasicConfig::new();
    config.overwrite("hashrouter", "hold_time", "600");
    config.overwrite("hashrouter", "relay_time", "15");

    assert_eq!(
        HashRouterSetup::from_basic_config(&config).expect("setup from config"),
        HashRouterSetup {
            hold_time: Duration::from_secs(600),
            relay_time: Duration::from_secs(15),
        }
    );

    let mut garbage = BasicConfig::new();
    garbage.overwrite("hashrouter", "hold_time", "alice");
    garbage.overwrite("hashrouter", "relay_time", "bob");
    assert_eq!(
        HashRouterSetup::from_basic_config(&garbage)
            .expect("garbage config values should keep defaults"),
        HashRouterSetup::default()
    );
}

#[test]
fn hash_router_owner_methods_preserve_flags_peers_and_relay_state() {
    let clock = std::sync::Arc::new(ManualHashRouterClock::new(Duration::ZERO));
    let router = HashRouter::with_clock(
        HashRouterSetup {
            hold_time: Duration::from_secs(30),
            relay_time: Duration::from_secs(1),
        },
        clock.clone(),
    );
    let key = Uint256::from_u64(7);

    assert_eq!(router.get_flags(key), HashRouterFlags::UNDEFINED);
    assert!(!router.add_suppression_peer(key, 11));
    assert!(router.set_flags(key, HashRouterFlags::BAD | HashRouterFlags::TRUSTED));
    assert_eq!(
        router.get_flags(key),
        HashRouterFlags::BAD | HashRouterFlags::TRUSTED
    );

    let relayed = router
        .should_relay(key)
        .expect("first relay should happen immediately");
    assert_eq!(relayed, BTreeSet::from([11]));
    assert!(router.should_relay(key).is_none());
    clock.advance(Duration::from_secs(1));
    assert_eq!(router.should_relay(key), Some(BTreeSet::new()));
}

#[test]
fn hash_router_should_process_interval_gate() {
    let clock = std::sync::Arc::new(ManualHashRouterClock::new(Duration::ZERO));
    let router = HashRouter::with_clock(HashRouterSetup::default(), clock.clone());
    let key = Uint256::from_u64(9);

    assert!(router.set_flags(key, HashRouterFlags::TRUSTED));

    let (first, first_flags) = router.should_process(key, 21, Duration::from_secs(10));
    assert!(first);
    assert_eq!(first_flags, HashRouterFlags::TRUSTED);

    clock.advance(Duration::from_secs(9));
    let (second, second_flags) = router.should_process(key, 22, Duration::from_secs(10));
    assert!(!second);
    assert_eq!(second_flags, HashRouterFlags::TRUSTED);

    clock.advance(Duration::from_secs(1));
    let (third, third_flags) = router.should_process(key, 23, Duration::from_secs(10));
    assert!(third);
    assert_eq!(third_flags, HashRouterFlags::TRUSTED);
}

#[test]
fn hash_router_owner_methods_preserve_create_on_read_and_idempotent_set_flags() {
    let clock = std::sync::Arc::new(ManualHashRouterClock::new(Duration::ZERO));
    let router = HashRouter::with_clock(
        HashRouterSetup {
            hold_time: Duration::from_secs(2),
            relay_time: Duration::from_secs(1),
        },
        clock.clone(),
    );
    let key = Uint256::from_u64(11);

    assert_eq!(router.entry_count(), 0);
    assert_eq!(router.get_flags(key), HashRouterFlags::UNDEFINED);
    assert_eq!(router.entry_count(), 1);

    assert!(router.set_flags(key, HashRouterFlags::BAD));
    assert!(!router.set_flags(key, HashRouterFlags::BAD));
    assert_eq!(router.get_flags(key), HashRouterFlags::BAD);

    clock.advance(Duration::from_secs(3));
    let fresh = Uint256::from_u64(12);
    assert_eq!(router.get_flags(fresh), HashRouterFlags::UNDEFINED);
    assert_eq!(router.get_flags(key), HashRouterFlags::UNDEFINED);
}

#[test]
fn hash_router_expiration_happens_on_insert_but_touch_prevents_expiry() {
    let clock = std::sync::Arc::new(ManualHashRouterClock::new(Duration::ZERO));
    let router = HashRouter::with_clock(
        HashRouterSetup {
            hold_time: Duration::from_secs(2),
            relay_time: Duration::from_secs(1),
        },
        clock.clone(),
    );

    let key1 = Uint256::from_u64(1);
    let key2 = Uint256::from_u64(2);
    let key3 = Uint256::from_u64(3);

    assert!(router.set_flags(key1, HashRouterFlags::PRIVATE1));
    clock.advance(Duration::from_secs(1));
    assert!(router.set_flags(key2, HashRouterFlags::PRIVATE2));
    assert_eq!(router.get_flags(key1), HashRouterFlags::PRIVATE1);

    clock.advance(Duration::from_secs(1));
    assert_eq!(router.get_flags(key2), HashRouterFlags::PRIVATE2);

    clock.advance(Duration::from_secs(1));
    assert!(router.set_flags(key3, HashRouterFlags::PRIVATE3));
    assert_eq!(router.get_flags(key1), HashRouterFlags::UNDEFINED);
    assert_eq!(router.get_flags(key2), HashRouterFlags::PRIVATE2);
    assert_eq!(router.get_flags(key3), HashRouterFlags::PRIVATE3);
}

#[test]
fn hash_router_add_suppression_peer_and_get_flags_shape() {
    let clock = std::sync::Arc::new(ManualHashRouterClock::new(Duration::ZERO));
    let router = HashRouter::with_clock(HashRouterSetup::default(), clock);
    let key = Uint256::from_u64(7);

    let (created_first, first_flags) = router.add_suppression_peer_and_get_flags(key, 31);
    assert!(created_first);
    assert_eq!(first_flags, HashRouterFlags::UNDEFINED);

    assert!(router.set_flags(key, HashRouterFlags::BAD | HashRouterFlags::PRIVATE1));

    let (created_second, second_flags) = router.add_suppression_peer_and_get_flags(key, 32);
    assert!(!created_second);
    assert_eq!(
        second_flags,
        HashRouterFlags::BAD | HashRouterFlags::PRIVATE1
    );
}
