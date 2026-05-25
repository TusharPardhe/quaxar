//! Deprecated RPC-error wrappers from `xrpl/protocol/RPCErr.*`.

use crate::{JsonValue, contains_error, make_error};

pub fn is_rpc_error(result: &JsonValue) -> bool {
    contains_error(result)
}

pub fn rpc_error(code: i32) -> JsonValue {
    make_error(code)
}
