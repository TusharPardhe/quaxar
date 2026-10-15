//! Sampled miss-ratio curve of decoded-node lookups by hash (SHARDS,
//! Waldspurger et al., USENIX FAST 2015). Every `SHAMapFamily::cache_lookup`
//! is a reference to the TreeNodeCache layer; the curve tells how large that
//! cache must be for a given hit ratio, so it can be sized from measurement
//! instead of a fixed `node_size` constant.

use basics::sha_map_hash::SHAMapHash;
use basics::sync::shards::SharedShards;

/// 1 in 16 lookups sampled; at most 256k sampled keys tracked (scaled reach
/// 4M entries). Steady-state lookup volume is low, so a high rate is cheap.
static SAMPLER: SharedShards = SharedShards::new(1.0 / 16.0, 262_144);

/// Enable or disable sampling (enabled only while the node is full).
pub fn set_enabled(enabled: bool) {
    SAMPLER.set_enabled(enabled);
}

/// Record one decoded-node lookup. Takes the lock only for sampled keys.
pub fn record(hash: SHAMapHash) {
    let bytes = hash.as_uint256().data();
    SAMPLER.record(u64::from_le_bytes(bytes[..8].try_into().expect("8 bytes")));
}

/// `(cache size, miss ratio)` points, sampled references, and the knee.
pub fn snapshot(tolerance: f64) -> (Vec<(u64, f64)>, u64, Option<u64>) {
    SAMPLER.snapshot(tolerance)
}

/// Halve the history so the curve follows the current workload.
pub fn decay() {
    SAMPLER.decay();
}
