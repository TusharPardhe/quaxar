//! Collectors, events, random utilities, and transaction submitters.
//!
//! `test/csf/events.h`, `test/csf/Histogram.h`, `test/csf/random.h`,
//! `test/csf/submitters.h`

use super::types::*;
use std::collections::HashMap;
use std::time::Duration;

// ─── Events ──────────────────────────────────────────────────────────────────

/// Events emitted by peers during simulation.
#[derive(Debug, Clone)]
pub enum SimEvent {
    SubmitTx {
        tx: Tx,
    },
    StartRound {
        best_ledger: LedgerID,
        prev_ledger: Ledger,
    },
    CloseLedger {
        prev_ledger: Ledger,
        txs: TxSetType,
    },
    AcceptLedger {
        ledger: Ledger,
        prior: Ledger,
    },
    WrongPrevLedger {
        wrong: LedgerID,
        right: LedgerID,
    },
    FullyValidateLedger {
        ledger: Ledger,
        prior: Ledger,
    },
    ShareProposal {
        proposal: Proposal,
    },
    ShareTx {
        tx: Tx,
    },
    ShareValidation {
        ledger_id: LedgerID,
    },
}

// ─── Collector trait ─────────────────────────────────────────────────────────

/// Trait for collecting simulation events.
pub trait Collector {
    fn on(&mut self, who: PeerID, when: SimTime, event: &SimEvent);
}

/// Null collector that discards all events.
pub struct NullCollector;
impl Collector for NullCollector {
    fn on(&mut self, _who: PeerID, _when: SimTime, _event: &SimEvent) {}
}

/// Collector that stores all events for later analysis.
#[derive(Default)]
pub struct EventLog {
    pub events: Vec<(PeerID, SimTime, SimEvent)>,
}

impl Collector for EventLog {
    fn on(&mut self, who: PeerID, when: SimTime, event: &SimEvent) {
        self.events.push((who, when, event.clone()));
    }
}

/// Collection of collectors (type-erased).
pub struct CollectorRefs {
    collectors: Vec<Box<dyn Collector>>,
}

impl Default for CollectorRefs {
    fn default() -> Self {
        Self::new()
    }
}

impl CollectorRefs {
    pub fn new() -> Self {
        Self {
            collectors: Vec::new(),
        }
    }

    pub fn add(&mut self, collector: impl Collector + 'static) {
        self.collectors.push(Box::new(collector));
    }

    pub fn on(&mut self, who: PeerID, when: SimTime, event: &SimEvent) {
        for c in &mut self.collectors {
            c.on(who, when, event);
        }
    }
}

// ─── Histogram ───────────────────────────────────────────────────────────────

/// Simple histogram for tracking value distributions.
#[derive(Debug, Clone, Default)]
pub struct Histogram {
    bins: HashMap<i64, u64>,
    count: u64,
    sum: f64,
}

impl Histogram {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, value: f64) {
        let bin = value.round() as i64;
        *self.bins.entry(bin).or_default() += 1;
        self.count += 1;
        self.sum += value;
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn avg(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum / self.count as f64
        }
    }

    pub fn min_val(&self) -> Option<i64> {
        self.bins.keys().copied().min()
    }

    pub fn max_val(&self) -> Option<i64> {
        self.bins.keys().copied().max()
    }
}

// ─── TxCollector ─────────────────────────────────────────────────────────────

/// Tracks transaction submission and acceptance statistics.
#[derive(Default)]
pub struct TxCollector {
    pub submitted: u64,
    pub accepted: u64,
    pub latency: Histogram,
}

impl Collector for TxCollector {
    fn on(&mut self, _who: PeerID, _when: SimTime, event: &SimEvent) {
        match event {
            SimEvent::SubmitTx { .. } => self.submitted += 1,
            SimEvent::AcceptLedger { ledger, .. } => {
                self.accepted += ledger.txs.len() as u64;
            }
            _ => {}
        }
    }
}

// ─── LedgerCollector ─────────────────────────────────────────────────────────

/// Tracks ledger close statistics.
#[derive(Default)]
pub struct LedgerCollector {
    pub closed: u64,
    pub fully_validated: u64,
}

impl Collector for LedgerCollector {
    fn on(&mut self, _who: PeerID, _when: SimTime, event: &SimEvent) {
        match event {
            SimEvent::AcceptLedger { .. } => self.closed += 1,
            SimEvent::FullyValidateLedger { .. } => self.fully_validated += 1,
            _ => {}
        }
    }
}

