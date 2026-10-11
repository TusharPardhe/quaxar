//! SHARDS miss-ratio-curve estimation (Waldspurger, Park, Garthwaite and
//! Ahmad, *Efficient MRC Construction with SHARDS*, USENIX FAST 2015).
//!
//! Each access key is hashed; only keys whose hash falls below a threshold
//! (spatial sampling at rate `R`) are tracked. For a sampled key the LRU stack
//! (reuse) distance in the sampled stream, scaled by `1/R`, estimates its
//! distance in the full stream. The histogram of scaled distances gives, for
//! every cache size `c`, the miss ratio of an LRU cache of size `c`:
//! `miss(c) = (cold + #{d >= c}) / total`. Memory is bounded by the number of
//! sampled distinct keys, independent of the workload size.

use std::collections::{BTreeSet, HashMap};

/// Cache sizes (in entries) at which the curve is reported.
pub const MRC_SIZES: [u64; 12] = [
    4_096, 8_192, 16_384, 32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576, 2_097_152,
    4_194_304, 8_388_608,
];

const MODULUS: u64 = 1 << 24;

#[derive(Debug)]
pub struct Shards {
    threshold: u64,
    max_tracked: usize,
    clock: u64,
    last: HashMap<u64, u64>,
    order: BTreeSet<(u64, u64)>,
    /// `bins[i]` counts scaled distances in `[MRC_SIZES[i-1], MRC_SIZES[i])`;
    /// the last bin holds distances `>= MRC_SIZES[last]`.
    bins: [u64; MRC_SIZES.len() + 1],
    cold: u64,
    total: u64,
}

impl Shards {
    /// `rate` is the sampling rate (e.g. `0.001`); `max_tracked` bounds the
    /// sampled distinct keys kept (oldest evicted, counted as cold later).
    pub fn new(rate: f64, max_tracked: usize) -> Self {
        let threshold = ((rate.clamp(1.0 / MODULUS as f64, 1.0)) * MODULUS as f64) as u64;
        Self {
            threshold: threshold.max(1),
            max_tracked: max_tracked.max(1),
            clock: 0,
            last: HashMap::new(),
            order: BTreeSet::new(),
            bins: [0; MRC_SIZES.len() + 1],
            cold: 0,
            total: 0,
        }
    }

    /// Whether `key_hash` falls in the sample (cheap pre-check, no state).
    pub fn would_sample(&self, key_hash: u64) -> bool {
        key_hash % MODULUS < self.threshold
    }

    pub fn rate(&self) -> f64 {
        self.threshold as f64 / MODULUS as f64
    }

    /// Record one access to a key with a uniformly distributed 64-bit hash.
    pub fn record(&mut self, key_hash: u64) {
        if key_hash % MODULUS >= self.threshold {
            return;
        }
        self.clock += 1;
        self.total += 1;
        let now = self.clock;
        match self.last.insert(key_hash, now) {
            Some(previous) => {
                let distinct_since = self.order.range((previous + 1, 0)..).count() as u64;
                self.order.remove(&(previous, key_hash));
                let scaled = (distinct_since as f64 / self.rate()) as u64;
                let bin = MRC_SIZES
                    .iter()
                    .position(|size| scaled < *size)
                    .unwrap_or(MRC_SIZES.len());
                self.bins[bin] += 1;
            }
            None => self.cold += 1,
        }
        self.order.insert((now, key_hash));
        while self.order.len() > self.max_tracked {
            if let Some(oldest) = self.order.pop_first() {
                self.last.remove(&oldest.1);
            }
        }
    }

    /// Sampled references recorded.
    pub fn samples(&self) -> u64 {
        self.total
    }

    /// `(cache size, miss ratio)` for each of [`MRC_SIZES`].
    pub fn curve(&self) -> Vec<(u64, f64)> {
        if self.total == 0 {
            return MRC_SIZES.iter().map(|size| (*size, 1.0)).collect();
        }
        MRC_SIZES
            .iter()
            .enumerate()
            .map(|(index, size)| {
                // Hits at size `size` are distances in bins 0..=index.
                let hits: u64 = self.bins[..=index].iter().sum();
                (*size, (self.total - hits) as f64 / self.total as f64)
            })
            .collect()
    }

    /// Diminishing-returns size of the miss-ratio curve (see
    /// [`knee_of_curve`]).
    pub fn knee(&self, tolerance: f64) -> Option<u64> {
        knee_of_curve(&self.curve(), tolerance)
    }

