//! Tests for the tx reduce relay RPC handler.

use protocol::JsonValue;
use rpc::tx_reduce_relay::{TxReduceRelaySource, do_tx_reduce_relay};
use std::collections::BTreeMap;

struct RecordingSource {
    result: JsonValue,
}

impl TxReduceRelaySource for RecordingSource {
    fn tx_metrics_json(&self) -> JsonValue {
        self.result.clone()
    }
}

#[test]
fn tx_reduce_relay_returns_overlay_metrics_unchanged() {
    let source = RecordingSource {
        result: JsonValue::Object(BTreeMap::from([
            ("txs".to_owned(), JsonValue::Signed(12)),
            ("drops".to_owned(), JsonValue::Unsigned(34)),
        ])),
    };

    let result = do_tx_reduce_relay(&source);

    assert_eq!(
        result,
        JsonValue::Object(BTreeMap::from([
            ("txs".to_owned(), JsonValue::Signed(12)),
            ("drops".to_owned(), JsonValue::Unsigned(34)),
        ]))
    );
}

#[test]
fn tx_reduce_relay_keeps_non_object_results_unchanged() {
    let source = RecordingSource {
        result: JsonValue::String("not an object".to_owned()),
    };

    let result = do_tx_reduce_relay(&source);

    assert_eq!(result, JsonValue::String("not an object".to_owned()));
}
