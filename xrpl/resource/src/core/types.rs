use basics::chrono::{ManualStopwatch, SystemStopwatch};
use protocol::encode_node_public_base58 as protocol_encode_node_public_base58;
use std::collections::VecDeque;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

pub const WARNING_THRESHOLD: i64 = 5_000;
pub const DROP_THRESHOLD: i64 = 25_000;
pub const DECAY_WINDOW_SECONDS: i64 = 32;
pub const MINIMUM_GOSSIP_BALANCE: i64 = 1_000;
pub const SECONDS_UNTIL_EXPIRATION: Duration = Duration::from_secs(300);
pub const GOSSIP_EXPIRATION_SECONDS: Duration = Duration::from_secs(30);
pub const FEE_LOG_AS_WARN: i32 = 3_000;
pub const FEE_LOG_AS_INFO: i32 = 1_000;
pub const FEE_LOG_AS_DEBUG: i32 = 100;

pub static FEE_MALFORMED_REQUEST: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(200, "malformed request"));
pub static FEE_REQUEST_NO_REPLY: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(10, "unsatisfiable request"));
pub static FEE_INVALID_SIGNATURE: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(2_000, "invalid signature"));
pub static FEE_USELESS_DATA: LazyLock<Charge> = LazyLock::new(|| Charge::new(150, "useless data"));
pub static FEE_INVALID_DATA: LazyLock<Charge> = LazyLock::new(|| Charge::new(400, "invalid data"));
pub static FEE_MALFORMED_RPC: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(100, "malformed RPC"));
pub static FEE_REFERENCE_RPC: LazyLock<Charge> = LazyLock::new(|| Charge::new(20, "reference RPC"));
pub static FEE_EXCEPTION_RPC: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(100, "exceptioned RPC"));
pub static FEE_MEDIUM_BURDEN_RPC: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(400, "medium RPC"));
pub static FEE_HEAVY_BURDEN_RPC: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(3_000, "heavy RPC"));
pub static FEE_TRIVIAL_PEER: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(1, "trivial peer request"));
pub static FEE_MODERATE_BURDEN_PEER: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(250, "moderate peer request"));
pub static FEE_HEAVY_BURDEN_PEER: LazyLock<Charge> =
    LazyLock::new(|| Charge::new(2_000, "heavy peer request"));
pub static FEE_WARNING: LazyLock<Charge> = LazyLock::new(|| Charge::new(4_000, "received warning"));
pub static FEE_DROP: LazyLock<Charge> = LazyLock::new(|| Charge::new(6_000, "dropped"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Ok,
    Warn,
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Inbound,
    Outbound,
    Unlimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Fatal,
}

pub type PublicKey = [u8; 33];

#[derive(Debug, Clone)]
pub struct Charge {
    cost: i32,
    label: String,
}

impl Charge {
    pub fn new(cost: i32, label: impl Into<String>) -> Self {
        Self {
            cost,
            label: label.into(),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn cost(&self) -> i32 {
        self.cost
    }
}

impl PartialEq for Charge {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

impl Eq for Charge {}

impl PartialOrd for Charge {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Charge {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cost.cmp(&other.cost)
    }
}

impl std::ops::Mul<i32> for Charge {
    type Output = Self;

    fn mul(self, rhs: i32) -> Self::Output {
        Self::new(self.cost * rhs, self.label)
    }
}

impl fmt::Display for Charge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (${})", self.label, self.cost)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Gossip {
    pub items: Vec<GossipItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GossipItem {
    pub balance: i64,
    pub address: SocketAddr,
}

impl GossipItem {
    pub fn new(balance: i64, address: SocketAddr) -> Self {
        Self { balance, address }
    }
}

pub trait ResourceMeter: Send + Sync + 'static {
    fn increment(&self);
}

pub trait ResourceCollector: Send + Sync + 'static {
    fn make_meter(&self, name: &str) -> Arc<dyn ResourceMeter>;
}

pub trait ResourceJournal: Send + Sync + 'static {
    fn log(&self, level: JournalLevel, message: &str);
}

pub trait ResourceClock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

impl ResourceClock for SystemStopwatch {
    fn now(&self) -> Instant {
        self.now()
    }
}

impl ResourceClock for ManualStopwatch {
    fn now(&self) -> Instant {
        self.now()
    }
}

#[derive(Debug, Default)]
pub struct NullMeter;

impl ResourceMeter for NullMeter {
    fn increment(&self) {}
}

#[derive(Debug, Default)]
pub struct NullCollector;

impl ResourceCollector for NullCollector {
    fn make_meter(&self, _name: &str) -> Arc<dyn ResourceMeter> {
        Arc::new(NullMeter)
    }
}

#[derive(Debug, Default)]
pub struct NullJournal;

impl ResourceJournal for NullJournal {
    fn log(&self, _level: JournalLevel, _message: &str) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub kind: Kind,
    pub address: SocketAddr,
}

impl Key {
    pub fn new(kind: Kind, address: SocketAddr) -> Self {
        Self { kind, address }
    }
}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
    }
}

