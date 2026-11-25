//! Tests for the fee RPC handler.

use app::{ApplicationRoot, NetworkOpsOperatingMode, ServiceRegistry};
use protocol::JsonValue;
use rpc::{FeeSource, do_fee};
use std::sync::Arc;
use tx::{QueueTxQRpcDrops, QueueTxQRpcLevels, QueueTxQRpcReport};

fn sample_open_tx() -> Arc<protocol::STTx> {
    Arc::new(protocol::STTx::new(protocol::TxType::OFFER_CREATE, |_| {}))
}

#[derive(Debug, Clone)]
struct FakeFee {
    result: JsonValue,
    synced: bool,
}

impl FeeSource for FakeFee {
    fn fee_json(&self) -> JsonValue {
        self.result.clone()
    }

    fn network_synced(&self) -> bool {
        self.synced
    }
}

#[test]
fn fee_returns_object() {
    let result = do_fee(&FakeFee {
        result: JsonValue::Object(Default::default()),
        synced: true,
    });

    assert!(matches!(result, JsonValue::Object(_)));
}

#[test]
fn fee_maps_non_object_results_to_internal_guard() {
    let JsonValue::Object(result) = do_fee(&FakeFee {
        result: JsonValue::String("not an object".to_owned()),
        synced: true,
    }) else {
        panic!("fee response must be object");
    };

    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("internal".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(73)));
}

#[test]
fn fee_requires_network_sync() {
    let JsonValue::Object(result) = do_fee(&FakeFee {
        result: JsonValue::Object(Default::default()),
        synced: false,
    }) else {
        panic!("fee response must be object");
    };

    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("noNetwork".to_owned()))
    );
    assert_eq!(result.get("error_code"), Some(&JsonValue::Signed(17)));
}

#[test]
fn fee_reads_the_live_queue_not_the_last_published_report() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    app.set_status_rpc_queue_report(Some(QueueTxQRpcReport {
        ledger_current_index: 777,
        expected_ledger_size: "32".to_owned(),
        current_ledger_size: "31".to_owned(),
        current_queue_size: "4".to_owned(),
        max_queue_size: Some("200".to_owned()),
        levels: QueueTxQRpcLevels {
            reference_level: "256".to_owned(),
            minimum_level: "300".to_owned(),
            median_level: "400".to_owned(),
            open_ledger_level: "500".to_owned(),
        },
        drops: QueueTxQRpcDrops {
            base_fee: "10".to_owned(),
            median_fee: "16".to_owned(),
            minimum_fee: "12".to_owned(),
            open_ledger_fee: "20".to_owned(),
        },
    }));

    let changed = ServiceRegistry::get_open_ledger(&app).modify(|next| {
        next.ledger_current_index = 778;
        next.push_transaction(sample_open_tx());
        true
    });
    assert!(changed);

    let JsonValue::Object(result) = do_fee(&rpc::ApplicationServerInfo::new(&app)) else {
        panic!("fee response must be object");
    };

    // rippled doFee -> TxQ::doRPC reads the open ledger and queue now. The
    // snapshot published at the last close (777, 31 transactions) is stale
    // once the open ledger moves on.
    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(778))
    );
    assert_eq!(
        result.get("current_ledger_size"),
        Some(&JsonValue::String("1".to_owned()))
    );
}

#[test]
fn fee_falls_back_to_live_app_txq_report_when_status_snapshot_is_missing() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    assert_eq!(app.status_rpc_queue_report(), None);

    let changed = ServiceRegistry::get_open_ledger(&app).modify(|next| {
        next.ledger_current_index = 712;
        next.base_fee_drops = 17;
        next.push_transaction(sample_open_tx());
        next.push_transaction(sample_open_tx());
        true
    });
    assert!(changed);

    let JsonValue::Object(result) = do_fee(&rpc::ApplicationServerInfo::new(&app)) else {
        panic!("fee response must be object");
    };

    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(712))
    );
    assert_eq!(
        result.get("current_ledger_size"),
        Some(&JsonValue::String("2".to_owned()))
    );
    assert_eq!(
        result.get("current_queue_size"),
        Some(&JsonValue::String("0".to_owned()))
    );
    let JsonValue::Object(drops) = result.get("drops").expect("drops must exist") else {
        panic!("drops must be object");
    };
    assert_eq!(
        drops.get("base_fee"),
        Some(&JsonValue::String("17".to_owned()))
    );
}

#[test]
fn fee_app_report_includes_all_expected_fields() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    let live = app.tx_q_rpc_report();

    let JsonValue::Object(result) = do_fee(&rpc::ApplicationServerInfo::new(&app)) else {
        panic!("fee response must be object");
    };

    // Verify all top-level fields
    assert_eq!(
        result.get("ledger_current_index"),
        Some(&JsonValue::Unsigned(u64::from(live.ledger_current_index)))
    );
    assert_eq!(
        result.get("expected_ledger_size"),
        Some(&JsonValue::String(live.expected_ledger_size.clone()))
    );
    assert_eq!(
        result.get("current_ledger_size"),
        Some(&JsonValue::String(live.current_ledger_size.clone()))
    );
    assert_eq!(
        result.get("current_queue_size"),
        Some(&JsonValue::String(live.current_queue_size.clone()))
    );
    assert_eq!(
        result.get("max_queue_size"),
        live.max_queue_size.clone().map(JsonValue::String).as_ref()
    );

    // Verify levels sub-object
    let JsonValue::Object(levels) = result.get("levels").expect("levels must exist") else {
        panic!("levels must be object");
    };
    assert_eq!(
        levels.get("reference_level"),
        Some(&JsonValue::String(live.levels.reference_level.clone()))
    );
    assert_eq!(
        levels.get("minimum_level"),
        Some(&JsonValue::String(live.levels.minimum_level.clone()))
    );
    assert_eq!(
        levels.get("median_level"),
        Some(&JsonValue::String(live.levels.median_level.clone()))
    );
    assert_eq!(
        levels.get("open_ledger_level"),
        Some(&JsonValue::String(live.levels.open_ledger_level.clone()))
    );

    // Verify drops sub-object
    let JsonValue::Object(drops) = result.get("drops").expect("drops must exist") else {
        panic!("drops must be object");
    };
    assert_eq!(
        drops.get("base_fee"),
        Some(&JsonValue::String(live.drops.base_fee.clone()))
    );
    assert_eq!(
        drops.get("median_fee"),
        Some(&JsonValue::String(live.drops.median_fee.clone()))
    );
    assert_eq!(
        drops.get("minimum_fee"),
        Some(&JsonValue::String(live.drops.minimum_fee.clone()))
    );
    assert_eq!(
        drops.get("open_ledger_fee"),
        Some(&JsonValue::String(live.drops.open_ledger_fee.clone()))
    );
}
