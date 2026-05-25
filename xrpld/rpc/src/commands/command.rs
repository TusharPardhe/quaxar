//! Small RPC dispatch seam for inbound server integration.

use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::commands::session::InfoSub;
use crate::fee::do_fee;
use crate::ledger_closed::do_ledger_closed;
use crate::ledger_current::do_ledger_current;
use crate::ping::do_ping;
use crate::state::context::{JsonContext, RpcRequestContext};
use crate::state::role::{Role, RpcAccessConfig};
use crate::state::server_definitions::do_server_definitions;
use crate::state::server_info::{ServerInfoSource, do_server_info};
use crate::state::server_state::do_server_state;
use crate::status::{RpcErrorCode, Status};
use crate::subscriptions::subscription::{SubscriptionManager, parse_streams};
use crate::{
    FeeSource, JsonContextHeaders, LedgerClosedSource, LedgerCurrentSource, rpc_helpers::rpc_error,
};

#[derive(Debug)]
pub struct RpcCommandContext<'a, Env> {
    pub method: &'a str,
    pub params: &'a JsonValue,
    pub env: &'a Env,
    pub role: Role,
    pub api_version: u32,
    pub headers: JsonContextHeaders<'a>,
    pub unlimited: bool,
    pub session: &'a mut InfoSub,
    pub subscriptions: &'a SubscriptionManager,
    pub access: &'a RpcAccessConfig,
    pub remote_ip: std::net::IpAddr,
}

impl<'a, Env> RpcCommandContext<'a, Env> {
    pub fn from_request<Runtime>(
        request: &'a RpcRequestContext<'a, Env, Runtime>,
        method: &'a str,
        session: &'a mut InfoSub,
        subscriptions: &'a SubscriptionManager,
        access: &'a RpcAccessConfig,
    ) -> Result<Self, Status> {
        Ok(Self {
            method,
            params: request.params,
            env: request.env,
            role: request.role,
            api_version: request.api_version,
            headers: request.headers,
            unlimited: request.unlimited,
            session,
            subscriptions,
            access,
            remote_ip: request.remote_ip_or_internal()?,
        })
    }
}

fn json_context<'a, Env>(context: &'a RpcCommandContext<'a, Env>) -> JsonContext<'a, Env> {
    JsonContext {
        params: context.params,
        env: context.env,
        role: context.role,
        api_version: context.api_version,
        headers: context.headers,
        unlimited: context.unlimited,
    }
}

fn object() -> JsonValue {
    JsonValue::Object(BTreeMap::new())
}

fn status_json(status: Status) -> JsonValue {
    let mut value = object();
    status.inject(&mut value);
    value
}

fn do_subscribe<Env>(context: &mut RpcCommandContext<'_, Env>) -> JsonValue {
    let JsonValue::Object(params) = context.params else {
        return rpc_error(RpcErrorCode::InvalidParams);
    };

    context.session.set_api_version(context.api_version);

    let Some(streams) = params.get("streams") else {
        return object();
    };

    let streams = match parse_streams(streams) {
        Ok(streams) => streams,
        Err(status) => return status_json(status),
    };

    for stream in streams {
        if context
            .subscriptions
            .subscribe(context.session, stream)
            .is_err()
        {
            return rpc_error(RpcErrorCode::StreamMalformed);
        }
    }

    object()
}

fn do_unsubscribe<Env>(context: &mut RpcCommandContext<'_, Env>) -> JsonValue {
    let JsonValue::Object(params) = context.params else {
        return rpc_error(RpcErrorCode::InvalidParams);
    };

    let Some(streams) = params.get("streams") else {
        return object();
    };

    let streams = match parse_streams(streams) {
        Ok(streams) => streams,
        Err(status) => return status_json(status),
    };

    for stream in streams {
        context.subscriptions.unsubscribe(context.session, stream);
    }

    object()
}

pub fn do_command<Env>(context: &mut RpcCommandContext<'_, Env>) -> JsonValue
where
    Env: FeeSource + LedgerClosedSource + LedgerCurrentSource + ServerInfoSource,
{
    match context.method {
        "fee" => do_fee(context.env),
        "ledger_closed" => do_ledger_closed(context.env),
        "ledger_current" => do_ledger_current(context.env),
        "ping" => {
            let json_context = json_context(context);
            do_ping(&json_context)
        }
        "server_definitions" => do_server_definitions(context.params),
        "server_info" => {
            let json_context = json_context(context);
            do_server_info(&json_context)
        }
        "server_state" => {
            let json_context = json_context(context);
            do_server_state(&json_context)
        }
        "subscribe" => do_subscribe(context),
        "unsubscribe" => do_unsubscribe(context),
        _ => rpc_error(RpcErrorCode::UnknownCommand),
    }
}

pub fn empty_response() -> JsonValue {
    object()
}

pub fn validate_subscribe_params(params: &JsonValue) -> Result<(), Status> {
    let JsonValue::Object(object) = params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if let Some(streams) = object.get("streams") {
        parse_streams(streams)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{RpcCommandContext, do_command};
    use crate::state::context::JsonContextHeaders;
    use crate::{
        FeeSource, LedgerClosed, LedgerClosedSource, LedgerCurrentSource, ServerInfoSource,
    };
    use protocol::JsonValue;
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr};

    #[derive(Debug, Default)]
    struct FakeEnv;

    impl FeeSource for FakeEnv {
        fn fee_json(&self) -> JsonValue {
            JsonValue::Null
        }
    }

    impl LedgerClosedSource for FakeEnv {
        fn closed_ledger(&self) -> Option<LedgerClosed> {
            None
        }
    }

    impl LedgerCurrentSource for FakeEnv {
        fn current_ledger_index(&self) -> u32 {
            0
        }
    }

    impl ServerInfoSource for FakeEnv {
        fn get_server_info(&self, _human: bool, _admin: bool, _counters: bool) -> JsonValue {
            JsonValue::Object(BTreeMap::from([(
                "supported".to_owned(),
                JsonValue::Bool(true),
            )]))
        }
    }

    #[test]
    fn do_command_routes_server_definitions() {
        let env = FakeEnv;
        let params = JsonValue::Object(BTreeMap::new());
        let mut session = crate::commands::session::InfoSub::new(crate::Role::User);
        let subscriptions = crate::SubscriptionManager::default();
        let access = crate::RpcAccessConfig::default();
        let mut context = RpcCommandContext {
            method: "server_definitions",
            params: &params,
            env: &env,
            role: crate::Role::User,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            unlimited: false,
            session: &mut session,
            subscriptions: &subscriptions,
            access: &access,
            remote_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        };

        let response = do_command(&mut context);
        let JsonValue::Object(object) = response else {
            panic!("server_definitions response must be an object");
        };

        assert!(object.contains_key("TYPES"));
    }
}