    /// Halve all counts so the curve tracks recent behaviour.
    pub fn decay(&mut self) {
        for bin in &mut self.bins {
            *bin /= 2;
        }
        self.cold /= 2;
        self.total = self.cold + self.bins.iter().sum::<u64>();
    }
}

/// A process-wide sampler: lock-free rejection of unsampled keys, a mutex
/// only for the sampled ~1/1024.
#[derive(Debug)]
pub struct SharedShards {
    rate: f64,
    max_tracked: usize,
    enabled: std::sync::atomic::AtomicBool,
    inner: std::sync::OnceLock<std::sync::Mutex<Shards>>,
}

impl SharedShards {
    pub const fn new(rate: f64, max_tracked: usize) -> Self {
        Self {
            rate,
            max_tracked,
            enabled: std::sync::atomic::AtomicBool::new(false),
            inner: std::sync::OnceLock::new(),
        }
    }

    /// Sampling is off until enabled, so a transient workload (initial sync)
    /// neither costs anything nor distorts the steady-state curve.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    fn get(&self) -> &std::sync::Mutex<Shards> {
        self.inner
            .get_or_init(|| std::sync::Mutex::new(Shards::new(self.rate, self.max_tracked)))
    }

    pub fn record(&self, key_hash: u64) {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let threshold = ((self.rate * MODULUS as f64) as u64).max(1);
        if key_hash % MODULUS >= threshold {
            return;
        }
        if let Ok(mut shards) = self.get().lock() {
            shards.record(key_hash);
        }
    }

    /// `(curve, sampled references, knee within tolerance)`.
    pub fn snapshot(&self, tolerance: f64) -> (Vec<(u64, f64)>, u64, Option<u64>) {
        self.get()
            .lock()
            .map(|shards| (shards.curve(), shards.samples(), shards.knee(tolerance)))
            .unwrap_or_default()
    }

    pub fn decay(&self) {
        if let Ok(mut shards) = self.get().lock() {
            shards.decay();
        }
    }
}

/// Diminishing-returns size of a miss-ratio curve: the largest cache size
/// reached by a doubling that still cuts the miss ratio by at least
/// `tolerance`.
///
/// A miss-ratio curve is convex and non-increasing. The sizes in [`MRC_SIZES`]
/// double at each step, so the natural unit is miss-ratio reduction *per
/// doubling*. The target is the top of the last doubling that still earns at
/// least `tolerance` of hit ratio; every doubling beyond it (a shallow tail)
/// is not worth the memory, and a flat head before the steep region is skipped
/// rather than mistaken for the stopping point. This expresses the memory
/// objective directly (do not hold a larger cache for a gain below `tolerance`
/// per doubling) and is well defined on a nearly straight convex curve, unlike
/// a Kneedle elbow.
///
/// - No doubling gains `tolerance` (no useful reuse): the smallest size.
/// - Every doubling gains at least `tolerance` (reuse keeps paying): the
///   largest size.
pub fn knee_of_curve(curve: &[(u64, f64)], tolerance: f64) -> Option<u64> {
    if curve.is_empty() {
        return None;
    }
    // Index of the top of the last doubling whose gain >= tolerance.
    let mut target_index = 0usize;
    for index in 1..curve.len() {
        let gain = curve[index - 1].1 - curve[index].1;
        if gain >= tolerance {
            target_index = index;
        }
    }
    Some(curve[target_index].0)
}

