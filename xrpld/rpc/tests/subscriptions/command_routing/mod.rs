//! Tests for RPC command routing (do_command dispatch).

pub(super) use std::collections::BTreeMap;
pub(super) use std::net::{IpAddr, Ipv4Addr};
pub(super) use std::str::FromStr;

pub(super) use ipnet::IpNet;
pub(super) use protocol::JsonValue;
pub(super) use rpc::{
    FeeSource, InfoSub, JsonContextHeaders, LedgerClosed, LedgerClosedSource, LedgerCurrentSource,
    RpcAccessConfig, RpcCommandContext, RpcErrorCode, RpcRole, SubscriptionManager,
    SubscriptionStream, do_command,
};

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Debug)]
pub(super) struct FakeServerInfoSource;

impl rpc::ServerInfoSource for FakeServerInfoSource {
    fn get_server_info(&self, human: bool, admin: bool, counters: bool) -> JsonValue {
        object([
            ("admin", JsonValue::Bool(admin)),
            ("counters", JsonValue::Bool(counters)),
            ("human", JsonValue::Bool(human)),
        ])
    }
}

impl FeeSource for FakeServerInfoSource {
    fn fee_json(&self) -> JsonValue {
        object([("fee", JsonValue::String("ok".to_owned()))])
    }
}

impl LedgerCurrentSource for FakeServerInfoSource {
    fn current_ledger_index(&self) -> u32 {
        991
    }
}

impl LedgerClosedSource for FakeServerInfoSource {
    fn closed_ledger(&self) -> Option<LedgerClosed> {
        Some(LedgerClosed {
            seq: 990,
            hash: basics::base_uint::Uint256::from_u64(0xABCD),
        })
    }
}

pub(super) fn access() -> RpcAccessConfig {
    RpcAccessConfig {
        admin_nets: vec![IpNet::from_str("127.0.0.0/8").expect("admin net")],
        secure_gateway_nets: vec![IpNet::from_str("203.0.113.0/24").expect("gateway net")],
        admin_user: String::new(),
        admin_password: String::new(),
    }
}

mod ping_and_roles;
mod routing_and_subscribe;
