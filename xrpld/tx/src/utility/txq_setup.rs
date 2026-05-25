//! `xrpld/app/misc/TxQ.h` setup parsing.
//!
//! This ports the current `setup_TxQ(Config const&)` role from reference:
//! - read the `transaction_queue` config section,
//! - preserve current defaults when values are absent or unparsable,
//! - enforce the same invalid `maximum_txn_in_ledger` checks,
//! - clamp the same consensus percentages,
//! - carry the current standalone-mode switch explicitly.

use std::fmt;

use basics::basic_config::{BasicConfig, set};

use crate::{FeeLevel64, QueueFeeMetricsConfig, QueueFeeMetricsState, TXQ_BASE_LEVEL};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxQSetupError {
    MaximumBelowMinimumTxnInLedger,
    MaximumBelowMinimumTxnInLedgerStandalone,
}

impl fmt::Display for TxQSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaximumBelowMinimumTxnInLedger => write!(
                formatter,
                "The minimum number of low-fee transactions allowed per ledger \
(minimum_txn_in_ledger) exceeds the maximum number of low-fee transactions \
allowed per ledger (maximum_txn_in_ledger)."
            ),
            Self::MaximumBelowMinimumTxnInLedgerStandalone => write!(
                formatter,
                "The minimum number of low-fee transactions allowed per ledger \
(minimum_txn_in_ledger_standalone) exceeds the maximum number of low-fee \
transactions allowed per ledger (maximum_txn_in_ledger)."
            ),
        }
    }
}

impl std::error::Error for TxQSetupError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxQSetup {
    pub ledgers_in_queue: usize,
    pub queue_size_min: usize,
    pub retry_sequence_percent: u32,
    pub minimum_escalation_multiplier: FeeLevel64,
    pub minimum_txn_in_ledger: usize,
    pub minimum_txn_in_ledger_standalone: usize,
    pub target_txn_in_ledger: usize,
    pub maximum_txn_in_ledger: Option<usize>,
    pub normal_consensus_increase_percent: u32,
    pub slow_consensus_decrease_percent: u32,
    pub maximum_txn_per_account: u32,
    pub minimum_last_ledger_buffer: u32,
    pub standalone: bool,
}

impl Default for TxQSetup {
    fn default() -> Self {
        Self {
            ledgers_in_queue: 20,
            queue_size_min: 2000,
            retry_sequence_percent: 25,
            minimum_escalation_multiplier: TXQ_BASE_LEVEL * 500,
            minimum_txn_in_ledger: 32,
            minimum_txn_in_ledger_standalone: 1000,
            target_txn_in_ledger: 256,
            maximum_txn_in_ledger: None,
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
            maximum_txn_per_account: 10,
            minimum_last_ledger_buffer: 2,
            standalone: false,
        }
    }
}

impl TxQSetup {
    pub fn minimum_txn_in_ledger_for_mode(&self) -> usize {
        if self.standalone {
            self.minimum_txn_in_ledger_standalone
        } else {
            self.minimum_txn_in_ledger
        }
    }

    pub fn fee_metrics_config(&self) -> QueueFeeMetricsConfig {
        QueueFeeMetricsConfig {
            ledgers_in_queue: self.ledgers_in_queue,
            queue_size_min: self.queue_size_min,
            minimum_escalation_multiplier: self.minimum_escalation_multiplier,
            minimum_txn_in_ledger: self.minimum_txn_in_ledger_for_mode(),
            target_txn_in_ledger: self.target_txn_in_ledger,
            maximum_txn_in_ledger: self.maximum_txn_in_ledger,
            normal_consensus_increase_percent: self.normal_consensus_increase_percent,
            slow_consensus_decrease_percent: self.slow_consensus_decrease_percent,
        }
    }

    pub fn fee_metrics_state(&self) -> QueueFeeMetricsState {
        QueueFeeMetricsState::new(self.fee_metrics_config())
    }
}

pub fn setup_txq(config: &BasicConfig, standalone: bool) -> Result<TxQSetup, TxQSetupError> {
    let mut setup = TxQSetup::default();
    let section = config.section("transaction_queue");

    set(&mut setup.ledgers_in_queue, "ledgers_in_queue", section);
    set(&mut setup.queue_size_min, "minimum_queue_size", section);
    set(
        &mut setup.retry_sequence_percent,
        "retry_sequence_percent",
        section,
    );
    set(
        &mut setup.minimum_escalation_multiplier,
        "minimum_escalation_multiplier",
        section,
    );
    set(
        &mut setup.minimum_txn_in_ledger,
        "minimum_txn_in_ledger",
        section,
    );
    set(
        &mut setup.minimum_txn_in_ledger_standalone,
        "minimum_txn_in_ledger_standalone",
        section,
    );
    set(
        &mut setup.target_txn_in_ledger,
        "target_txn_in_ledger",
        section,
    );

    let mut maximum_txn_in_ledger = 0usize;
    if set(&mut maximum_txn_in_ledger, "maximum_txn_in_ledger", section) {
        if maximum_txn_in_ledger < setup.minimum_txn_in_ledger {
            return Err(TxQSetupError::MaximumBelowMinimumTxnInLedger);
        }
        if maximum_txn_in_ledger < setup.minimum_txn_in_ledger_standalone {
            return Err(TxQSetupError::MaximumBelowMinimumTxnInLedgerStandalone);
        }

        setup.maximum_txn_in_ledger = Some(maximum_txn_in_ledger);
    }

    set(
        &mut setup.normal_consensus_increase_percent,
        "normal_consensus_increase_percent",
        section,
    );
    setup.normal_consensus_increase_percent =
        setup.normal_consensus_increase_percent.clamp(0, 1000);

    set(
        &mut setup.slow_consensus_decrease_percent,
        "slow_consensus_decrease_percent",
        section,
    );
    setup.slow_consensus_decrease_percent = setup.slow_consensus_decrease_percent.clamp(0, 100);

    set(
        &mut setup.maximum_txn_per_account,
        "maximum_txn_per_account",
        section,
    );
    set(
        &mut setup.minimum_last_ledger_buffer,
        "minimum_last_ledger_buffer",
        section,
    );

    setup.standalone = standalone;
    Ok(setup)
}
