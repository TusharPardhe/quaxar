//! Sampled miss-ratio curve of decoded-node lookups by hash (SHARDS,
//! Waldspurger et al., USENIX FAST 2015). Every `SHAMapFamily::cache_lookup`
//! is a reference to the TreeNodeCache layer; the curve tells how large that
//! cache must be for a given hit ratio, so it can be sized from measurement
//! instead of a fixed `node_size` constant.

use std::sync::{Mutex, OnceLock};

use basics::sha_map_hash::SHAMapHash;
use basics::sync::shards::Shards;

/// 1 in 1024 lookups sampled; at most 64k sampled keys tracked (~6 MB).
const RATE: f64 = 1.0 / 1024.0;
const MAX_TRACKED: usize = 65_536;

fn sampler() -> &'static Mutex<Shards> {
    static SAMPLER: OnceLock<Mutex<Shards>> = OnceLock::new();
    SAMPLER.get_or_init(|| Mutex::new(Shards::new(RATE, MAX_TRACKED)))
}

fn key_hash(hash: SHAMapHash) -> u64 {
    let bytes = hash.as_uint256().data();
    u64::from_le_bytes(bytes[..8].try_into().expect("8 bytes"))
}

/// Record one decoded-node lookup. Takes the lock only for sampled keys.
pub fn record(hash: SHAMapHash) {
    let key = key_hash(hash);
    let threshold = (RATE * (1u64 << 24) as f64) as u64;
    if key % (1u64 << 24) >= threshold {
        return;
    }
    if let Ok(mut shards) = sampler().lock() {
        shards.record(key);
    }
}

/// `(cache size, miss ratio)` points, sampled references, and the knee.
pub fn snapshot(tolerance: f64) -> (Vec<(u64, f64)>, u64, Option<u64>) {
    sampler()
        .lock()
        .map(|shards| (shards.curve(), shards.samples(), shards.knee(tolerance)))
        .unwrap_or_default()
}

/// Halve the history so the curve follows the current workload.
pub fn decay() {
    if let Ok(mut shards) = sampler().lock() {
        shards.decay();
    }
}
