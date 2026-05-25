//! Tests for server ledger human.

use std::collections::BTreeMap;
use std::sync::Arc;

use app::ApplicationRoot;
use ledger::{Fees, Ledger, LedgerHeader};
use protocol::JsonValue;
use rpc::{ApplicationServerInfo, JsonContext, JsonContextHeaders, RpcRole, do_server_info};

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

fn sample_ledger(seq: u32, close_time: u32, hash_byte: u8) -> Arc<Ledger> {
    let mut ledger = Ledger::from_ledger_seq_and_close_time(seq, close_time, false);
    ledger.set_ledger_info(LedgerHeader {
        hash: basics::sha_map_hash::SHAMapHash::new(basics::base_uint::Uint256::from_array(
            [hash_byte; 32],
        )),
        ..ledger.header()
    });
    ledger.set_fees(Fees {
        base: 10,
        reserve: 2_000_000,
        increment: 200_000,
    });
    Arc::new(ledger)
}

#[test]
fn server_info_human_validated_ledger_zeroes_high_age_and_emits_close_offset() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    assert_eq!(
        app.time_keeper()
            .adjust_close_time(time::Duration::seconds(240)),
        time::Duration::seconds(60)
    );
    let now_close_time = app.current_close_time_seconds();
    app.on_validated_ledger(sample_ledger(101, now_close_time.saturating_sub(30), 0x22));
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(1_000_000));

    let result = do_server_info(&context(
        &JsonValue::Object(BTreeMap::new()),
        &ApplicationServerInfo::new(&app),
    ));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    let JsonValue::Object(validated_ledger) = info
        .get("validated_ledger")
        .expect("validated ledger must exist")
    else {
        panic!("validated ledger must be an object");
    };

    assert_eq!(validated_ledger.get("age"), Some(&JsonValue::Unsigned(0)));
    assert_eq!(
        validated_ledger.get("close_time_offset"),
        Some(&JsonValue::Unsigned(60))
    );
}

#[test]
fn server_info_human_closed_ledger_uses_current_close_time_age() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let now_close_time = app.current_close_time_seconds();
    app.on_closed_ledger(sample_ledger(100, now_close_time.saturating_sub(15), 0x11));

    let result = do_server_info(&context(
        &JsonValue::Object(BTreeMap::new()),
        &ApplicationServerInfo::new(&app),
    ));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    let JsonValue::Object(closed_ledger) = info.get("closed_ledger").expect("closed ledger") else {
        panic!("closed ledger must be an object");
    };

    assert_eq!(closed_ledger.get("age"), Some(&JsonValue::Unsigned(15)));
}
