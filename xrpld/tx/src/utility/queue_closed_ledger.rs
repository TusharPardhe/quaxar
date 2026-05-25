//! Deterministic maintenance half of `TxQ::processClosedLedger(...)`.
//!
//! This ports the queue-size update rule, expiry by `LastLedgerSequence`,
//! per-account `dropPenalty` propagation, and empty-account cleanup.

use std::collections::BTreeMap;

use protocol::SeqProxy;

use crate::{FeeLevel64, QueueFeeMetricsSnapshot, QueueFeeMetricsState, TxQAccount};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosedLedgerCandidate<Account> {
    pub account: Account,
    pub seq_proxy: SeqProxy,
    pub last_valid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedLedgerMaintenance<Account> {
    pub next_max_size: usize,
    pub expired_candidates: Vec<ClosedLedgerCandidate<Account>>,
    pub emptied_accounts: Vec<Account>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedLedgerMaintenanceWithMetrics<Account> {
    pub validated_tx_count: usize,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub maintenance: ClosedLedgerMaintenance<Account>,
}

pub fn process_closed_ledger<Account, T, I>(
    accounts: &mut BTreeMap<Account, TxQAccount<Account, T>>,
    by_fee_candidates: I,
    ledger_seq: u32,
    current_max_size: usize,
    time_leap: bool,
    txns_expected: usize,
    ledgers_in_queue: usize,
    queue_size_min: usize,
) -> ClosedLedgerMaintenance<Account>
where
    Account: Clone + Ord,
    I: IntoIterator<Item = ClosedLedgerCandidate<Account>>,
{
    let next_max_size = if time_leap {
        current_max_size
    } else {
        // Match the reference unsigned-size behavior instead of panicking on debug
        // overflow when the queue-sizing math gets extreme.
        std::cmp::max(txns_expected.wrapping_mul(ledgers_in_queue), queue_size_min)
    };

    let mut expired_candidates = Vec::new();

    for candidate in by_fee_candidates {
        if candidate
            .last_valid
            .is_none_or(|last_valid| last_valid > ledger_seq)
        {
            continue;
        }

        let account = accounts
            .get_mut(&candidate.account)
            .expect("xrpl::TxQ::processClosedLedger : account found");
        assert!(
            account.account == candidate.account,
            "xrpl::TxQ::processClosedLedger : account matches key"
        );
        account.drop_penalty = true;
        assert!(
            account.remove(candidate.seq_proxy),
            "xrpl::TxQ::processClosedLedger : candidate found in account"
        );
        expired_candidates.push(candidate);
    }

    let emptied_accounts = accounts
        .iter()
        .filter(|(_, account)| account.empty())
        .map(|(account, _)| account.clone())
        .collect::<Vec<_>>();

    for account in &emptied_accounts {
        let removed = accounts.remove(account);
        assert!(
            removed.is_some(),
            "xrpl::TxQ::processClosedLedger : empty account removed"
        );
    }

    ClosedLedgerMaintenance {
        next_max_size,
        expired_candidates,
        emptied_accounts,
    }
}

pub fn process_closed_ledger_with_metrics<Account, T, I>(
    metrics: &mut QueueFeeMetricsState,
    validated_fee_levels: &[FeeLevel64],
    accounts: &mut BTreeMap<Account, TxQAccount<Account, T>>,
    by_fee_candidates: I,
    ledger_seq: u32,
    current_max_size: usize,
    time_leap: bool,
) -> ClosedLedgerMaintenanceWithMetrics<Account>
where
    Account: Clone + Ord,
    I: IntoIterator<Item = ClosedLedgerCandidate<Account>>,
{
    let validated_tx_count = metrics.update(validated_fee_levels, time_leap);
    let metrics_snapshot = metrics.snapshot();
    let maintenance = process_closed_ledger(
        accounts,
        by_fee_candidates,
        ledger_seq,
        current_max_size,
        time_leap,
        metrics_snapshot.txns_expected,
        metrics.ledgers_in_queue(),
        metrics.queue_size_min(),
    );

    ClosedLedgerMaintenanceWithMetrics {
        validated_tx_count,
        metrics_snapshot,
        maintenance,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use protocol::SeqProxy;

    use super::{
        ClosedLedgerCandidate, ClosedLedgerMaintenanceWithMetrics, process_closed_ledger,
        process_closed_ledger_with_metrics,
    };
    use crate::{
        MaybeTxCore, QueueFeeMetricsConfig, QueueFeeMetricsSnapshot, QueueFeeMetricsState,
        TXQ_BASE_LEVEL, TxConsequences, TxQAccount,
    };

    fn metrics_state() -> QueueFeeMetricsState {
        QueueFeeMetricsState::new(QueueFeeMetricsConfig {
            ledgers_in_queue: 3,
            queue_size_min: 20,
            minimum_escalation_multiplier: TXQ_BASE_LEVEL * 500,
            minimum_txn_in_ledger: 32,
            target_txn_in_ledger: 256,
            maximum_txn_in_ledger: Some(400),
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
        })
    }

    #[test]
    fn process_closed_ledger_updates_max_size_only_without_time_leap() {
        let mut accounts = BTreeMap::<&str, TxQAccount<&str, &str>>::new();

        let normal = process_closed_ledger(&mut accounts, [], 10, 90, false, 12, 3, 20);
        let time_leap = process_closed_ledger(&mut accounts, [], 10, 90, true, 40, 5, 20);

        assert_eq!(normal.next_max_size, 36);
        assert_eq!(time_leap.next_max_size, 90);
    }

    #[test]
    fn process_closed_ledger_wraps_queue_size_math_unsigned_arithmetic() {
        let mut accounts = BTreeMap::<&str, TxQAccount<&str, &str>>::new();

        let result =
            process_closed_ledger(&mut accounts, [], 10, 90, false, usize::MAX / 2 + 1, 2, 20);

        assert_eq!(result.next_max_size, 20);
    }

    #[test]
    fn process_closed_ledger_removes_expired_candidates_and_empty_accounts() {
        let mut keep = TxQAccount::new("keep");
        keep.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("expired", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        keep.add(
            SeqProxy::sequence(6),
            MaybeTxCore::new("live", TxConsequences::new(1, SeqProxy::sequence(6))),
        );

        let mut drop = TxQAccount::new("drop");
        drop.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new("expired", TxConsequences::new(1, SeqProxy::sequence(9))),
        );

        let mut accounts = BTreeMap::from([("drop", drop), ("keep", keep)]);

        let result = process_closed_ledger(
            &mut accounts,
            [
                ClosedLedgerCandidate {
                    account: "keep",
                    seq_proxy: SeqProxy::sequence(5),
                    last_valid: Some(20),
                },
                ClosedLedgerCandidate {
                    account: "keep",
                    seq_proxy: SeqProxy::sequence(6),
                    last_valid: Some(21),
                },
                ClosedLedgerCandidate {
                    account: "drop",
                    seq_proxy: SeqProxy::sequence(9),
                    last_valid: Some(20),
                },
                ClosedLedgerCandidate {
                    account: "keep",
                    seq_proxy: SeqProxy::ticket(3),
                    last_valid: None,
                },
            ],
            20,
            50,
            false,
            8,
            2,
            30,
        );

        assert_eq!(result.next_max_size, 30);
        assert_eq!(
            result.expired_candidates,
            vec![
                ClosedLedgerCandidate {
                    account: "keep",
                    seq_proxy: SeqProxy::sequence(5),
                    last_valid: Some(20),
                },
                ClosedLedgerCandidate {
                    account: "drop",
                    seq_proxy: SeqProxy::sequence(9),
                    last_valid: Some(20),
                },
            ]
        );
        assert_eq!(result.emptied_accounts, vec!["drop"]);

        let keep_account = accounts.get("keep").expect("keep account should remain");
        assert!(keep_account.drop_penalty);
        assert_eq!(keep_account.get_txn_count(), 1);
        assert!(
            keep_account
                .transactions
                .contains_key(&SeqProxy::sequence(6))
        );
        assert!(!accounts.contains_key("drop"));
    }

    #[test]
    fn process_closed_ledger_with_metrics_updates_fee_metrics_before_maintenance() {
        let mut keep = TxQAccount::new("keep");
        keep.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("expired", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut accounts = BTreeMap::from([("keep", keep)]);
        let mut metrics = metrics_state();

        let result = process_closed_ledger_with_metrics(
            &mut metrics,
            &vec![TXQ_BASE_LEVEL * 600; 300],
            &mut accounts,
            [ClosedLedgerCandidate {
                account: "keep",
                seq_proxy: SeqProxy::sequence(5),
                last_valid: Some(20),
            }],
            20,
            50,
            false,
        );

        assert_eq!(
            result,
            ClosedLedgerMaintenanceWithMetrics {
                validated_tx_count: 300,
                metrics_snapshot: QueueFeeMetricsSnapshot {
                    txns_expected: 360,
                    escalation_multiplier: TXQ_BASE_LEVEL * 600,
                },
                maintenance: super::ClosedLedgerMaintenance {
                    next_max_size: 1080,
                    expired_candidates: vec![ClosedLedgerCandidate {
                        account: "keep",
                        seq_proxy: SeqProxy::sequence(5),
                        last_valid: Some(20),
                    }],
                    emptied_accounts: vec!["keep"],
                },
            }
        );
        assert!(accounts.is_empty());
        assert_eq!(metrics.snapshot(), result.metrics_snapshot);
    }
}
