//! Deterministic `xrpl/core/HashRouter.h` entry behavior.
//!
//! This ports the flag constants, bitwise behavior, `Entry` flag/peer state,
//! setup defaults/bounds validation, and the deterministic `setFlags(...)`,
//! `shouldRelay(...)`, and `shouldProcess(...)` entry-local rules.

use std::{collections::BTreeSet, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HashRouterFlags(u16);

impl HashRouterFlags {
    pub const UNDEFINED: Self = Self(0x00);
    pub const BAD: Self = Self(0x02);
    pub const SAVED: Self = Self(0x04);
    pub const HELD: Self = Self(0x08);
    pub const TRUSTED: Self = Self(0x10);

    pub const PRIVATE1: Self = Self(0x0100);
    pub const PRIVATE2: Self = Self(0x0200);
    pub const PRIVATE3: Self = Self(0x0400);
    pub const PRIVATE4: Self = Self(0x0800);
    pub const PRIVATE5: Self = Self(0x1000);
    pub const PRIVATE6: Self = Self(0x2000);

    pub const fn bits(self) -> u16 {
        self.0
    }
}

pub type PeerShortId = u32;

pub const HASH_ROUTER_DEFAULT_HOLD_TIME: Duration = Duration::from_secs(300);
pub const HASH_ROUTER_DEFAULT_RELAY_TIME: Duration = Duration::from_secs(30);
pub const HASH_ROUTER_MIN_HOLD_TIME: Duration = Duration::from_secs(12);
pub const HASH_ROUTER_MIN_RELAY_TIME: Duration = Duration::from_secs(8);

pub fn validate_hash_router_setup(
    hold_time: Duration,
    relay_time: Duration,
) -> Result<(), &'static str> {
    if hold_time < HASH_ROUTER_MIN_HOLD_TIME {
        return Err(
            "HashRouter hold time must be at least 12 seconds (the approximate validation time for three ledgers).",
        );
    }

    if relay_time < HASH_ROUTER_MIN_RELAY_TIME {
        return Err(
            "HashRouter relay time must be at least 8 seconds (the approximate validation time for two ledgers).",
        );
    }

    if relay_time > hold_time {
        return Err("HashRouter relay time must be less than or equal to hold time");
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HashRouterEntry {
    flags: HashRouterFlags,
    peers: BTreeSet<PeerShortId>,
    relayed: Option<Duration>,
    processed: Option<Duration>,
}

impl HashRouterEntry {
    pub fn add_peer(&mut self, peer: PeerShortId) {
        if peer != 0 {
            self.peers.insert(peer);
        }
    }

    pub const fn flags(&self) -> HashRouterFlags {
        self.flags
    }

    pub fn set_flags(&mut self, flags_to_set: HashRouterFlags) -> bool {
        let (merged, changed) = merge_set_flags(self.flags, flags_to_set);
        self.flags = merged;
        changed
    }

    pub fn release_peer_set(&mut self) -> BTreeSet<PeerShortId> {
        std::mem::take(&mut self.peers)
    }

    pub const fn relayed(&self) -> Option<Duration> {
        self.relayed
    }

    pub fn add_peer_with_status(&mut self, peer: PeerShortId) -> (bool, Option<Duration>) {
        let created = !self.peers.contains(&peer);
        self.add_peer(peer);
        (created, self.relayed)
    }

    pub fn should_relay(&mut self, now: Duration, relay_time: Duration) -> bool {
        if self
            .relayed
            .is_some_and(|relayed| relayed + relay_time > now)
        {
            return false;
        }

        self.relayed = Some(now);
        true
    }

    pub fn should_process(&mut self, now: Duration, interval: Duration) -> bool {
        if self
            .processed
            .is_some_and(|processed| processed + interval > now)
        {
            return false;
        }

        self.processed = Some(now);
        true
    }
}

impl std::ops::BitOr for HashRouterFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for HashRouterFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

impl std::ops::BitAnd for HashRouterFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::BitAndAssign for HashRouterFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = *self & rhs;
    }
}

