use app::{ApplicationRoot, StatusMetricsSource};
use perflog::{PerfLogImp, PerfLogJournal, PerfLogReportSource, PerfLogSetup};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Default)]
struct FixedReportSource;

impl PerfLogReportSource for FixedReportSource {
    fn node_store_counts_json(&self) -> Value {
        json!({"entries": 5})
    }

    fn state_accounting(&self, report: &mut Value) {
        let Some(object) = report.as_object_mut() else {
            panic!("report must be an object");
        };
        object.insert(
            "state_accounting".to_owned(),
            json!({"full": {"transitions": "2", "duration_us": "50"}}),
        );
        object.insert(
            "server_state_duration_us".to_owned(),
            Value::String("60".to_owned()),
        );
        object.insert(
            "initial_sync_duration_us".to_owned(),
            Value::String("70".to_owned()),
        );
    }
}

#[test]
fn application_root_defaults_status_metrics_to_its_owned_perf_log() {
    let root = ApplicationRoot::new(0).expect("root shell should build");

    assert!(root.status_metrics().is_some());
    assert_eq!(
        root.status_metrics().expect("metrics").counters_json(),
        StatusMetricsSource::counters_json(root.perf_log().as_ref())
    );
}

struct NullJournal;

impl PerfLogJournal for NullJournal {
    fn log(&self, _level: perflog::JournalLevel, _message: &str) {}
}

#[test]
fn perflog_status_metrics_expose_state_accounting_fields_for_server_status() {
    let log = PerfLogImp::new_with_hostname(
        PerfLogSetup::default(),
        vec!["server_info".to_owned()],
        Arc::new(FixedReportSource),
        Arc::new(NullJournal),
        Arc::new(|| {}),
        "host",
    );

    assert_eq!(
        log.state_accounting_json(),
        json!({"full": {"transitions": "2", "duration_us": "50"}})
    );
    assert_eq!(log.server_state_duration_us(), Some("60".to_owned()));
    assert_eq!(log.initial_sync_duration_us(), Some("70".to_owned()));
}
