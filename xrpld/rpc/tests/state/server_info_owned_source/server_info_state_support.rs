//! Tests for server info state support.

use std::sync::Arc;

use perflog::{NullJournal, PerfLogImp, PerfLogReportSource, PerfLogSetup};
use serde_json::Value;

#[derive(Debug, Clone)]

pub struct TestPerfLogReportSource {
    pub nodestore: Value,
    pub state_accounting: Value,
    pub server_state_duration_us: Option<String>,
    pub initial_sync_duration_us: Option<String>,
}

impl PerfLogReportSource for TestPerfLogReportSource {
    fn node_store_counts_json(&self) -> Value {
        self.nodestore.clone()
    }

    fn state_accounting(&self, report: &mut Value) {
        let Value::Object(report) = report else {
            return;
        };

        report.insert("state_accounting".to_owned(), self.state_accounting.clone());
        if let Some(duration) = &self.server_state_duration_us {
            report.insert(
                "server_state_duration_us".to_owned(),
                Value::String(duration.clone()),
            );
        }
        if let Some(duration) = &self.initial_sync_duration_us {
            report.insert(
                "initial_sync_duration_us".to_owned(),
                Value::String(duration.clone()),
            );
        }
    }
}

pub fn make_test_perf_log(
    rpc_methods: &[&str],
    report_source: TestPerfLogReportSource,
) -> Arc<PerfLogImp> {
    Arc::new(PerfLogImp::new_with_hostname(
        PerfLogSetup::default(),
        rpc_methods
            .iter()
            .map(|method| (*method).to_owned())
            .collect(),
        Arc::new(report_source),
        Arc::new(NullJournal),
        Arc::new(|| {}),
        "rpc-test",
    ))
}
