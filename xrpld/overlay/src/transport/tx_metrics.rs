//! Tx reduce-relay metrics aligned with `detail/TxMetrics.*`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::JsonValue;

use crate::message::ProtocolMessageType;
use crate::slot::{Clock, SystemClock};

#[derive(Debug)]
pub struct SingleMetrics {
    clock: Arc<dyn Clock>,
    interval_start: Duration,
    accum: u64,
    pub rolling_avg: u64,
    n: u32,
    per_time_unit: bool,
    rolling_avg_aggregate: VecDeque<u64>,
}

impl SingleMetrics {
    pub fn new(clock: Arc<dyn Clock>, per_time_unit: bool) -> Self {
        Self {
            interval_start: clock.now(),
            clock,
            accum: 0,
            rolling_avg: 0,
            n: 0,
            per_time_unit,
            rolling_avg_aggregate: VecDeque::from(vec![0; 30]),
        }
    }

    pub fn add_metrics(&mut self, value: u32) {
        self.accum += u64::from(value);
        self.n = self.n.saturating_add(1);

        let elapsed = self.clock.now().saturating_sub(self.interval_start);
        let elapsed_secs = elapsed.as_secs();
        if elapsed_secs == 0 {
            return;
        }

        let divisor = if self.per_time_unit {
            elapsed_secs.max(1)
        } else {
            u64::from(self.n).max(1)
        };
        let avg = self.accum / divisor;
        if self.rolling_avg_aggregate.len() == 30 {
            self.rolling_avg_aggregate.pop_front();
        }
        self.rolling_avg_aggregate.push_back(avg);
        self.rolling_avg = self.rolling_avg_aggregate.iter().sum::<u64>()
            / self.rolling_avg_aggregate.len() as u64;
        self.interval_start = self.clock.now();
        self.accum = 0;
        self.n = 0;
    }
}

#[derive(Debug)]
pub struct MultipleMetrics {
    pub m1: SingleMetrics,
    pub m2: SingleMetrics,
}

impl MultipleMetrics {
    pub fn new(clock: Arc<dyn Clock>, per_time_unit_1: bool, per_time_unit_2: bool) -> Self {
        Self {
            m1: SingleMetrics::new(Arc::clone(&clock), per_time_unit_1),
            m2: SingleMetrics::new(clock, per_time_unit_2),
        }
    }

    pub fn add_metrics(&mut self, val2: u32) {
        self.add_pair_metrics(1, val2);
    }

    pub fn add_pair_metrics(&mut self, val1: u32, val2: u32) {
        self.m1.add_metrics(val1);
        self.m2.add_metrics(val2);
    }
}

#[derive(Debug)]
struct TxMetricsState {
    tx: MultipleMetrics,
    have_tx: MultipleMetrics,
    get_ledger: MultipleMetrics,
    ledger_data: MultipleMetrics,
    transactions: MultipleMetrics,
    selected_peers: SingleMetrics,
    suppressed_peers: SingleMetrics,
    not_enabled: SingleMetrics,
    missing_tx: SingleMetrics,
}

#[derive(Debug)]
pub struct TxMetrics {
    inner: Mutex<TxMetricsState>,
}

impl Default for TxMetrics {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock))
    }
}