// ─── Random utilities ────────────────────────────────────────────────────────

use rand::Rng;
use rand::distributions::Distribution;

/// Randomly shuffle a vector based on weights.
///
pub fn random_weighted_shuffle<T: Clone, R: Rng>(
    mut v: Vec<T>,
    mut w: Vec<f64>,
    rng: &mut R,
) -> Vec<T> {
    for i in 0..v.len().saturating_sub(1) {
        let total: f64 = w[i..].iter().sum();
        if total <= 0.0 {
            break;
        }
        let threshold = rand::distributions::Uniform::new(0.0f64, 1.0).sample(rng) * total;
        let mut cumulative = 0.0;
        let mut idx = i;
        #[allow(clippy::needless_range_loop)]
        for j in i..w.len() {
            cumulative += w[j];
            if cumulative >= threshold {
                idx = j;
                break;
            }
        }
        v.swap(i, idx);
        w.swap(i, idx);
    }
    v
}

/// Generate a vector of random samples from a distribution.
///
pub fn sample_distribution<D: Distribution<f64>, R: Rng>(
    size: usize,
    dist: &D,
    rng: &mut R,
) -> Vec<f64> {
    (0..size).map(|_| dist.sample(rng)).collect()
}

/// Constant "distribution" that always returns the same value.
#[derive(Debug, Clone, Copy)]
pub struct ConstantDistribution(pub f64);

impl Distribution<f64> for ConstantDistribution {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> f64 {
        self.0
    }
}

/// Power-law distribution P(x) = (x/xmin)^-a.
#[derive(Debug, Clone)]
pub struct PowerLawDistribution {
    xmin: f64,
    inv: f64,
}

impl PowerLawDistribution {
    pub fn new(xmin: f64, a: f64) -> Self {
        Self {
            xmin,
            inv: 1.0 / (1.0 - a),
        }
    }
}

impl Distribution<f64> for PowerLawDistribution {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        let u: f64 = rand::distributions::Uniform::new(0.0f64, 1.0).sample(rng);
        self.xmin * (1.0f64 - u).powf(self.inv)
    }
}

// ─── Rate & Submitter ────────────────────────────────────────────────────────

/// Transaction submission rate.
#[derive(Debug, Clone, Copy)]
pub struct Rate {
    pub count: usize,
    pub duration: Duration,
}

impl Rate {
    pub fn inv(&self) -> f64 {
        self.duration.as_nanos() as f64 / self.count as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_tracks_stats() {
        let mut h = Histogram::new();
        h.insert(1.0);
        h.insert(2.0);
        h.insert(3.0);
        assert_eq!(h.count(), 3);
        assert!((h.avg() - 2.0).abs() < 0.001);
        assert_eq!(h.min_val(), Some(1));
        assert_eq!(h.max_val(), Some(3));
    }

    #[test]
    fn event_log_stores_events() {
        let mut log = EventLog::default();
        log.on(
            1,
            Duration::from_secs(1),
            &SimEvent::SubmitTx { tx: Tx::new(42) },
        );
        assert_eq!(log.events.len(), 1);
    }

    #[test]
    fn collector_refs_dispatches_to_all() {
        let mut refs = CollectorRefs::new();
        refs.add(EventLog::default());
        refs.on(1, Duration::ZERO, &SimEvent::SubmitTx { tx: Tx::new(1) });
        // Just verify no panic
    }

    #[test]
    fn constant_distribution_returns_fixed_value() {
        use rand::thread_rng;
        let dist = ConstantDistribution(42.0);
        let mut rng = thread_rng();
        assert_eq!(dist.sample(&mut rng), 42.0);
    }

    #[test]
    fn random_weighted_shuffle_respects_weights() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(12345);
        let v = vec![1, 2, 3, 4, 5];
        let w = vec![0.0, 0.0, 0.0, 0.0, 1.0]; // only item 5 has weight
        let result = random_weighted_shuffle(v, w, &mut rng);
        assert_eq!(result[0], 5); // item 5 should be first
    }

    #[test]
    fn tx_collector_counts_submissions() {
        let mut tc = TxCollector::default();
        tc.on(1, Duration::ZERO, &SimEvent::SubmitTx { tx: Tx::new(1) });
        tc.on(1, Duration::ZERO, &SimEvent::SubmitTx { tx: Tx::new(2) });
        assert_eq!(tc.submitted, 2);
    }
}
