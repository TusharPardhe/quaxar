//! Observed transaction and view facts that the current top-level
//! `TxQ::apply(...)` caller has in hand before it lowers those facts into the
//! landed read-side builder.
//!
//! This captures the already-observed facts in one typed seam and lowers them
//! into `QueueApplyTopReadInputs`.

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};

use crate::{
    ApplyFlags, OrderCandidates, QueueApplyLedgerAccountState, QueueApplyLedgerReadState,
    QueueApplyLedgerTicketState, QueueApplyTopReadInputs, QueueFeeMetricsSnapshot,
    QueueHoldPreflight, TxConsequences,
};

pub trait QueueApplyObservedTxSource {
    type Account;
    type TransactionId: Clone;

    fn account(&self) -> &Self::Account;
    fn transaction_id(&self) -> Self::TransactionId;
    fn tx_id(&self) -> Uint256;
    fn tx_seq_proxy(&self) -> SeqProxy;
}

/// Narrow tx-field seam for the deterministic `TxQ::canBeHeld(...)` facts that
/// the higher `xrpld` apply wrappers can honestly derive from the transaction
/// without claiming full `STTx` ownership.
pub trait QueueApplyHoldPreflightTxSource: QueueApplyObservedTxSource {
    fn has_previous_txn_id(&self) -> bool;
    fn has_account_txn_id(&self) -> bool;
    fn last_valid_ledger(&self) -> Option<u32>;
}

pub trait QueueApplyObservedViewSource<Account> {
    fn rules(&self) -> &Rules;
    fn account_lookup(&self, account: &Account) -> QueueApplyObservedAccountLookup;
    fn ticket_lookup(
        &self,
        account: &Account,
        tx_seq_proxy: SeqProxy,
    ) -> QueueApplyObservedTicketLookup;
    fn calculated_base_fee_drops(&self) -> i64;
    fn fee_paid_drops(&self) -> i64;
    fn default_base_fee_drops(&self) -> i64;
    fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot;
    fn open_ledger_tx_count(&self) -> usize;
    fn open_ledger_seq(&self) -> u32;
    fn reserve_drops(&self) -> u64;
    fn base_fee_drops(&self) -> u64;
}

#[derive(Debug, Clone)]
pub struct QueueApplyObservedTxInputs<'a, Account, TxId> {
    pub account: &'a Account,
    pub transaction_id: TxId,
    pub tx_id: Uint256,
    pub tx_seq_proxy: SeqProxy,
}

#[derive(Debug, Clone)]
pub struct QueueApplyObservedTx<'a, Account, TxId> {
    pub account: &'a Account,
    pub transaction_id: TxId,
    pub tx_id: Uint256,
    pub tx_seq_proxy: SeqProxy,
    pub preflight: QueueHoldPreflight,
    pub flags: ApplyFlags,
    pub consequences: TxConsequences,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyObservedAccountLookup {
    Missing,
    Present { sequence: u32, balance_drops: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueApplyObservedTicketLookup {
    NotRequired,
    Present,
    Missing,
}

#[derive(Debug, Clone, Copy)]
pub struct QueueApplyObservedViewInputs<'a> {
    pub rules: &'a Rules,
    pub account_lookup: QueueApplyObservedAccountLookup,
    pub ticket_lookup: QueueApplyObservedTicketLookup,
    pub calculated_base_fee_drops: i64,
    pub fee_paid_drops: i64,
    pub default_base_fee_drops: i64,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub open_ledger_tx_count: usize,
    pub open_ledger_seq: u32,
    pub reserve_drops: u64,
    pub base_fee_drops: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct QueueApplyObservedView<'a> {
    pub rules: &'a Rules,
    pub ledger_read: QueueApplyLedgerReadState,
    pub calculated_base_fee_drops: i64,
    pub fee_paid_drops: i64,
    pub default_base_fee_drops: i64,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub open_ledger_tx_count: usize,
    pub open_ledger_seq: u32,
    pub reserve_drops: u64,
    pub base_fee_drops: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct QueueApplyObservedQueue<'a> {
    pub minimum_last_ledger_buffer: u32,
    pub maximum_txn_per_account: usize,
    pub retry_sequence_percent: u32,
    pub queue_is_full: bool,
    pub can_be_held_result: Ter,
    pub order: &'a OrderCandidates,
}

pub fn build_queue_apply_observed_tx_inputs_from_source<'a, Source>(
    tx: &'a Source,
) -> QueueApplyObservedTxInputs<'a, Source::Account, Source::TransactionId>
where
    Source: QueueApplyObservedTxSource,
{
    QueueApplyObservedTxInputs {
        account: tx.account(),
        transaction_id: tx.transaction_id(),
        tx_id: tx.tx_id(),
        tx_seq_proxy: tx.tx_seq_proxy(),
    }
}