/// Render a curve as `size_k:miss` pairs for logs and RPC.
pub fn format_curve(curve: &[(u64, f64)]) -> String {
    curve
        .iter()
        .map(|(size, miss)| format!("{}k:{:.3}", size / 1024, miss))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix(x: u64) -> u64 {
        // splitmix64: uniform key hashes for synthetic streams.
        let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[test]
    fn cyclic_working_set_has_knee_at_its_size() {
        // A loop over 100k keys: an LRU cache smaller than 100k always
        // misses, one at least 100k always hits (after the first pass).
        let mut shards = Shards::new(0.01, 100_000);
        for _ in 0..20 {
            for key in 0..100_000u64 {
                shards.record(mix(key));
            }
        }
        let curve = shards.curve();
        let at = |size: u64| curve.iter().find(|(s, _)| *s == size).unwrap().1;
        assert!(at(65_536) > 0.9, "below the working set: {}", at(65_536));
        assert!(at(131_072) < 0.1, "above the working set: {}", at(131_072));
        assert_eq!(shards.knee(0.01), Some(131_072));
    }

    #[test]
    fn skewed_hot_set_is_cheap_to_cache() {
        // 90% of accesses to 2k hot keys, 10% to a 1M-key cold tail.
        let mut shards = Shards::new(0.01, 100_000);
        for i in 0..2_000_000u64 {
            let key = if i % 10 == 0 { 10_000_000 + i } else { i % 2_000 };
            shards.record(mix(key));
        }
        let curve = shards.curve();
        assert!(curve[0].1 < 0.15, "4k entries already catch the hot set: {}", curve[0].1);
        assert_eq!(shards.knee(0.01), Some(4_096));
    }

    #[test]
    fn convex_sloping_nodeobject_curve_stops_at_first_weak_doubling() {
        // Measured node-object curve after 8 h on testnet.
        let curve = vec![
            (4_096, 1.000),
            (8_192, 1.000),
            (16_384, 0.999),
            (32_768, 0.999),
            (65_536, 0.995),
            (131_072, 0.963), // 65k->131k 0.032
            (262_144, 0.934), // 0.029
            (524_288, 0.875), // 0.059
            (1_048_576, 0.804), // 0.071  <- last >= 0.05
            (2_097_152, 0.778), // 0.026
            (4_194_304, 0.757), // 0.021
            (8_388_608, 0.757), // 0.0
        ];
        // Last doubling earning >= 0.05 is 524k->1M, so 1M. Far below the 4M
        // the old within-tolerance rule chose.
        assert_eq!(knee_of_curve(&curve, 0.05), Some(1_048_576));
        // A looser budget keeps paying further out.
        assert_eq!(knee_of_curve(&curve, 0.02), Some(4_194_304));
    }

    #[test]
    fn convex_sloping_treenode_curve_last_strong_doubling() {
        // Measured tree-node curve after 8 h on testnet.
        let curve = vec![
            (4_096, 0.527),
            (8_192, 0.523),
            (16_384, 0.518),
            (32_768, 0.515),
            (65_536, 0.511),
            (131_072, 0.492), // 0.019
            (262_144, 0.475), // 0.017
            (524_288, 0.447), // 0.028
            (1_048_576, 0.413), // 0.034
            (2_097_152, 0.386), // 0.027
            (4_194_304, 0.345), // 0.041  <- last >= 0.03
            (8_388_608, 0.345), // 0.0
        ];
        // This curve keeps paying ~0.03 per doubling into the millions, so at a
        // 0.03 budget the last strong doubling reaches 4M. A tighter budget of
        // 0.05 finds no qualifying doubling and keeps the cache minimal.
        assert_eq!(knee_of_curve(&curve, 0.03), Some(4_194_304));
        assert_eq!(knee_of_curve(&curve, 0.05), Some(4_096));
    }

    #[test]
    fn flat_curve_with_no_reuse_picks_the_minimum() {
        let curve = vec![
            (4_096, 0.998),
            (8_192, 0.997),
            (16_384, 0.997),
            (32_768, 0.996),
            (65_536, 0.996),
            (131_072, 0.995),
            (262_144, 0.995),
            (524_288, 0.994),
            (1_048_576, 0.994),
            (2_097_152, 0.993),
            (4_194_304, 0.993),
            (8_388_608, 0.993),
        ];
        assert_eq!(knee_of_curve(&curve, 0.02), Some(4_096));
    }

    #[test]
    fn sharp_step_curve_picks_the_working_set_size() {
        // A clean step at 256k: the only doubling above tolerance is
        // 131k->256k, so the target is 256k even though it has a flat head.
        let curve = vec![
            (4_096, 1.0),
            (8_192, 1.0),
            (16_384, 1.0),
            (32_768, 1.0),
            (65_536, 1.0),
            (131_072, 1.0),
            (262_144, 0.02),
            (524_288, 0.01),
            (1_048_576, 0.01),
            (2_097_152, 0.01),
            (4_194_304, 0.01),
            (8_388_608, 0.01),
        ];
        assert_eq!(knee_of_curve(&curve, 0.02), Some(262_144));
    }

    #[test]
    fn curve_that_always_pays_returns_the_maximum() {
        let curve = vec![
            (4_096, 0.80),
            (8_192, 0.60),
            (16_384, 0.45),
            (32_768, 0.33),
            (65_536, 0.24),
            (131_072, 0.17),
            (262_144, 0.12),
            (524_288, 0.08),
            (1_048_576, 0.05),
            (2_097_152, 0.03),
            (4_194_304, 0.015),
            (8_388_608, 0.0),
        ];
        assert_eq!(knee_of_curve(&curve, 0.01), Some(8_388_608));
    }
}
