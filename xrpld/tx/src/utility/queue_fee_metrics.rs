//! Deterministic fee-level helpers used by `TxQ`.
//!
//! This ports the the reference implementation behavior of:
//! 1. converting a paid XRP fee into a `FeeLevel64`,
//! 2. scaling the open-ledger required fee from a fee-metrics snapshot,
//! 3. exposing the current thin `getRequiredFeeLevel(...)` wrapper,
//! 4. updating the mutable `FeeMetrics` state used by `processClosedLedger(...)`.

use std::collections::VecDeque;

use basics::mul_div::mul_div;

use crate::{ApplyFlags, FeeLevel64};

/// Matches `xrpl::TxQ::baseLevel`.
pub const TXQ_BASE_LEVEL: FeeLevel64 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFeeMetricsSnapshot {
    pub txns_expected: usize,
    pub escalation_multiplier: FeeLevel64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFeeMetricsConfig {
    pub ledgers_in_queue: usize,
    pub queue_size_min: usize,
    pub minimum_escalation_multiplier: FeeLevel64,
    pub minimum_txn_in_ledger: usize,
    pub target_txn_in_ledger: usize,
    pub maximum_txn_in_ledger: Option<usize>,
    pub normal_consensus_increase_percent: u32,
    pub slow_consensus_decrease_percent: u32,
}

impl QueueFeeMetricsConfig {
    fn history_capacity(self) -> usize {
        self.ledgers_in_queue.max(1)
    }

    fn minimum_txn_count(self) -> usize {
        self.minimum_txn_in_ledger
    }

    fn target_txn_count(self) -> usize {
        self.target_txn_in_ledger.max(self.minimum_txn_count())
    }

    fn maximum_txn_count(self) -> Option<usize> {
        self.maximum_txn_in_ledger
            .map(|maximum| maximum.max(self.target_txn_count()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueFeeMetricsState {
    config: QueueFeeMetricsConfig,
    txns_expected: usize,
    recent_txn_counts: VecDeque<usize>,
    escalation_multiplier: FeeLevel64,
}

impl QueueFeeMetricsState {
    pub fn new(config: QueueFeeMetricsConfig) -> Self {
        Self {
            txns_expected: config.minimum_txn_count(),
            recent_txn_counts: VecDeque::with_capacity(config.history_capacity()),
            escalation_multiplier: config.minimum_escalation_multiplier,
            config,
        }
    }

    pub fn snapshot(&self) -> QueueFeeMetricsSnapshot {
        QueueFeeMetricsSnapshot {
            txns_expected: self.txns_expected,
            escalation_multiplier: self.escalation_multiplier,
        }
    }

    pub fn txns_expected(&self) -> usize {
        self.txns_expected
    }

    pub fn ledgers_in_queue(&self) -> usize {
        self.config.ledgers_in_queue
    }

    pub fn queue_size_min(&self) -> usize {
        self.config.queue_size_min
    }

    pub fn update(&mut self, validated_fee_levels: &[FeeLevel64], time_leap: bool) -> usize {
        let size = validated_fee_levels.len();

        if time_leap {
            let cut_pct = 100_u64 - u64::from(self.config.slow_consensus_decrease_percent);
            let upper_limit = std::cmp::max(
                mul_div(self.txns_expected as u64, cut_pct, 100).unwrap_or(u64::MAX),
                self.config.minimum_txn_count() as u64,
            );
            let clamped = mul_div(size as u64, cut_pct, 100)
                .unwrap_or(u64::MAX)
                .clamp(self.config.minimum_txn_count() as u64, upper_limit);
            self.txns_expected = usize::try_from(clamped).unwrap_or(usize::MAX);
            self.recent_txn_counts.clear();
        } else if size > self.txns_expected || size > self.config.target_txn_count() {
            let next_count = mul_div(
                size as u64,
                u64::from(100 + self.config.normal_consensus_increase_percent),
                100,
            )
            .unwrap_or(u64::MAX);
            self.recent_txn_counts
                .push_back(usize::try_from(next_count).unwrap_or(usize::MAX));
            while self.recent_txn_counts.len() > self.config.history_capacity() {
                self.recent_txn_counts.pop_front();
            }

            let max_recent = *self
                .recent_txn_counts
                .iter()
                .max()
                .expect("xrpl::TxQ::FeeMetrics::update : recent counts not empty");
            let next = if max_recent >= self.txns_expected {
                max_recent
            } else {
                ((self.txns_expected as u128 * 9) + max_recent as u128)
                    .checked_div(10)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(usize::MAX)
            };
            self.txns_expected = self
                .config
                .maximum_txn_count()
                .map_or(next, |maximum| next.min(maximum));
        }

        self.escalation_multiplier = if size == 0 {
            self.config.minimum_escalation_multiplier
        } else {
            let mut fee_levels = validated_fee_levels.to_vec();
            fee_levels.sort_unstable();
            let mid_high = fee_levels[size / 2] as u128;
            let mid_low = fee_levels[(size - 1) / 2] as u128;
            let median = (mid_high + mid_low)
                .div_ceil(2)
                .try_into()
                .unwrap_or(FeeLevel64::MAX);
            median.max(self.config.minimum_escalation_multiplier)
        };

        size
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFeeLevelPaidInputs {
    pub calculated_base_fee_drops: i64,
    pub fee_paid_drops: i64,
    pub default_base_fee_drops: i64,
}

fn checked_positive_u64(value: i64, label: &str) -> u64 {
    u64::try_from(value)
        .unwrap_or_else(|_| panic!("{label} must fit in u64 in narrowed TxQ fee-level port"))
}

fn checked_count_u64(value: usize, label: &str) -> u64 {
    u64::try_from(value)
        .unwrap_or_else(|_| panic!("{label} must fit in u64 in narrowed TxQ fee-metrics port"))
}

pub fn evaluate_fee_level_paid(inputs: QueueFeeLevelPaidInputs) -> FeeLevel64 {
    let fee_mod_drops = if inputs.calculated_base_fee_drops > 0 {
        0
    } else if inputs.default_base_fee_drops == 0 {
        1
    } else {
        inputs.default_base_fee_drops
    };

    let base_fee_drops = inputs.calculated_base_fee_drops + fee_mod_drops;
    let effective_fee_paid_drops = inputs.fee_paid_drops + fee_mod_drops;

    assert!(base_fee_drops > 0, "xrpl::getFeeLevelPaid : positive fee");

    if effective_fee_paid_drops <= 0 || base_fee_drops <= 0 {
        return 0;
    }

    let computed_level = mul_div(
        checked_positive_u64(effective_fee_paid_drops, "effective_fee_paid_drops"),
        TXQ_BASE_LEVEL,
        checked_positive_u64(base_fee_drops, "base_fee_drops"),
    )
    .unwrap_or(FeeLevel64::MAX);

    tracing::debug!(target: "tx", fee_paid = inputs.fee_paid_drops, base_fee = inputs.calculated_base_fee_drops, computed_level, "Fee level computed");

    computed_level
}

pub fn scale_fee_level(
    snapshot: QueueFeeMetricsSnapshot,
    open_ledger_tx_count: usize,
) -> FeeLevel64 {
    if open_ledger_tx_count <= snapshot.txns_expected {
        return TXQ_BASE_LEVEL;
    }

    let current = checked_count_u64(open_ledger_tx_count, "open_ledger_tx_count");
    let target = checked_count_u64(snapshot.txns_expected, "snapshot.txns_expected");

    let fee_level = mul_div(
        snapshot.escalation_multiplier,
        current.saturating_mul(current),
        target.saturating_mul(target),
    )
    .unwrap_or(FeeLevel64::MAX);

    tracing::info!(target: "tx", fee_level, open_ledger_count = open_ledger_tx_count, target = snapshot.txns_expected, "Fee escalation active");

    fee_level
}

pub fn evaluate_required_fee_level(
    snapshot: QueueFeeMetricsSnapshot,
    open_ledger_tx_count: usize,
    _flags: ApplyFlags,
) -> FeeLevel64 {
    scale_fee_level(snapshot, open_ledger_tx_count)
}

#[cfg(test)]
mod tests {
    use super::{
        QueueFeeLevelPaidInputs, QueueFeeMetricsConfig, QueueFeeMetricsSnapshot,
        QueueFeeMetricsState, TXQ_BASE_LEVEL, evaluate_fee_level_paid, evaluate_required_fee_level,
        scale_fee_level,
    };
    use crate::ApplyFlags;

    fn metrics_config() -> QueueFeeMetricsConfig {
        QueueFeeMetricsConfig {
            ledgers_in_queue: 3,
            queue_size_min: 20,
            minimum_escalation_multiplier: TXQ_BASE_LEVEL * 500,
            minimum_txn_in_ledger: 32,
            target_txn_in_ledger: 256,
            maximum_txn_in_ledger: Some(400),
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
        }
    }

    #[test]
    fn fee_level_paid_matches_current_cpp_ratio_rule() {
        assert_eq!(
            evaluate_fee_level_paid(QueueFeeLevelPaidInputs {
                calculated_base_fee_drops: 10,
                fee_paid_drops: 20,
                default_base_fee_drops: 10,
            }),
            512
        );
    }

    #[test]
    fn fee_level_paid_uses_default_base_fee_when_calculated_base_is_zero() {
        assert_eq!(
            evaluate_fee_level_paid(QueueFeeLevelPaidInputs {
                calculated_base_fee_drops: 0,
                fee_paid_drops: 0,
                default_base_fee_drops: 10,
            }),
            TXQ_BASE_LEVEL
        );
    }

    #[test]
    fn fee_level_paid_uses_one_drop_mod_when_both_base_fees_are_zero() {
        assert_eq!(
            evaluate_fee_level_paid(QueueFeeLevelPaidInputs {
                calculated_base_fee_drops: 0,
                fee_paid_drops: 0,
                default_base_fee_drops: 0,
            }),
            TXQ_BASE_LEVEL
        );
    }

    #[test]
    fn fee_level_paid_returns_zero_for_non_positive_effective_fee() {
        assert_eq!(
            evaluate_fee_level_paid(QueueFeeLevelPaidInputs {
                calculated_base_fee_drops: 10,
                fee_paid_drops: -1,
                default_base_fee_drops: 10,
            }),
            0
        );
    }

    #[test]
    fn scale_fee_level_stays_at_base_level_until_target_is_exceeded() {
        let snapshot = QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        };

        assert_eq!(scale_fee_level(snapshot, 0), TXQ_BASE_LEVEL);
        assert_eq!(scale_fee_level(snapshot, 32), TXQ_BASE_LEVEL);
    }

    #[test]
    fn scale_fee_level_matches_current_cpp_quadratic_escalation_rule() {
        let snapshot = QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        };

        assert_eq!(scale_fee_level(snapshot, 33), 136_125);
    }

    #[test]
    fn scale_fee_level_saturates_on_mul_div_overflow() {
        let snapshot = QueueFeeMetricsSnapshot {
            txns_expected: 1,
            escalation_multiplier: u64::MAX,
        };

        assert_eq!(scale_fee_level(snapshot, 2), u64::MAX);
    }

    #[test]
    fn required_fee_level_currently_delegates_to_scaled_open_ledger_fee() {
        let snapshot = QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        };

        assert_eq!(
            evaluate_required_fee_level(snapshot, 33, ApplyFlags::NONE),
            scale_fee_level(snapshot, 33)
        );
        assert_eq!(
            evaluate_required_fee_level(snapshot, 33, ApplyFlags::FAIL_HARD),
            scale_fee_level(snapshot, 33)
        );
    }

    #[test]
    fn fee_metrics_state_starts_at_config_minimums() {
        let state = QueueFeeMetricsState::new(metrics_config());

        assert_eq!(state.txns_expected(), 32);
        assert_eq!(
            state.snapshot(),
            QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: TXQ_BASE_LEVEL * 500,
            }
        );
        assert_eq!(state.ledgers_in_queue(), 3);
        assert_eq!(state.queue_size_min(), 20);
    }

    #[test]
    fn fee_metrics_update_grows_expected_and_tracks_median() {
        let mut state = QueueFeeMetricsState::new(metrics_config());

        let size = state.update(
            &[
                TXQ_BASE_LEVEL * 500,
                TXQ_BASE_LEVEL * 700,
                TXQ_BASE_LEVEL * 900,
            ],
            false,
        );

        assert_eq!(size, 3);
        assert_eq!(state.txns_expected(), 32);
        assert_eq!(state.snapshot().escalation_multiplier, TXQ_BASE_LEVEL * 700);

        let size = state.update(&vec![TXQ_BASE_LEVEL * 600; 300], false);

        assert_eq!(size, 300);
        assert_eq!(state.txns_expected(), 360);
        assert_eq!(state.snapshot().escalation_multiplier, TXQ_BASE_LEVEL * 600);
    }

    #[test]
    fn fee_metrics_update_time_leap_clamps_and_clears_history() {
        let mut state = QueueFeeMetricsState::new(metrics_config());
        let _ = state.update(&vec![TXQ_BASE_LEVEL * 600; 300], false);
        assert_eq!(state.txns_expected(), 360);

        let size = state.update(&vec![TXQ_BASE_LEVEL * 800; 150], true);

        assert_eq!(size, 150);
        assert_eq!(state.txns_expected(), 75);
        assert_eq!(state.snapshot().escalation_multiplier, TXQ_BASE_LEVEL * 800);

        let _ = state.update(&[TXQ_BASE_LEVEL * 500], false);
        assert_eq!(state.txns_expected(), 75);
    }

    #[test]
    fn fee_metrics_update_resets_multiplier_to_minimum_when_ledger_is_empty() {
        let mut state = QueueFeeMetricsState::new(metrics_config());
        let _ = state.update(&[TXQ_BASE_LEVEL * 900], false);

        let size = state.update(&[], false);

        assert_eq!(size, 0);
        assert_eq!(
            state.snapshot().escalation_multiplier,
            metrics_config().minimum_escalation_multiplier
        );
    }
}