pub fn build_queue_apply_observed_tx<'a, Account, TxId>(
    inputs: QueueApplyObservedTxInputs<'a, Account, TxId>,
    preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
) -> QueueApplyObservedTx<'a, Account, TxId> {
    QueueApplyObservedTx {
        account: inputs.account,
        transaction_id: inputs.transaction_id,
        tx_id: inputs.tx_id,
        tx_seq_proxy: inputs.tx_seq_proxy,
        preflight,
        flags,
        consequences,
    }
}

pub fn derive_queue_hold_preflight_from_tx_source<TxSource>(
    tx_source: &TxSource,
    flags: ApplyFlags,
) -> QueueHoldPreflight
where
    TxSource: QueueApplyHoldPreflightTxSource,
{
    QueueHoldPreflight::new(
        tx_source.has_previous_txn_id(),
        tx_source.has_account_txn_id(),
        flags,
        tx_source.last_valid_ledger(),
    )
}

pub fn build_queue_apply_observed_view<'a>(
    inputs: QueueApplyObservedViewInputs<'a>,
) -> QueueApplyObservedView<'a> {
    QueueApplyObservedView {
        rules: inputs.rules,
        ledger_read: QueueApplyLedgerReadState {
            account: match inputs.account_lookup {
                QueueApplyObservedAccountLookup::Missing => QueueApplyLedgerAccountState::Missing,
                QueueApplyObservedAccountLookup::Present {
                    sequence,
                    balance_drops,
                } => QueueApplyLedgerAccountState::Present {
                    seq_proxy: SeqProxy::sequence(sequence),
                    balance_drops,
                },
            },
            ticket: match inputs.ticket_lookup {
                QueueApplyObservedTicketLookup::NotRequired => {
                    QueueApplyLedgerTicketState::NotRequired
                }
                QueueApplyObservedTicketLookup::Present => QueueApplyLedgerTicketState::Present,
                QueueApplyObservedTicketLookup::Missing => QueueApplyLedgerTicketState::Missing,
            },
        },
        calculated_base_fee_drops: inputs.calculated_base_fee_drops,
        fee_paid_drops: inputs.fee_paid_drops,
        default_base_fee_drops: inputs.default_base_fee_drops,
        metrics_snapshot: inputs.metrics_snapshot,
        open_ledger_tx_count: inputs.open_ledger_tx_count,
        open_ledger_seq: inputs.open_ledger_seq,
        reserve_drops: inputs.reserve_drops,
        base_fee_drops: inputs.base_fee_drops,
    }
}