pub const fn any(flags: HashRouterFlags) -> bool {
    flags.bits() != 0
}

/// Mirrors the deterministic merge rule of `HashRouter::setFlags(...)`.
pub fn contains_all(current: HashRouterFlags, required: HashRouterFlags) -> bool {
    (current & required) == required
}

/// Mirrors the deterministic merge rule of `HashRouter::setFlags(...)`.
///
/// Returns the merged flags plus whether the input changed.
pub fn merge_set_flags(
    current: HashRouterFlags,
    flags_to_set: HashRouterFlags,
) -> (HashRouterFlags, bool) {
    assert!(
        any(flags_to_set),
        "HashRouter::setFlags requires non-zero input"
    );

    let changed = !contains_all(current, flags_to_set);
    (current | flags_to_set, changed)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, time::Duration};

    use super::{
        HASH_ROUTER_DEFAULT_HOLD_TIME, HASH_ROUTER_DEFAULT_RELAY_TIME, HashRouterEntry,
        HashRouterFlags, any, contains_all, merge_set_flags, validate_hash_router_setup,
    };

    #[test]
    fn flag_bit_values_match_current_cpp_constants() {
        assert_eq!(HashRouterFlags::UNDEFINED.bits(), 0x00);
        assert_eq!(HashRouterFlags::BAD.bits(), 0x02);
        assert_eq!(HashRouterFlags::SAVED.bits(), 0x04);
        assert_eq!(HashRouterFlags::HELD.bits(), 0x08);
        assert_eq!(HashRouterFlags::TRUSTED.bits(), 0x10);
        assert_eq!(HashRouterFlags::PRIVATE1.bits(), 0x0100);
        assert_eq!(HashRouterFlags::PRIVATE2.bits(), 0x0200);
        assert_eq!(HashRouterFlags::PRIVATE3.bits(), 0x0400);
        assert_eq!(HashRouterFlags::PRIVATE4.bits(), 0x0800);
        assert_eq!(HashRouterFlags::PRIVATE5.bits(), 0x1000);
        assert_eq!(HashRouterFlags::PRIVATE6.bits(), 0x2000);
    }

    #[test]
    fn bitwise_helpers_match_cpp_roles() {
        let combined = HashRouterFlags::BAD | HashRouterFlags::HELD;
        assert_eq!(combined.bits(), 0x0A);
        assert!(any(combined & HashRouterFlags::BAD));
        assert!(!any(combined & HashRouterFlags::TRUSTED));
    }

    #[test]
    fn merge_set_flags_matches_current_hash_router_rule() {
        let (merged, changed) = merge_set_flags(HashRouterFlags::BAD, HashRouterFlags::PRIVATE1);
        assert_eq!(merged, HashRouterFlags::BAD | HashRouterFlags::PRIVATE1);
        assert!(changed);

        let (merged_again, changed_again) = merge_set_flags(merged, HashRouterFlags::PRIVATE1);
        assert_eq!(merged_again, merged);
        assert!(!changed_again);
    }

    #[test]
    fn contains_all_mask_subset_rule() {
        let flags = HashRouterFlags::BAD | HashRouterFlags::PRIVATE1 | HashRouterFlags::HELD;

        assert!(contains_all(flags, HashRouterFlags::BAD));
        assert!(contains_all(
            flags,
            HashRouterFlags::BAD | HashRouterFlags::HELD
        ));
        assert!(!contains_all(flags, HashRouterFlags::TRUSTED));
        assert!(!contains_all(
            flags,
            HashRouterFlags::BAD | HashRouterFlags::TRUSTED
        ));
    }

    #[test]
    #[should_panic(expected = "HashRouter::setFlags requires non-zero input")]
    fn merge_set_flags_rejects_zero_flags_assert() {
        let _ = merge_set_flags(HashRouterFlags::BAD, HashRouterFlags::UNDEFINED);
    }

    #[test]
    fn entry_add_peer_ignores_zero_and_deduplicates() {
        let mut entry = HashRouterEntry::default();

        entry.add_peer(0);
        entry.add_peer(7);
        entry.add_peer(7);
        entry.add_peer(9);

        assert_eq!(entry.release_peer_set(), BTreeSet::from([7, 9]));
        assert!(entry.release_peer_set().is_empty());
    }

    #[test]
    fn entry_set_flags_reuses_merge_rule() {
        let mut entry = HashRouterEntry::default();

        assert!(entry.set_flags(HashRouterFlags::HELD));
        assert_eq!(entry.flags(), HashRouterFlags::HELD);
        assert!(!entry.set_flags(HashRouterFlags::HELD));
        assert_eq!(entry.flags(), HashRouterFlags::HELD);
    }

    #[test]
    fn entry_should_relay_tracks_timestamp_and_clears_peers_on_release() {
        let mut entry = HashRouterEntry::default();
        entry.add_peer(11);
        entry.add_peer(13);

        assert!(entry.should_relay(Duration::from_secs(10), Duration::from_secs(30)));
        assert_eq!(entry.relayed(), Some(Duration::from_secs(10)));
        assert!(!entry.should_relay(Duration::from_secs(39), Duration::from_secs(30)));
        assert!(entry.should_relay(Duration::from_secs(40), Duration::from_secs(30)));
        assert_eq!(entry.release_peer_set(), BTreeSet::from([11, 13]),);
        assert!(entry.release_peer_set().is_empty());
    }

    #[test]
    fn entry_should_process_tracks_last_processed_interval() {
        let mut entry = HashRouterEntry::default();

        assert!(entry.should_process(Duration::from_secs(5), Duration::from_secs(10)));
        assert!(!entry.should_process(Duration::from_secs(14), Duration::from_secs(10)));
        assert!(entry.should_process(Duration::from_secs(15), Duration::from_secs(10)));
    }

    #[test]
    fn add_peer_with_status_reports_existing_membership_and_relay_status() {
        let mut entry = HashRouterEntry::default();

        let (created, relayed) = entry.add_peer_with_status(21);
        assert!(created);
        assert_eq!(relayed, None);

        assert!(entry.should_relay(Duration::from_secs(8), Duration::from_secs(30)));

        let (created_again, relayed_again) = entry.add_peer_with_status(21);
        assert!(!created_again);
        assert_eq!(relayed_again, Some(Duration::from_secs(8)));
    }

    #[test]
    fn hash_router_setup_defaults_and_bounds_match_cpp_setup_helper() {
        assert_eq!(HASH_ROUTER_DEFAULT_HOLD_TIME, Duration::from_secs(300));
        assert_eq!(HASH_ROUTER_DEFAULT_RELAY_TIME, Duration::from_secs(30));

        assert_eq!(
            validate_hash_router_setup(Duration::from_secs(300), Duration::from_secs(30)),
            Ok(())
        );
        assert_eq!(
            validate_hash_router_setup(Duration::from_secs(600), Duration::from_secs(15)),
            Ok(())
        );
        assert_eq!(
            validate_hash_router_setup(Duration::from_secs(400), Duration::from_secs(400)),
            Ok(())
        );
        assert_eq!(
            validate_hash_router_setup(Duration::from_secs(60), Duration::from_secs(120)),
            Err("HashRouter relay time must be less than or equal to hold time")
        );
        assert_eq!(
            validate_hash_router_setup(Duration::from_secs(10), Duration::from_secs(120)),
            Err(
                "HashRouter hold time must be at least 12 seconds (the approximate validation time for three ledgers)."
            )
        );
        assert_eq!(
            validate_hash_router_setup(Duration::from_secs(500), Duration::from_secs(6)),
            Err(
                "HashRouter relay time must be at least 8 seconds (the approximate validation time for two ledgers)."
            )
        );
    }
}