impl TxMetrics {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Mutex::new(TxMetricsState {
                tx: MultipleMetrics::new(Arc::clone(&clock), true, true),
                have_tx: MultipleMetrics::new(Arc::clone(&clock), true, true),
                get_ledger: MultipleMetrics::new(Arc::clone(&clock), true, true),
                ledger_data: MultipleMetrics::new(Arc::clone(&clock), true, true),
                transactions: MultipleMetrics::new(Arc::clone(&clock), true, true),
                selected_peers: SingleMetrics::new(Arc::clone(&clock), false),
                suppressed_peers: SingleMetrics::new(Arc::clone(&clock), false),
                not_enabled: SingleMetrics::new(Arc::clone(&clock), false),
                missing_tx: SingleMetrics::new(clock, true),
            }),
        }
    }

    pub fn add_message_metrics(&self, message_type: ProtocolMessageType, size: u32) {
        let mut inner = self.inner.lock().expect("tx metrics lock");
        match message_type {
            ProtocolMessageType::MtTransaction => inner.tx.add_metrics(size),
            ProtocolMessageType::MtHaveTransactions => inner.have_tx.add_metrics(size),
            ProtocolMessageType::MtGetLedger => inner.get_ledger.add_metrics(size),
            ProtocolMessageType::MtLedgerData => inner.ledger_data.add_metrics(size),
            ProtocolMessageType::MtTransactions => inner.transactions.add_metrics(size),
            _ => {}
        }
    }

    pub fn add_relay_selection_metrics(&self, selected: u32, suppressed: u32, not_enabled: u32) {
        let mut inner = self.inner.lock().expect("tx metrics lock");
        inner.selected_peers.add_metrics(selected);
        inner.suppressed_peers.add_metrics(suppressed);
        inner.not_enabled.add_metrics(not_enabled);
    }

    pub fn add_missing_metrics(&self, missing: u32) {
        self.inner
            .lock()
            .expect("tx metrics lock")
            .missing_tx
            .add_metrics(missing);
    }

    pub fn json(&self) -> JsonValue {
        let inner = self.inner.lock().expect("tx metrics lock");
        JsonValue::Object(BTreeMap::from([
            (
                "txr_tx_cnt".to_owned(),
                JsonValue::String(inner.tx.m1.rolling_avg.to_string()),
            ),
            (
                "txr_tx_sz".to_owned(),
                JsonValue::String(inner.tx.m2.rolling_avg.to_string()),
            ),
            (
                "txr_have_txs_cnt".to_owned(),
                JsonValue::String(inner.have_tx.m1.rolling_avg.to_string()),
            ),
            (
                "txr_have_txs_sz".to_owned(),
                JsonValue::String(inner.have_tx.m2.rolling_avg.to_string()),
            ),
            (
                "txr_get_ledger_cnt".to_owned(),
                JsonValue::String(inner.get_ledger.m1.rolling_avg.to_string()),
            ),
            (
                "txr_get_ledger_sz".to_owned(),
                JsonValue::String(inner.get_ledger.m2.rolling_avg.to_string()),
            ),
            (
                "txr_ledger_data_cnt".to_owned(),
                JsonValue::String(inner.ledger_data.m1.rolling_avg.to_string()),
            ),
            (
                "txr_ledger_data_sz".to_owned(),
                JsonValue::String(inner.ledger_data.m2.rolling_avg.to_string()),
            ),
            (
                "txr_transactions_cnt".to_owned(),
                JsonValue::String(inner.transactions.m1.rolling_avg.to_string()),
            ),
            (
                "txr_transactions_sz".to_owned(),
                JsonValue::String(inner.transactions.m2.rolling_avg.to_string()),
            ),
            (
                "txr_selected_cnt".to_owned(),
                JsonValue::String(inner.selected_peers.rolling_avg.to_string()),
            ),
            (
                "txr_suppressed_cnt".to_owned(),
                JsonValue::String(inner.suppressed_peers.rolling_avg.to_string()),
            ),
            (
                "txr_not_enabled_cnt".to_owned(),
                JsonValue::String(inner.not_enabled.rolling_avg.to_string()),
            ),
            (
                "txr_missing_tx_freq".to_owned(),
                JsonValue::String(inner.missing_tx.rolling_avg.to_string()),
            ),
        ]))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use protocol::JsonValue;

    use super::TxMetrics;
    use crate::ProtocolMessageType;
    use crate::slot::ManualClock;

    #[test]
    fn tx_metrics_roll_into_json_with_cpp_keys() {
        let clock = Arc::new(ManualClock::new(Duration::from_secs(10)));
        let metrics = TxMetrics::new(clock.clone());

        metrics.add_message_metrics(ProtocolMessageType::MtTransaction, 120);
        metrics.add_relay_selection_metrics(4, 2, 1);
        metrics.add_missing_metrics(3);
        clock.advance(Duration::from_secs(1));
        metrics.add_message_metrics(ProtocolMessageType::MtTransaction, 80);
        metrics.add_relay_selection_metrics(6, 1, 0);
        metrics.add_missing_metrics(2);

        let JsonValue::Object(object) = metrics.json() else {
            panic!("expected object");
        };
        assert!(object.contains_key("txr_tx_cnt"));
        assert!(object.contains_key("txr_selected_cnt"));
        assert!(object.contains_key("txr_missing_tx_freq"));
    }
}