pub fn build_queue_apply_observed_view_inputs_from_source<'a, Account, Source>(
    source: &'a Source,
    account: &Account,
    tx_seq_proxy: SeqProxy,
) -> QueueApplyObservedViewInputs<'a>
where
    Source: QueueApplyObservedViewSource<Account>,
{
    QueueApplyObservedViewInputs {
        rules: source.rules(),
        account_lookup: source.account_lookup(account),
        ticket_lookup: source.ticket_lookup(account, tx_seq_proxy),
        calculated_base_fee_drops: source.calculated_base_fee_drops(),
        fee_paid_drops: source.fee_paid_drops(),
        default_base_fee_drops: source.default_base_fee_drops(),
        metrics_snapshot: source.metrics_snapshot(),
        open_ledger_tx_count: source.open_ledger_tx_count(),
        open_ledger_seq: source.open_ledger_seq(),
        reserve_drops: source.reserve_drops(),
        base_fee_drops: source.base_fee_drops(),
    }
}

pub fn build_queue_apply_top_read_inputs_from_observed<'a, Account, TxId>(
    observed_tx: QueueApplyObservedTx<'a, Account, TxId>,
    observed_view: QueueApplyObservedView<'a>,
    observed_queue: QueueApplyObservedQueue<'a>,
) -> QueueApplyTopReadInputs<'a, Account, TxId> {
    QueueApplyTopReadInputs {
        rules: observed_view.rules,
        account: observed_tx.account,
        transaction_id: observed_tx.transaction_id,
        tx_seq_proxy: observed_tx.tx_seq_proxy,
        ledger_read: observed_view.ledger_read,
        calculated_base_fee_drops: observed_view.calculated_base_fee_drops,
        fee_paid_drops: observed_view.fee_paid_drops,
        default_base_fee_drops: observed_view.default_base_fee_drops,
        metrics_snapshot: observed_view.metrics_snapshot,
        open_ledger_tx_count: observed_view.open_ledger_tx_count,
        flags: observed_tx.flags,
        preflight: observed_tx.preflight,
        is_blocker: observed_tx.consequences.is_blocker(),
        open_ledger_seq: observed_view.open_ledger_seq,
        minimum_last_ledger_buffer: observed_queue.minimum_last_ledger_buffer,
        maximum_txn_per_account: observed_queue.maximum_txn_per_account,
        retry_sequence_percent: observed_queue.retry_sequence_percent,
        queue_is_full: observed_queue.queue_is_full,
        reserve_drops: observed_view.reserve_drops,
        base_fee_drops: observed_view.base_fee_drops,
        can_be_held_result: observed_queue.can_be_held_result,
        last_valid: observed_tx.preflight.last_valid_ledger,
        order: observed_queue.order,
        tx_id: observed_tx.tx_id,
    }
}

pub fn build_queue_apply_top_read_inputs_from_observed_facts<'a, Account, TxId>(
    tx_inputs: QueueApplyObservedTxInputs<'a, Account, TxId>,
    preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    view_inputs: QueueApplyObservedViewInputs<'a>,
    queue_inputs: QueueApplyObservedQueue<'a>,
) -> QueueApplyTopReadInputs<'a, Account, TxId> {
    build_queue_apply_top_read_inputs_from_observed(
        build_queue_apply_observed_tx(tx_inputs, preflight, flags, consequences),
        build_queue_apply_observed_view(view_inputs),
        queue_inputs,
    )
}

pub fn build_queue_apply_top_read_inputs_from_sources<'a, TxSource, ViewSource>(
    tx_source: &'a TxSource,
    view_source: &'a ViewSource,
    preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    queue_inputs: QueueApplyObservedQueue<'a>,
) -> QueueApplyTopReadInputs<'a, TxSource::Account, TxSource::TransactionId>
where
    TxSource: QueueApplyObservedTxSource,
    ViewSource: QueueApplyObservedViewSource<TxSource::Account>,
{
    let tx_inputs = build_queue_apply_observed_tx_inputs_from_source(tx_source);
    let view_inputs = build_queue_apply_observed_view_inputs_from_source(
        view_source,
        tx_inputs.account,
        tx_inputs.tx_seq_proxy,
    );

    build_queue_apply_top_read_inputs_from_observed_facts(
        tx_inputs,
        preflight,
        flags,
        consequences,
        view_inputs,
        queue_inputs,
    )
}

