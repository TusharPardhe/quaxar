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

    /// Smallest size whose miss ratio is within `tolerance` of the curve's
    /// best (largest-size) miss ratio: the knee beyond which more memory
    /// buys less than `tolerance` hit ratio.
    pub fn knee(&self, tolerance: f64) -> Option<u64> {
        let curve = self.curve();
        let best = curve.last()?.1;
        curve
            .iter()
            .find(|(_, miss)| *miss <= best + tolerance)
            .map(|(size, _)| *size)
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
}
