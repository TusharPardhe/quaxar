//! Reduce-relay slot ownership aligned with `overlay/Slot.h`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use basics::base_uint::Uint256;
use protocol::PublicKey;

use crate::message::ProtocolMessageType;

pub const MIN_UNSQUELCH_EXPIRE: Duration = Duration::from_secs(300);
pub const MAX_UNSQUELCH_EXPIRE_DEFAULT: Duration = Duration::from_secs(600);
pub const SQUELCH_PER_PEER: Duration = Duration::from_secs(10);
pub const MAX_UNSQUELCH_EXPIRE_PEERS: Duration = Duration::from_secs(3600);
pub const IDLED: Duration = Duration::from_secs(8);
pub const MIN_MESSAGE_THRESHOLD: u16 = 19;
pub const MAX_MESSAGE_THRESHOLD: u16 = 20;
pub const MAX_SELECTED_PEERS: u16 = 5;
pub const WAIT_ON_BOOTUP: Duration = Duration::from_secs(600);
pub const MAX_TX_QUEUE_SIZE: usize = 10_000;

pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> Duration;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct ManualClock {
    now: Arc<std::sync::Mutex<Duration>>,
}

impl ManualClock {
    pub fn new(now: Duration) -> Self {
        Self {
            now: Arc::new(std::sync::Mutex::new(now)),
        }
    }

    pub fn advance(&self, delta: Duration) {
        let mut now = self.now.lock().expect("manual clock lock");
        *now += delta;
    }