#[cfg(test)]
mod tests {
    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueApplyObservedAccountLookup, QueueApplyObservedQueue, QueueApplyObservedTicketLookup,
        QueueApplyObservedTx, QueueApplyObservedTxInputs, QueueApplyObservedTxSource,
        QueueApplyObservedView, QueueApplyObservedViewInputs, QueueApplyObservedViewSource,
        build_queue_apply_observed_tx, build_queue_apply_observed_tx_inputs_from_source,
        build_queue_apply_observed_view, build_queue_apply_observed_view_inputs_from_source,
        build_queue_apply_top_read_inputs_from_observed,
        build_queue_apply_top_read_inputs_from_observed_facts,
        build_queue_apply_top_read_inputs_from_sources,
    };
    use crate::{
        ApplyFlags, OrderCandidates, QueueApplyLedgerAccountState, QueueApplyLedgerReadState,
        QueueApplyLedgerTicketState, QueueFeeMetricsSnapshot, QueueHoldPreflight, TxConsequences,
        TxConsequencesCategory,
    };

    #[derive(Debug)]
    struct TestObservedTxSource<'a> {
        account: &'a String,
        transaction_id: &'static str,
        tx_id: Uint256,
        tx_seq_proxy: SeqProxy,
    }

    impl QueueApplyObservedTxSource for TestObservedTxSource<'_> {
        type Account = String;
        type TransactionId = &'static str;

        fn account(&self) -> &Self::Account {
            self.account
        }

        fn transaction_id(&self) -> Self::TransactionId {
            self.transaction_id
        }

        fn tx_id(&self) -> Uint256 {
            self.tx_id
        }

        fn tx_seq_proxy(&self) -> SeqProxy {
            self.tx_seq_proxy
        }
    }

    #[derive(Debug)]
    struct TestObservedViewSource {
        rules: Rules,
        account_lookup: QueueApplyObservedAccountLookup,
        ticket_lookup: QueueApplyObservedTicketLookup,
        calculated_base_fee_drops: i64,
        fee_paid_drops: i64,
        default_base_fee_drops: i64,
        metrics_snapshot: QueueFeeMetricsSnapshot,
        open_ledger_tx_count: usize,
        open_ledger_seq: u32,
        reserve_drops: u64,
        base_fee_drops: u64,
    }

    impl QueueApplyObservedViewSource<String> for TestObservedViewSource {
        fn rules(&self) -> &Rules {
            &self.rules
        }

        fn account_lookup(&self, _account: &String) -> QueueApplyObservedAccountLookup {
            self.account_lookup
        }

        fn ticket_lookup(
            &self,
            _account: &String,
            _tx_seq_proxy: SeqProxy,
        ) -> QueueApplyObservedTicketLookup {
            self.ticket_lookup
        }

        fn calculated_base_fee_drops(&self) -> i64 {
            self.calculated_base_fee_drops
        }

        fn fee_paid_drops(&self) -> i64 {
            self.fee_paid_drops
        }

        fn default_base_fee_drops(&self) -> i64 {
            self.default_base_fee_drops
        }

        fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot {
            self.metrics_snapshot
        }

        fn open_ledger_tx_count(&self) -> usize {
            self.open_ledger_tx_count
        }

        fn open_ledger_seq(&self) -> u32 {
            self.open_ledger_seq
        }

        fn reserve_drops(&self) -> u64 {
            self.reserve_drops
        }

        fn base_fee_drops(&self) -> u64 {
            self.base_fee_drops
        }
    }

    #[test]
    fn observed_builder_derives_blocker_and_last_valid_from_tx_facts() {
        let rules = Rules::new(std::iter::empty());
        let account = "acct";
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let built = build_queue_apply_top_read_inputs_from_observed(
            QueueApplyObservedTx {
                account: &account,
                transaction_id: "ABC123",
                tx_id: Uint256::from_u64(9),
                tx_seq_proxy: SeqProxy::ticket(9),
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::FAIL_HARD, Some(250)),
                flags: ApplyFlags::FAIL_HARD,
                consequences: TxConsequences::with_category(
                    1,
                    SeqProxy::ticket(9),
                    TxConsequencesCategory::Blocker,
                ),
            },
            QueueApplyObservedView {
                rules: &rules,
                ledger_read: QueueApplyLedgerReadState {
                    account: QueueApplyLedgerAccountState::Present {
                        seq_proxy: SeqProxy::sequence(8),
                        balance_drops: 1_000,
                    },
                    ticket: QueueApplyLedgerTicketState::Missing,
                },
                calculated_base_fee_drops: 10,
                fee_paid_drops: 20,
                default_base_fee_drops: 10,
                metrics_snapshot: QueueFeeMetricsSnapshot {
                    txns_expected: 32,
                    escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
                },
                open_ledger_tx_count: 4,
                open_ledger_seq: 100,
                reserve_drops: 200,
                base_fee_drops: 10,
            },
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: true,
                can_be_held_result: Ter::TER_PRE_SEQ,
                order: &order,
            },
        );

        assert_eq!(built.transaction_id, "ABC123");
        assert_eq!(built.tx_id, Uint256::from_u64(9));
        assert_eq!(built.tx_seq_proxy, SeqProxy::ticket(9));
        assert_eq!(built.last_valid, Some(250));
        assert!(built.is_blocker);
        assert_eq!(built.flags, ApplyFlags::FAIL_HARD);
        assert!(built.queue_is_full);
        assert_eq!(built.can_be_held_result, Ter::TER_PRE_SEQ);
    }

    #[test]
    fn observed_tx_inputs_builder_extracts_fields_from_source() {
        let account = String::from("acct");
        let source = TestObservedTxSource {
            account: &account,
            transaction_id: "ABC123",
            tx_id: Uint256::from_u64(9),
            tx_seq_proxy: SeqProxy::ticket(9),
        };

        let built = build_queue_apply_observed_tx_inputs_from_source(&source);

        assert_eq!(built.account, &account);
        assert_eq!(built.transaction_id, "ABC123");
        assert_eq!(built.tx_id, Uint256::from_u64(9));
        assert_eq!(built.tx_seq_proxy, SeqProxy::ticket(9));
    }

    #[test]
    fn observed_tx_builder_captures_tx_fields_without_reasking_callers() {
        let account = "acct";

        let built = build_queue_apply_observed_tx(
            QueueApplyObservedTxInputs {
                account: &account,
                transaction_id: "ABC123",
                tx_id: Uint256::from_u64(9),
                tx_seq_proxy: SeqProxy::ticket(9),
            },
            QueueHoldPreflight::new(false, false, ApplyFlags::FAIL_HARD, Some(250)),
            ApplyFlags::FAIL_HARD,
            TxConsequences::with_category(1, SeqProxy::ticket(9), TxConsequencesCategory::Blocker),
        );

        assert_eq!(built.account, &account);
        assert_eq!(built.transaction_id, "ABC123");
        assert_eq!(built.tx_id, Uint256::from_u64(9));
        assert_eq!(built.tx_seq_proxy, SeqProxy::ticket(9));
        assert_eq!(built.preflight.last_valid_ledger, Some(250));
        assert_eq!(built.flags, ApplyFlags::FAIL_HARD);
        assert!(built.consequences.is_blocker());
    }

    #[test]
    fn observed_view_inputs_builder_reads_lookup_and_fee_facts_from_source() {
        let account = String::from("acct");
        let source = TestObservedViewSource {
            rules: Rules::new(std::iter::empty()),
            account_lookup: QueueApplyObservedAccountLookup::Present {
                sequence: 8,
                balance_drops: 1_000,
            },
            ticket_lookup: QueueApplyObservedTicketLookup::Missing,
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 4,
            open_ledger_seq: 100,
            reserve_drops: 200,
            base_fee_drops: 10,
        };

        let built = build_queue_apply_observed_view_inputs_from_source(
            &source,
            &account,
            SeqProxy::ticket(9),
        );

        assert!(std::ptr::eq(built.rules, &source.rules));
        assert_eq!(
            built.account_lookup,
            QueueApplyObservedAccountLookup::Present {
                sequence: 8,
                balance_drops: 1_000,
            }
        );
        assert_eq!(built.ticket_lookup, QueueApplyObservedTicketLookup::Missing);
        assert_eq!(built.calculated_base_fee_drops, 10);
        assert_eq!(built.open_ledger_seq, 100);
    }

    #[test]
    fn observed_view_builder_maps_account_and_ticket_lookup_results() {
        let rules = Rules::new(std::iter::empty());
        let built = build_queue_apply_observed_view(QueueApplyObservedViewInputs {
            rules: &rules,
            account_lookup: QueueApplyObservedAccountLookup::Present {
                sequence: 8,
                balance_drops: 1_000,
            },
            ticket_lookup: QueueApplyObservedTicketLookup::Missing,
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 4,
            open_ledger_seq: 100,
            reserve_drops: 200,
            base_fee_drops: 10,
        });

        assert_eq!(
            built.ledger_read,
            QueueApplyLedgerReadState {
                account: QueueApplyLedgerAccountState::Present {
                    seq_proxy: SeqProxy::sequence(8),
                    balance_drops: 1_000,
                },
                ticket: QueueApplyLedgerTicketState::Missing,
            }
        );
        assert_eq!(built.calculated_base_fee_drops, 10);
        assert_eq!(built.open_ledger_seq, 100);
        assert_eq!(built.reserve_drops, 200);
    }

    #[test]
    fn observed_builder_passes_view_side_fee_inputs_through() {
        let rules = Rules::new(std::iter::empty());
        let account = "acct";
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let snapshot = QueueFeeMetricsSnapshot {
            txns_expected: 64,
            escalation_multiplier: crate::TXQ_BASE_LEVEL * 400,
        };

        let built = build_queue_apply_top_read_inputs_from_observed(
            QueueApplyObservedTx {
                account: &account,
                transaction_id: "ABC123",
                tx_id: Uint256::from_u64(9),
                tx_seq_proxy: SeqProxy::sequence(8),
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                flags: ApplyFlags::NONE,
                consequences: TxConsequences::new(1, SeqProxy::sequence(8)),
            },
            QueueApplyObservedView {
                rules: &rules,
                ledger_read: QueueApplyLedgerReadState {
                    account: QueueApplyLedgerAccountState::Missing,
                    ticket: QueueApplyLedgerTicketState::NotRequired,
                },
                calculated_base_fee_drops: 12,
                fee_paid_drops: 34,
                default_base_fee_drops: 56,
                metrics_snapshot: snapshot,
                open_ledger_tx_count: 7,
                open_ledger_seq: 200,
                reserve_drops: 300,
                base_fee_drops: 11,
            },
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 4,
                maximum_txn_per_account: 20,
                retry_sequence_percent: 50,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
                order: &order,
            },
        );

        assert_eq!(built.calculated_base_fee_drops, 12);
        assert_eq!(built.fee_paid_drops, 34);
        assert_eq!(built.default_base_fee_drops, 56);
        assert_eq!(built.metrics_snapshot, snapshot);
        assert_eq!(built.open_ledger_tx_count, 7);
        assert_eq!(built.open_ledger_seq, 200);
        assert_eq!(built.reserve_drops, 300);
        assert_eq!(built.base_fee_drops, 11);
        assert_eq!(built.minimum_last_ledger_buffer, 4);
        assert_eq!(built.maximum_txn_per_account, 20);
        assert_eq!(built.retry_sequence_percent, 50);
        assert!(!built.is_blocker);
    }

    #[test]
    fn observed_source_builder_joins_tx_and_view_sources_in_one_step() {
        let account = String::from("acct");
        let tx_source = TestObservedTxSource {
            account: &account,
            transaction_id: "ABC123",
            tx_id: Uint256::from_u64(9),
            tx_seq_proxy: SeqProxy::ticket(9),
        };
        let view_source = TestObservedViewSource {
            rules: Rules::new(std::iter::empty()),
            account_lookup: QueueApplyObservedAccountLookup::Present {
                sequence: 8,
                balance_drops: 1_000,
            },
            ticket_lookup: QueueApplyObservedTicketLookup::Missing,
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 4,
            open_ledger_seq: 100,
            reserve_drops: 200,
            base_fee_drops: 10,
        };
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let built = build_queue_apply_top_read_inputs_from_sources(
            &tx_source,
            &view_source,
            QueueHoldPreflight::new(false, false, ApplyFlags::FAIL_HARD, Some(250)),
            ApplyFlags::FAIL_HARD,
            TxConsequences::with_category(1, SeqProxy::ticket(9), TxConsequencesCategory::Blocker),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: true,
                can_be_held_result: Ter::TER_PRE_SEQ,
                order: &order,
            },
        );

        assert_eq!(built.transaction_id, "ABC123");
        assert_eq!(built.tx_seq_proxy, SeqProxy::ticket(9));
        assert_eq!(
            built.ledger_read,
            QueueApplyLedgerReadState {
                account: QueueApplyLedgerAccountState::Present {
                    seq_proxy: SeqProxy::sequence(8),
                    balance_drops: 1_000,
                },
                ticket: QueueApplyLedgerTicketState::Missing,
            }
        );
        assert_eq!(built.last_valid, Some(250));
        assert!(built.is_blocker);
    }

    #[test]
    fn observed_fact_builder_joins_tx_fields_and_lookup_results_in_one_step() {
        let rules = Rules::new(std::iter::empty());
        let account = "acct";
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let built = build_queue_apply_top_read_inputs_from_observed_facts(
            QueueApplyObservedTxInputs {
                account: &account,
                transaction_id: "ABC123",
                tx_id: Uint256::from_u64(9),
                tx_seq_proxy: SeqProxy::ticket(9),
            },
            QueueHoldPreflight::new(false, false, ApplyFlags::FAIL_HARD, Some(250)),
            ApplyFlags::FAIL_HARD,
            TxConsequences::with_category(1, SeqProxy::ticket(9), TxConsequencesCategory::Blocker),
            QueueApplyObservedViewInputs {
                rules: &rules,
                account_lookup: QueueApplyObservedAccountLookup::Present {
                    sequence: 8,
                    balance_drops: 1_000,
                },
                ticket_lookup: QueueApplyObservedTicketLookup::Missing,
                calculated_base_fee_drops: 10,
                fee_paid_drops: 20,
                default_base_fee_drops: 10,
                metrics_snapshot: QueueFeeMetricsSnapshot {
                    txns_expected: 32,
                    escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
                },
                open_ledger_tx_count: 4,
                open_ledger_seq: 100,
                reserve_drops: 200,
                base_fee_drops: 10,
            },
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: true,
                can_be_held_result: Ter::TER_PRE_SEQ,
                order: &order,
            },
        );

        assert_eq!(built.transaction_id, "ABC123");
        assert_eq!(built.tx_seq_proxy, SeqProxy::ticket(9));
        assert_eq!(
            built.ledger_read,
            QueueApplyLedgerReadState {
                account: QueueApplyLedgerAccountState::Present {
                    seq_proxy: SeqProxy::sequence(8),
                    balance_drops: 1_000,
                },
                ticket: QueueApplyLedgerTicketState::Missing,
            }
        );
        assert_eq!(built.last_valid, Some(250));
        assert!(built.is_blocker);
        assert!(built.queue_is_full);
    }
}
