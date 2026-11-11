//! `log_level` handler port from `xrpld/rpc/handlers/admin/log/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use protocol::JsonValue;

pub struct LogLevelSource;

pub fn do_log_level<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, LogLevelSource, Runtime>,
) -> Result<JsonValue, Status> {
    let JsonValue::Object(params) = ctx.params else {
        return Ok(ctx.runtime.log_level_get());
    };

    if let Some(severity) = params.get("severity") {
        // rippled #7595: a non-string severity or partition is invalidParams.
        let severity = severity
            .as_str()
            .ok_or_else(|| Status::new(RpcErrorCode::InvalidParams))?;
        let partition = match params.get("partition") {
            None => "base",
            Some(partition) => partition
                .as_str()
                .ok_or_else(|| Status::new(RpcErrorCode::InvalidParams))?,
        };
        let status = ctx
            .runtime
            .log_level_set(partition.to_string(), severity.to_string());
        if !status.is_ok() {
            return Err(status);
        }
    }

    Ok(ctx.runtime.log_level_get())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{LogLevelSource, do_log_level};
    use crate::state::context::{JsonContextHeaders, RpcLoadType, RpcRequestContext};
    use crate::state::role::Role;
    use crate::status::{RpcErrorCode, Status};
    use protocol::JsonValue;

    fn run(entries: &[(&str, JsonValue)]) -> Result<JsonValue, Status> {
        let params = JsonValue::Object(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        );
        do_log_level(&RpcRequestContext {
            params: &params,
            env: &LogLevelSource,
            runtime: &(),
            role: Role::Admin,
            api_version: 2,
            headers: JsonContextHeaders::default(),
            request_headers: BTreeMap::new(),
            unlimited: true,
            remote_ip: None,
            load_type: RpcLoadType::Reference,
        })
    }

    /// rippled #7595 (ServerInfo_test testLogLevelParameterTypes).
    #[test]
    fn non_string_severity_or_partition_is_invalid_params() {
        let invalid = Err(Status::new(RpcErrorCode::InvalidParams));
        assert_eq!(run(&[("severity", JsonValue::Unsigned(5))]), invalid);
        assert_eq!(
            run(&[
                ("severity", JsonValue::String("warn".into())),
                ("partition", JsonValue::Unsigned(5)),
            ]),
            invalid
        );
        assert!(
            run(&[
                ("severity", JsonValue::String("warn".into())),
                ("partition", JsonValue::String("Peer".into())),
            ])
            .is_ok()
        );
    }
}