    pub fn set(&self, now: Duration) {
        *self.now.lock().expect("manual clock lock") = now;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.lock().expect("manual clock lock")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeerState {
    Counting,
    Selected,
    Squelched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotState {
    Counting,
    Selected,
}

pub trait SquelchHandler: Send + Sync {
    fn squelch(&self, validator: PublicKey, id: u32, duration: u32);
    fn unsquelch(&self, validator: PublicKey, id: u32);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPeerSnapshot {
    pub state: PeerState,
    pub count: u16,
    pub squelch_expire_ms: u32,
    pub last_message_ms: u32,
}

#[derive(Debug, Clone)]
struct PeerInfo {
    state: PeerState,
    count: u16,
    expire: Duration,
    last_message: Duration,
}

pub struct Slot {
    clock: Arc<dyn Clock>,
    handler: Arc<dyn SquelchHandler>,
    max_selected_peers: u16,
    peers: HashMap<u32, PeerInfo>,
    considered: HashSet<u32>,
    reached_threshold: u16,
    last_selected: Duration,
    state: SlotState,
}

impl Slot {
    pub fn new(
        clock: Arc<dyn Clock>,
        handler: Arc<dyn SquelchHandler>,
        max_selected_peers: u16,
    ) -> Self {
        let now = clock.now();
        Self {
            clock,
            handler,
            max_selected_peers,
            peers: HashMap::new(),
            considered: HashSet::new(),
            reached_threshold: 0,
            last_selected: now,
            state: SlotState::Counting,
        }
    }

    pub fn update<F>(
        &mut self,
        validator: PublicKey,
        id: u32,
        _message_type: ProtocolMessageType,
        mut ignored_squelch: F,
    ) where
        F: FnMut(),
    {
        let now = self.clock.now();
        let Some(peer) = self.peers.get_mut(&id) else {
            self.peers.insert(
                id,
                PeerInfo {
                    state: PeerState::Counting,
                    count: 0,
                    expire: now,
                    last_message: now,
                },
            );
            self.init_counting();
            return;
        };

        if peer.state == PeerState::Squelched && now > peer.expire {
            peer.state = PeerState::Counting;
            peer.last_message = now;
            self.init_counting();
            return;
        }

        peer.last_message = now;
        if peer.state == PeerState::Squelched {
            ignored_squelch();
        }
        if self.state != SlotState::Counting || peer.state == PeerState::Squelched {
            return;
        }

        peer.count = peer.count.saturating_add(1);
        if peer.count > MIN_MESSAGE_THRESHOLD {
            self.considered.insert(id);
        }
        if peer.count == MAX_MESSAGE_THRESHOLD + 1 {
            self.reached_threshold = self.reached_threshold.saturating_add(1);
        }

        if now.saturating_sub(self.last_selected) > MAX_UNSQUELCH_EXPIRE_DEFAULT * 2 {
            self.init_counting();
            return;
        }

        if self.reached_threshold == self.max_selected_peers {
            self.select_peers(validator, now);
        }
    }

    pub fn delete_peer(&mut self, validator: PublicKey, id: u32, erase: bool) {
        let Some(existing) = self.peers.get(&id).cloned() else {
            return;
        };

        let now = self.clock.now();
        let mut to_unsquelch = Vec::new();

        if existing.state == PeerState::Selected {
            for (peer_id, peer) in &mut self.peers {
                if peer.state == PeerState::Squelched {
                    to_unsquelch.push(*peer_id);
                }
                peer.state = PeerState::Counting;
                peer.count = 0;
                peer.expire = now;
            }
            self.considered.clear();
            self.reached_threshold = 0;
            self.state = SlotState::Counting;
        } else if self.considered.remove(&id) && existing.count > MAX_MESSAGE_THRESHOLD {
            self.reached_threshold = self.reached_threshold.saturating_sub(1);
        }

        if let Some(peer) = self.peers.get_mut(&id) {
            peer.last_message = now;
            peer.count = 0;
        }
        if erase {
            self.peers.remove(&id);
        }

        for peer_id in to_unsquelch {
            self.handler.unsquelch(validator, peer_id);
        }
    }

    pub fn delete_idle_peer(&mut self, validator: PublicKey) {
        let now = self.clock.now();
        let idle = self
            .peers
            .iter()
            .filter_map(|(id, peer)| (now.saturating_sub(peer.last_message) > IDLED).then_some(*id))
            .collect::<Vec<_>>();
        for id in idle {
            self.delete_peer(validator, id, false);
        }
    }

    pub fn last_selected(&self) -> Duration {
        self.last_selected
    }

    pub fn state(&self) -> SlotState {
        self.state
    }

    pub fn in_state(&self, state: PeerState) -> u16 {
        self.peers
            .values()
            .filter(|peer| peer.state == state)
            .count() as u16
    }

    pub fn not_in_state(&self, state: PeerState) -> u16 {
        self.peers
            .values()
            .filter(|peer| peer.state != state)
            .count() as u16
    }

    pub fn selected(&self) -> BTreeSet<u32> {
        self.peers
            .iter()
            .filter_map(|(id, peer)| (peer.state == PeerState::Selected).then_some(*id))
            .collect()
    }

    pub fn peers(&self) -> BTreeMap<u32, SlotPeerSnapshot> {
        let now = self.clock.now();
        self.peers
            .iter()
            .map(|(id, peer)| {
                (
                    *id,
                    SlotPeerSnapshot {
                        state: peer.state,
                        count: peer.count,
                        squelch_expire_ms: peer
                            .expire
                            .saturating_sub(now)
                            .as_millis()
                            .min(u32::MAX as u128)
                            as u32,
                        last_message_ms: now
                            .saturating_sub(peer.last_message)
                            .as_millis()
                            .min(u32::MAX as u128) as u32,
                    },
                )
            })
            .collect()
    }

    fn select_peers(&mut self, validator: PublicKey, now: Duration) {
        let mut considered = self.considered.iter().copied().collect::<Vec<_>>();
        considered.sort_unstable();
        let selected = considered
            .into_iter()
            .filter(|id| {
                self.peers
                    .get(id)
                    .is_some_and(|peer| now.saturating_sub(peer.last_message) < IDLED)
            })
            .take(self.max_selected_peers as usize)
            .collect::<BTreeSet<_>>();

        if selected.len() != self.max_selected_peers as usize {
            self.init_counting();
            return;
        }

        self.last_selected = now;
        let squelch_duration = self.get_squelch_duration(
            self.peers
                .len()
                .saturating_sub(self.max_selected_peers as usize),
        );
        for (id, peer) in &mut self.peers {
            peer.count = 0;
            if selected.contains(id) {
                peer.state = PeerState::Selected;
                continue;
            }
            if peer.state != PeerState::Squelched {
                peer.state = PeerState::Squelched;
                peer.expire = now + squelch_duration;
                self.handler.squelch(
                    validator,
                    *id,
                    squelch_duration.as_secs().min(u32::MAX as u64) as u32,
                );
            }
        }
        self.considered.clear();
        self.reached_threshold = 0;
        self.state = SlotState::Selected;
    }

    fn get_squelch_duration(&self, npeers: usize) -> Duration {
        let per_peer = Duration::from_secs(SQUELCH_PER_PEER.as_secs() * npeers as u64);
        let mut max_duration = std::cmp::max(MAX_UNSQUELCH_EXPIRE_DEFAULT, per_peer);
        if max_duration > MAX_UNSQUELCH_EXPIRE_PEERS {
            max_duration = MAX_UNSQUELCH_EXPIRE_PEERS;
        }
        let span = max_duration
            .as_secs()
            .saturating_sub(MIN_UNSQUELCH_EXPIRE.as_secs());
        Duration::from_secs(MIN_UNSQUELCH_EXPIRE.as_secs() + (span / 2))
    }

    fn init_counting(&mut self) {
        self.reset_counts();
        self.considered.clear();
        self.reached_threshold = 0;
        self.state = SlotState::Counting;
    }

    fn reset_counts(&mut self) {
        for peer in self.peers.values_mut() {
            peer.count = 0;
        }
    }
}

#[derive(Debug, Clone)]
struct SeenMessage {
    seen_at: Duration,
    peers: HashSet<u32>,
}

pub struct Slots {
    clock: Arc<dyn Clock>,
    handler: Arc<dyn SquelchHandler>,
    base_squelch_enabled: bool,
    max_selected_peers: u16,
    wait_on_bootup: Duration,
    started_at: Duration,
    slots: HashMap<PublicKey, Slot>,
    peers_with_message: HashMap<Uint256, SeenMessage>,
}

impl Slots {
    pub fn new(
        clock: Arc<dyn Clock>,
        handler: Arc<dyn SquelchHandler>,
        base_squelch_enabled: bool,
        max_selected_peers: u16,
        wait_on_bootup: Duration,
    ) -> Self {
        let started_at = clock.now();
        Self {
            clock,
            handler,
            base_squelch_enabled,
            max_selected_peers,
            wait_on_bootup,
            started_at,
            slots: HashMap::new(),
            peers_with_message: HashMap::new(),
        }
    }

    pub fn base_squelch_ready(&self) -> bool {
        self.base_squelch_enabled && self.reduce_relay_ready()
    }

    pub fn reduce_relay_ready(&self) -> bool {
        self.clock.now().saturating_sub(self.started_at) >= self.wait_on_bootup
    }

    pub fn update_slot_and_squelch<F>(
        &mut self,
        key: Uint256,
        validator: PublicKey,
        id: u32,
        message_type: ProtocolMessageType,
        callback: F,
    ) where
        F: FnMut(),
    {
        if !self.add_peer_message(key, id) {
            return;
        }
        let slot = self.slots.entry(validator).or_insert_with(|| {
            Slot::new(
                Arc::clone(&self.clock),
                Arc::clone(&self.handler),
                self.max_selected_peers,
            )
        });
        slot.update(validator, id, message_type, callback);
    }

    pub fn update_many(
        &mut self,
        key: Uint256,
        validator: PublicKey,
        peers: impl IntoIterator<Item = u32>,
        message_type: ProtocolMessageType,
    ) {
        for peer in peers {
            self.update_slot_and_squelch(key, validator, peer, message_type, || {});
        }
    }

    pub fn delete_peer(&mut self, id: u32, erase: bool) {
        for (validator, slot) in &mut self.slots {
            slot.delete_peer(*validator, id, erase);
        }
    }

    pub fn delete_idle_peers(&mut self) {
        let now = self.clock.now();
        self.slots.retain(|validator, slot| {
            slot.delete_idle_peer(*validator);
            now.saturating_sub(slot.last_selected()) <= MAX_UNSQUELCH_EXPIRE_DEFAULT
        });
    }

    pub fn get_selected(&self, validator: PublicKey) -> BTreeSet<u32> {
        self.slots
            .get(&validator)
            .map_or_else(BTreeSet::new, Slot::selected)
    }

    pub fn get_state(&self, validator: PublicKey) -> Option<SlotState> {
        self.slots.get(&validator).map(Slot::state)
    }

    pub fn get_peers(&self, validator: PublicKey) -> BTreeMap<u32, SlotPeerSnapshot> {
        self.slots
            .get(&validator)
            .map_or_else(BTreeMap::new, Slot::peers)
    }

    fn add_peer_message(&mut self, key: Uint256, id: u32) -> bool {
        let now = self.clock.now();
        self.peers_with_message
            .retain(|_, entry| now.saturating_sub(entry.seen_at) <= IDLED);

        if key.is_zero() {
            return true;
        }

        match self.peers_with_message.get_mut(&key) {
            None => {
                self.peers_with_message.insert(
                    key,
                    SeenMessage {
                        seen_at: now,
                        peers: HashSet::from([id]),
                    },
                );
                true
            }
            Some(entry) => {
                entry.seen_at = now;
                entry.peers.insert(id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use protocol::{KeyType, PublicKey, SecretKey, derive_public_key};

    use super::{
        Clock, IDLED, MAX_MESSAGE_THRESHOLD, ManualClock, PeerState, Slot, SlotState, Slots,
        SquelchHandler,
    };
    use crate::ProtocolMessageType;
    use crate::slot::WAIT_ON_BOOTUP;
    use basics::base_uint::Uint256;

    #[derive(Debug, Default)]
    struct RecordingHandler {
        squelched: Mutex<Vec<(PublicKey, u32, u32)>>,
        unsquelched: Mutex<Vec<(PublicKey, u32)>>,
    }

    impl SquelchHandler for RecordingHandler {
        fn squelch(&self, validator: PublicKey, id: u32, duration: u32) {
            self.squelched
                .lock()
                .expect("squelched lock")
                .push((validator, id, duration));
        }

        fn unsquelch(&self, validator: PublicKey, id: u32) {
            self.unsquelched
                .lock()
                .expect("unsquelched lock")
                .push((validator, id));
        }
    }

    fn validator() -> PublicKey {
        let secret = SecretKey::from_bytes([9u8; 32]);
        derive_public_key(KeyType::Secp256k1, &secret).expect("validator key")
    }

    #[test]
    fn slot_selects_and_squelches_when_threshold_is_reached() {
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(1_000)));
        let handler = Arc::new(RecordingHandler::default());
        let validator = validator();
        let mut slot = Slot::new(clock, handler.clone(), 3);

        for id in 1..=4 {
            slot.update(validator, id, ProtocolMessageType::MtValidation, || {});
        }
        for _ in 0..=MAX_MESSAGE_THRESHOLD {
            for id in 1..=3 {
                slot.update(validator, id, ProtocolMessageType::MtValidation, || {});
            }
        }

        assert_eq!(slot.state(), SlotState::Selected);
        assert_eq!(slot.selected(), BTreeSet::from([1, 2, 3]));
        let squelched = handler.squelched.lock().expect("squelched lock");
        assert_eq!(squelched.len(), 1);
        assert_eq!(squelched[0].1, 4);
        assert_eq!(slot.in_state(PeerState::Selected), 3);
        assert_eq!(slot.in_state(PeerState::Squelched), 1);
    }

    #[test]
    fn deleting_selected_peer_unsquelches_all_squelched_peers() {
        let manual = Arc::new(ManualClock::new(Duration::from_secs(1_000)));
        let clock: Arc<dyn Clock> = manual.clone();
        let handler = Arc::new(RecordingHandler::default());
        let validator = validator();
        let mut slot = Slot::new(clock, handler.clone(), 2);

        for id in 1..=3 {
            slot.update(validator, id, ProtocolMessageType::MtValidation, || {});
        }
        for _ in 0..=MAX_MESSAGE_THRESHOLD {
            for id in 1..=2 {
                slot.update(validator, id, ProtocolMessageType::MtValidation, || {});
            }
        }
        slot.delete_peer(validator, 1, true);

        assert_eq!(slot.state(), SlotState::Counting);
        let unsquelched = handler.unsquelched.lock().expect("unsquelched lock");
        assert_eq!(unsquelched.len(), 1);
        assert_eq!(unsquelched[0].1, 3);
    }

    #[test]
    fn slots_ignore_duplicate_peer_message_keys_and_age_them_out() {
        let manual = Arc::new(ManualClock::new(Duration::from_secs(1_000)));
        let clock: Arc<dyn Clock> = manual.clone();
        let handler = Arc::new(RecordingHandler::default());
        let validator = validator();
        let mut slots = Slots::new(clock, handler, true, 2, WAIT_ON_BOOTUP);
        manual.advance(WAIT_ON_BOOTUP);

        let key = Uint256::from_u64(7);
        slots.update_slot_and_squelch(key, validator, 1, ProtocolMessageType::MtValidation, || {});
        slots.update_slot_and_squelch(key, validator, 1, ProtocolMessageType::MtValidation, || {
            panic!("duplicate message should not reach slot");
        });

        assert_eq!(slots.get_peers(validator).len(), 1);

        manual.advance(IDLED + Duration::from_secs(1));
        slots.update_slot_and_squelch(key, validator, 1, ProtocolMessageType::MtValidation, || {});
        assert_eq!(slots.get_peers(validator).len(), 1);
    }
}
