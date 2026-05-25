//! Read-only `ping` RPC slice.
//!
//! This keeps the the reference implementation `doPing(...)` response shaping on an explicit
//! caller-owned context seam instead of a hidden websocket/runtime owner.

use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::{JsonContext, RpcRole};

pub fn do_ping<Env>(context: &JsonContext<'_, Env>) -> JsonValue {
    tracing::trace!(target: "rpc", method = "ping", "ping query");
    let mut result = BTreeMap::new();

    match context.role {
        RpcRole::Admin => {
            result.insert("role".to_owned(), JsonValue::String("admin".to_owned()));
        }
        RpcRole::Identified => {
            result.insert(
                "role".to_owned(),
                JsonValue::String("identified".to_owned()),
            );
            result.insert(
                "username".to_owned(),
                JsonValue::String(context.headers.user.to_owned()),
            );
            if !context.headers.forwarded_for.is_empty() {
                result.insert(
                    "ip".to_owned(),
                    JsonValue::String(context.headers.forwarded_for.to_owned()),
                );
            }
        }
        RpcRole::Proxy => {
            result.insert("role".to_owned(), JsonValue::String("proxied".to_owned()));
            result.insert(
                "ip".to_owned(),
                JsonValue::String(context.headers.forwarded_for.to_owned()),
            );
        }
        _ => {}
    }

    if context.unlimited {
        result.insert("unlimited".to_owned(), JsonValue::Bool(true));
    }

    JsonValue::Object(result)
}