#[derive(Debug)]
pub(crate) struct DecayingSample<const WINDOW: i64> {
    value: i64,
    when: Instant,
}

impl<const WINDOW: i64> DecayingSample<WINDOW> {
    pub fn new(now: Instant) -> Self {
        Self {
            value: 0,
            when: now,
        }
    }

    fn decay(&mut self, now: Instant) {
        if now == self.when {
            return;
        }

        if self.value != 0 {
            let mut elapsed = now.duration_since(self.when).as_secs() as i64;
            if elapsed > 4 * WINDOW {
                self.value = 0;
            } else {
                while elapsed > 0 {
                    self.value -= (self.value + WINDOW - 1) / WINDOW;
                    elapsed -= 1;
                }
            }
        }

        self.when = now;
    }

    pub fn add(&mut self, value: i64, now: Instant) -> i64 {
        self.decay(now);
        self.value += value;
        self.value / WINDOW
    }

    pub fn value(&mut self, now: Instant) -> i64 {
        self.decay(now);
        self.value / WINDOW
    }
}

#[derive(Debug)]
pub(crate) struct Entry {
    pub key: Key,
    pub public_key: Option<PublicKey>,
    pub refcount: usize,
    pub local_balance: DecayingSample<DECAY_WINDOW_SECONDS>,
    pub remote_balance: i64,
    pub last_warning_time: Option<Instant>,
    pub when_expires: Option<Instant>,
}

impl Entry {
    pub fn new(key: Key, now: Instant) -> Self {
        Self {
            key,
            public_key: None,
            refcount: 0,
            local_balance: DecayingSample::new(now),
            remote_balance: 0,
            last_warning_time: None,
            when_expires: None,
        }
    }

    pub fn is_unlimited(&self) -> bool {
        self.key.kind == Kind::Unlimited
    }

    pub fn balance(&mut self, now: Instant) -> i64 {
        self.local_balance.value(now) + self.remote_balance
    }

    pub fn add(&mut self, charge: i64, now: Instant) -> i64 {
        self.local_balance.add(charge, now) + self.remote_balance
    }

    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        get_fingerprint(self.key.address, self.public_key)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ImportItem {
    pub balance: i64,
    pub key: Key,
}

#[derive(Debug, Clone)]
pub(crate) struct Import {
    pub when_expires: Instant,
    pub items: Vec<ImportItem>,
}

impl Import {
    pub fn new(when_expires: Instant) -> Self {
        Self {
            when_expires,
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ResourceState {
    pub entries: std::collections::HashMap<Key, Entry>,
    pub inbound: VecDeque<Key>,
    pub outbound: VecDeque<Key>,
    pub admin: VecDeque<Key>,
    pub inactive: VecDeque<Key>,
    pub imports: std::collections::HashMap<String, Import>,
}

impl ResourceState {
    pub fn new() -> Self {
        Self::default()
    }
}

pub(crate) fn get_fingerprint(address: SocketAddr, public_key: Option<PublicKey>) -> String {
    let mut fingerprint = format!("IP Address: {address}");
    if let Some(public_key) = public_key {
        fingerprint.push_str(", Public Key: ");
        fingerprint.push_str(&encode_node_public_base58(public_key));
    }
    fingerprint
}

pub(crate) fn encode_node_public_base58(public_key: PublicKey) -> String {
    protocol_encode_node_public_base58(public_key)
}

pub(crate) fn normalize_inbound_address(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(address.ip(), 0)
}

pub(crate) fn normalize_unlimited_address(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(address.ip(), 1)
}

#[cfg(test)]
mod tests {
    use super::{encode_node_public_base58, get_fingerprint};
    use std::net::SocketAddr;

    const CPP_NODE_PUBLIC_KEY: [u8; 33] = [
        0x03, 0xD4, 0x9C, 0x56, 0xE1, 0xB1, 0x85, 0xF1, 0xBE, 0x89, 0x9A, 0xE6, 0x6A, 0x02, 0xEF,
        0xC1, 0x7F, 0x78, 0xEA, 0x6F, 0xC5, 0x3A, 0xF8, 0x5E, 0x0F, 0xE5, 0x4C, 0x6E, 0x8B, 0x7F,
        0x8C, 0x71, 0xA8,
    ];

    #[test]
    fn node_public_base58_genesis_vector() {
        assert_eq!(
            encode_node_public_base58(CPP_NODE_PUBLIC_KEY),
            "n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
        );
    }

    #[test]
    fn fingerprint_uses_cpp_node_public_rendering() {
        let address: SocketAddr = "127.0.0.1:0".parse().expect("address should parse");

        assert_eq!(
            get_fingerprint(address, Some(CPP_NODE_PUBLIC_KEY)),
            "IP Address: 127.0.0.1:0, Public Key: n94a1u4jAz288pZLtw6yFWVbi89YamiC6JBXPVUj5zmExe5fTVg9"
        );
    }
}
