//! `ledger_request` handler port from `xrpld/rpc/handlers/admin/data/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use basics::base_uint::Uint256;
use protocol::JsonValue;

pub struct LedgerRequestSource;

pub fn do_ledger_request<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, LedgerRequestSource, Runtime>,
) -> Result<JsonValue, Status> {
    tracing::debug!(target: "rpc", method = "ledger_request", "RPC request received");
    let JsonValue::Object(params) = ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    let has_index = params.contains_key("ledger_index");
    let has_hash = params.contains_key("ledger_hash");
    if has_index == has_hash {
        return Err(Status::make_param_error(
            "Exactly one of ledger_index or ledger_hash can be set.",
        ));
    }

    if has_index {
        let ledger_index = params
            .get("ledger_index")
            .and_then(JsonValue::as_u64)
            .map(|value| value as u32)
            .ok_or_else(|| Status::expected_field_error("ledger_index", "number"))?;
        let status = ctx.runtime.ledger_request(ledger_index);
        if !status.is_ok() {
            return Err(status);
        }

        return Ok(protocol::json!({
            "message": "ledger requested",
            "ledger_index": ledger_index
        }));
    }

    let ledger_hash = params
        .get("ledger_hash")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Status::expected_field_error("ledger_hash", "string"))?;
    let hash = Uint256::from_hex(ledger_hash)
        .map_err(|_| Status::expected_field_error("ledger_hash", "hex string"))?;

    let status = ctx.runtime.ledger_request_by_hash(hash);
    if !status.is_ok() {
        return Err(status);
    }

    Ok(protocol::json!({
        "message": "ledger requested",
        "ledger_hash": hash.to_string()
    }))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use super::{LedgerRequestSource, do_ledger_request};
    use crate::state::context::{JsonContextHeaders, RpcLoadType, RpcRequestContext, RpcRuntime};
    use crate::state::role::Role;
    use crate::status::{RpcErrorCode, Status};
    use basics::base_uint::Uint256;
    use protocol::JsonValue;

    #[derive(Default)]
    struct FakeRuntime {
        requested_seq: Cell<Option<u32>>,
        requested_hash: RefCell<Option<Uint256>>,
        seq_status: Cell<Option<RpcErrorCode>>,
        hash_status: Cell<Option<RpcErrorCode>>,
    }

    impl RpcRuntime for FakeRuntime {
        fn ledger_request(&self, seq: u32) -> Status {
            self.requested_seq.set(Some(seq));
            self.seq_status.get().map_or(Status::OK, Status::new)
        }

        fn ledger_request_by_hash(&self, hash: Uint256) -> Status {
            self.requested_hash.replace(Some(hash));
            self.hash_status.get().map_or(Status::OK, Status::new)
        }
    }

    fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
        JsonValue::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    fn context<'a>(
        params: &'a JsonValue,
        runtime: &'a FakeRuntime,
    ) -> RpcRequestContext<'a, LedgerRequestSource, FakeRuntime> {
        RpcRequestContext {
            params,
            env: &LedgerRequestSource,
            runtime,
            role: Role::Admin,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            request_headers: BTreeMap::new(),
            unlimited: true,
            remote_ip: None,
            load_type: RpcLoadType::Reference,
        }
    }

    #[test]
    fn ledger_request_dispatches_index_requests() {
        let runtime = FakeRuntime::default();
        let params = object([("ledger_index", JsonValue::Unsigned(321))]);
        let result =
            do_ledger_request(&context(&params, &runtime)).expect("index request should work");

        assert_eq!(runtime.requested_seq.get(), Some(321));
        assert!(runtime.requested_hash.borrow().is_none());
        let JsonValue::Object(result) = result else {
            panic!("result must be an object");
        };
        assert_eq!(result.get("ledger_index"), Some(&JsonValue::Unsigned(321)));
    }

    #[test]
    fn ledger_request_dispatches_hash_requests() {
        let runtime = FakeRuntime::default();
        let hash = Uint256::from_array([0x42; 32]);
        let params = object([("ledger_hash", JsonValue::String(hash.to_string()))]);
        let result =
            do_ledger_request(&context(&params, &runtime)).expect("hash request should work");

        assert_eq!(*runtime.requested_hash.borrow(), Some(hash));
        assert_eq!(runtime.requested_seq.get(), None);
        let JsonValue::Object(result) = result else {
            panic!("result must be an object");
        };
        assert_eq!(
            result.get("ledger_hash"),
            Some(&JsonValue::String(hash.to_string()))
        );
    }

    #[test]
    fn ledger_request_requires_exactly_one_selector() {
        let runtime = FakeRuntime::default();
        let hash = Uint256::from_array([0x11; 32]);
        let params = object([
            ("ledger_index", JsonValue::Unsigned(5)),
            ("ledger_hash", JsonValue::String(hash.to_string())),
        ]);
        let error =
            do_ledger_request(&context(&params, &runtime)).expect_err("both fields should fail");
        assert_eq!(error.error_code(), Some(RpcErrorCode::InvalidParams));

        let params = object([]);
        let error =
            do_ledger_request(&context(&params, &runtime)).expect_err("missing fields should fail");
        assert_eq!(error.error_code(), Some(RpcErrorCode::InvalidParams));
    }

    #[test]
    fn ledger_request_rejects_malformed_hash() {
        let runtime = FakeRuntime::default();
        let params = object([("ledger_hash", JsonValue::String("XYZ".to_owned()))]);
        let error =
            do_ledger_request(&context(&params, &runtime)).expect_err("bad hash should fail");
        assert_eq!(error.error_code(), Some(RpcErrorCode::InvalidParams));
    }

    #[test]
    fn ledger_request_propagates_runtime_errors() {
        let runtime = FakeRuntime {
            seq_status: Cell::new(Some(RpcErrorCode::LedgerNotFound)),
            ..FakeRuntime::default()
        };
        let params = object([("ledger_index", JsonValue::Unsigned(900))]);
        let error = do_ledger_request(&context(&params, &runtime))
            .expect_err("runtime error should bubble out");
        assert_eq!(error.error_code(), Some(RpcErrorCode::LedgerNotFound));

        let runtime = FakeRuntime {
            hash_status: Cell::new(Some(RpcErrorCode::NoNetwork)),
            ..FakeRuntime::default()
        };
        let hash = Uint256::from_array([0x99; 32]);
        let params = object([("ledger_hash", JsonValue::String(hash.to_string()))]);
        let error = do_ledger_request(&context(&params, &runtime))
            .expect_err("runtime hash error should bubble out");
        assert_eq!(error.error_code(), Some(RpcErrorCode::NoNetwork));
    }
}
