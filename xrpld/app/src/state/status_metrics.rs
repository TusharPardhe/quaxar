//! Narrow perf-log-backed status metrics seam for `NetworkOPs::getServerInfo(...)`.

use perflog::{PerfLog, PerfLogImp};
use serde_json::Value;

pub trait StatusMetricsSource: Send + Sync + 'static {
    fn counters_json(&self) -> Value;
    fn current_activities_json(&self) -> Value;
    fn nodestore_counts_json(&self) -> Value;
    fn state_accounting_json(&self) -> Value;
    fn server_state_duration_us(&self) -> Option<String>;
    fn initial_sync_duration_us(&self) -> Option<String>;
}

impl StatusMetricsSource for PerfLogImp {
    fn counters_json(&self) -> Value {
        PerfLog::counters_json(self)
    }

    fn current_activities_json(&self) -> Value {
        PerfLog::current_json(self)
    }

    fn nodestore_counts_json(&self) -> Value {
        self.snapshot_report()
            .get("nodestore")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()))
    }

    fn state_accounting_json(&self) -> Value {
        self.snapshot_report()
            .get("state_accounting")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()))
    }

    fn server_state_duration_us(&self) -> Option<String> {
        snapshot_string_field(self.snapshot_report(), "server_state_duration_us")
    }

    fn initial_sync_duration_us(&self) -> Option<String> {
        snapshot_string_field(self.snapshot_report(), "initial_sync_duration_us")
    }
}

fn snapshot_string_field(report: Value, key: &str) -> Option<String> {
    match report.get(key) {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::snapshot_string_field;
    use serde_json::json;

    #[test]
    fn snapshot_string_field_reads_string_and_numeric_values() {
        assert_eq!(
            snapshot_string_field(
                json!({"server_state_duration_us": "42"}),
                "server_state_duration_us"
            ),
            Some("42".to_owned())
        );
        assert_eq!(
            snapshot_string_field(
                json!({"server_state_duration_us": 42}),
                "server_state_duration_us"
            ),
            Some("42".to_owned())
        );
        assert_eq!(
            snapshot_string_field(json!({}), "server_state_duration_us"),
            None
        );
    }
}
