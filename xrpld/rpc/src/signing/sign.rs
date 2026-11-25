//! `sign` handler port from `xrpld/rpc/handlers/admin/signing/the reference source`.

use crate::commands::rpc_helpers::{check_signing_supported, transaction_sign};
use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::Status;
use protocol::{JsonValue, jss};

pub struct SignSource;

pub const SIGN_DEPRECATED: &str = "This command has been deprecated and will be removed in a \
     future version of the server. Please migrate to a standalone signing tool.";

/// rippled `doSign`: the `deprecated` notice is added to errors too.
pub fn do_sign<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, SignSource, Runtime>,
) -> Result<JsonValue, Status> {
    check_signing_supported(ctx)?;
    let mut ret = match transaction_sign(ctx) {
        Ok(ret) => ret,
        Err(status) => {
            let mut ret = JsonValue::Object(Default::default());
            status.inject(&mut ret);
            ret
        }
    };
    if let JsonValue::Object(object) = &mut ret {
        object.insert(
            jss::deprecated.to_owned(),
            JsonValue::String(SIGN_DEPRECATED.to_owned()),
        );
    }
    Ok(ret)
}
