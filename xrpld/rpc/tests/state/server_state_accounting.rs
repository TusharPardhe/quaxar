//! Tests for server state accounting.

use app::{ApplicationRoot, StatusMetricsSource};
use protocol::JsonValue;
use rpc::{ApplicationServerInfo, JsonContext, JsonContextHeaders, RpcRole, do_server_state};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct FixedStatusMetricsSource;

impl StatusMetricsSource for FixedStatusMetricsSource {
    fn counters_json(&self) -> serde_json::Value {
        json!({})
    }

    fn current_activities_json(&self) -> serde_json::Value {
        json!({})
    }

    fn nodestore_counts_json(&self) -> serde_json::Value {
        json!({})
    }

    fn state_accounting_json(&self) -> serde_json::Value {
        json!({"tracking": {"transitions": "4", "duration_us": "40"}})
    }

    fn server_state_duration_us(&self) -> Option<String> {
        Some("60".to_owned())
    }

    fn initial_sync_duration_us(&self) -> Option<String> {
        Some("70".to_owned())
    }
}

fn context<'a, Env>(params: &'a JsonValue, env: &'a Env) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role: RpcRole::User,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    }
}

#[test]
fn server_state_emits_state_accounting_without_requiring_counters() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.attach_status_metrics(Arc::new(FixedStatusMetricsSource));
    let params = JsonValue::Object(BTreeMap::new());

    let result = do_server_state(&context(&params, &ApplicationServerInfo::new(&app)));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(state) = result.get("state").expect("state must exist") else {
        panic!("state must be an object");
    };
    let JsonValue::Object(state_accounting) = state
        .get("state_accounting")
        .expect("state accounting must exist")
    else {
        panic!("state accounting must be an object");
    };
    let JsonValue::Object(tracking) = state_accounting
        .get("tracking")
        .expect("tracking must exist")
    else {
        panic!("tracking must be an object");
    };

    assert_eq!(
        tracking.get("duration_us"),
        Some(&JsonValue::String("40".to_owned()))
    );
    assert_eq!(
        state.get("server_state_duration_us"),
        Some(&JsonValue::String("60".to_owned()))
    );
    assert_eq!(
        state.get("initial_sync_duration_us"),
        Some(&JsonValue::String("70".to_owned()))
    );
}
