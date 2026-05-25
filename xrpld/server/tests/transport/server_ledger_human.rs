use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use app::ApplicationRoot;
use ledger::{Fees, Ledger, LedgerHeader};
use protocol::JsonValue;
use server::{BuiltinDispatcher, RequestMetadata, RpcDispatcher, RpcRequest, SubscriptionManager};

fn metadata(role: rpc::RpcRole) -> RequestMetadata {
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let mut metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    metadata.role = role;
    metadata.unlimited = matches!(role, rpc::RpcRole::Admin);
    metadata
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
fn dispatcher_surfaces_human_validated_ledger_age() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let now_close_time = app.current_close_time_seconds();
    app.on_validated_ledger(sample_ledger(101, now_close_time.saturating_sub(30), 0x22));
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(42));

    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let reply = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata(rpc::RpcRole::User),
        session: None,
    });
    let server::RpcReply::Result(reply) = reply else {
        panic!("reply must be a result");
    };

    let json = server::from_protocol_json(&reply);
    assert_eq!(
        json["info"]["validated_ledger"]["age"],
        serde_json::json!(42)
    );
}
