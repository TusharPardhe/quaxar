//! `can_delete` handler port from `xrpld/rpc/handlers/admin/data/the reference source`.

use crate::state::context::{RpcRequestContext, RpcRuntime};
use crate::status::{RpcErrorCode, Status};
use basics::base_uint::Uint256;
use protocol::JsonValue;

pub struct CanDeleteSource;

pub fn do_can_delete<Runtime: RpcRuntime>(
    ctx: &RpcRequestContext<'_, CanDeleteSource, Runtime>,
) -> Result<JsonValue, Status> {
    let JsonValue::Object(params) = ctx.params else {
        return Err(Status::new(RpcErrorCode::InvalidParams));
    };

    if !ctx.runtime.can_delete_enabled() {
        return Err(Status::new(RpcErrorCode::NotEnabled));
    }

    if let Some(can_delete) = params.get("can_delete") {
        if let Some(seq_str) = can_delete.as_str() {
            let seq_str = seq_str.to_ascii_lowercase();
            if seq_str.chars().all(|c| c.is_ascii_digit()) {
                let seq = seq_str
                    .parse::<u32>()
                    .map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
                let status = ctx.runtime.can_delete_set(seq);
                if !status.is_ok() {
                    return Err(status);
                }
            } else if seq_str == "never" {
                let status = ctx.runtime.can_delete_set(0);
                if !status.is_ok() {
                    return Err(status);
                }
            } else if seq_str == "always" {
                let status = ctx.runtime.can_delete_set(u32::MAX);
                if !status.is_ok() {
                    return Err(status);
                }
            } else if seq_str == "now" {
                let last_rotated = ctx.runtime.can_delete_last_rotated();
                if last_rotated == 0 {
                    return Err(Status::new(RpcErrorCode::NotReady));
                }
                let status = ctx.runtime.can_delete_set(last_rotated);
                if !status.is_ok() {
                    return Err(status);
                }
            } else if seq_str.len() == 64 {
                let hash = Uint256::from_hex(&seq_str)
                    .map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
                let seq = ctx
                    .runtime
                    .can_delete_seq_by_hash(hash)
                    .ok_or_else(|| Status::new(RpcErrorCode::LedgerNotFound))?;
                let status = ctx.runtime.can_delete_set(seq);
                if !status.is_ok() {
                    return Err(status);
                }
            } else {
                return Err(Status::new(RpcErrorCode::InvalidParams));
            }
        } else if let Some(seq) = can_delete.as_u64() {
            let seq = u32::try_from(seq).map_err(|_| Status::new(RpcErrorCode::InvalidParams))?;
            let status = ctx.runtime.can_delete_set(seq);
            if !status.is_ok() {
                return Err(status);
            }
        } else {
            return Err(Status::expected_field_error(
                "can_delete",
                "number or string",
            ));
        }
    }

    Ok(protocol::json!({
        "can_delete": ctx.runtime.can_delete_get()
    }))
}

#[cfg(test)]
mod tests {
    use super::{CanDeleteSource, do_can_delete};
    use crate::state::context::{JsonContextHeaders, RpcLoadType, RpcRequestContext, RpcRuntime};
    use crate::state::role::Role;
    use crate::status::{RpcErrorCode, Status};
    use basics::base_uint::Uint256;
    use protocol::JsonValue;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeRuntime {
        enabled: bool,
        can_delete: Cell<u32>,
        last_rotated: u32,
        hash_seq: Option<(Uint256, u32)>,
    }

    impl RpcRuntime for FakeRuntime {
        fn can_delete_get(&self) -> u32 {
            self.can_delete.get()
        }

        fn can_delete_enabled(&self) -> bool {
            self.enabled
        }

        fn can_delete_last_rotated(&self) -> u32 {
            self.last_rotated
        }

        fn can_delete_seq_by_hash(&self, hash: Uint256) -> Option<u32> {
            self.hash_seq
                .filter(|(expected, _)| *expected == hash)
                .map(|(_, seq)| seq)
        }

        fn can_delete_set(&self, seq: u32) -> Status {
            self.can_delete.set(seq);
            Status::OK
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

    fn request<'a>(
        params: &'a JsonValue,
        runtime: &'a FakeRuntime,
    ) -> RpcRequestContext<'a, CanDeleteSource, FakeRuntime> {
        RpcRequestContext {
            params,
            env: &CanDeleteSource,
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

    fn can_delete_field(value: &JsonValue) -> Option<&JsonValue> {
        let JsonValue::Object(object) = value else {
            return None;
        };
        object.get("can_delete")
    }

    #[test]
    fn rejects_when_advisory_delete_is_disabled() {
        let params = object([]);
        let runtime = FakeRuntime::default();

        let error = do_can_delete(&request(&params, &runtime)).expect_err("must reject");

        assert_eq!(error.error_code(), Some(RpcErrorCode::NotEnabled));
    }

    #[test]
    fn reports_current_can_delete_when_enabled() {
        let params = object([]);
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(500),
            last_rotated: 0,
            hash_seq: None,
        };

        let response = do_can_delete(&request(&params, &runtime)).expect("can_delete response");

        assert_eq!(can_delete_field(&response), Some(&JsonValue::Unsigned(500)));
    }

    #[test]
    fn now_requires_a_prior_rotation() {
        let params = object([("can_delete", JsonValue::String("now".to_owned()))]);
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(0),
            last_rotated: 0,
            hash_seq: None,
        };

        let error = do_can_delete(&request(&params, &runtime)).expect_err("must reject");

        assert_eq!(error.error_code(), Some(RpcErrorCode::NotReady));
        assert_eq!(runtime.can_delete.get(), 0);
    }

    #[test]
    fn now_uses_last_rotated_ledger() {
        let params = object([("can_delete", JsonValue::String("now".to_owned()))]);
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(0),
            last_rotated: 700,
            hash_seq: None,
        };

        let response = do_can_delete(&request(&params, &runtime)).expect("can_delete response");

        assert_eq!(runtime.can_delete.get(), 700);
        assert_eq!(can_delete_field(&response), Some(&JsonValue::Unsigned(700)));
    }

    #[test]
    fn never_and_always_match_reference_values() {
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(42),
            last_rotated: 700,
            hash_seq: None,
        };

        let never = object([("can_delete", JsonValue::String("never".to_owned()))]);
        let response = do_can_delete(&request(&never, &runtime)).expect("never response");
        assert_eq!(runtime.can_delete.get(), 0);
        assert_eq!(can_delete_field(&response), Some(&JsonValue::Unsigned(0)));

        let always = object([("can_delete", JsonValue::String("always".to_owned()))]);
        let response = do_can_delete(&request(&always, &runtime)).expect("always response");
        assert_eq!(runtime.can_delete.get(), u32::MAX);
        assert_eq!(
            can_delete_field(&response),
            Some(&JsonValue::Unsigned(u64::from(u32::MAX)))
        );
    }

    #[test]
    fn accepts_keywords_case_insensitively() {
        let params = object([("can_delete", JsonValue::String("NOW".to_owned()))]);
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(0),
            last_rotated: 900,
            hash_seq: None,
        };

        let response = do_can_delete(&request(&params, &runtime)).expect("now response");

        assert_eq!(runtime.can_delete.get(), 900);
        assert_eq!(can_delete_field(&response), Some(&JsonValue::Unsigned(900)));
    }

    #[test]
    fn ledger_hash_sets_can_delete_to_matching_ledger_sequence() {
        let hash_text = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        let hash = Uint256::from_hex(hash_text).expect("test hash");
        let params = object([("can_delete", JsonValue::String(hash_text.to_owned()))]);
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(0),
            last_rotated: 0,
            hash_seq: Some((hash, 1234)),
        };

        let response = do_can_delete(&request(&params, &runtime)).expect("hash response");

        assert_eq!(runtime.can_delete.get(), 1234);
        assert_eq!(
            can_delete_field(&response),
            Some(&JsonValue::Unsigned(1234))
        );
    }

    #[test]
    fn valid_unknown_ledger_hash_reports_ledger_not_found() {
        let params = object([(
            "can_delete",
            JsonValue::String(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
            ),
        )]);
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(0),
            last_rotated: 0,
            hash_seq: None,
        };

        let error = do_can_delete(&request(&params, &runtime)).expect_err("must reject");

        assert_eq!(error.error_code(), Some(RpcErrorCode::LedgerNotFound));
    }

    #[test]
    fn rejects_invalid_strings_and_out_of_range_unsigned_values() {
        let runtime = FakeRuntime {
            enabled: true,
            can_delete: Cell::new(42),
            last_rotated: 0,
            hash_seq: None,
        };

        let invalid = object([("can_delete", JsonValue::String("not-a-ledger".to_owned()))]);
        let error = do_can_delete(&request(&invalid, &runtime)).expect_err("must reject");
        assert_eq!(error.error_code(), Some(RpcErrorCode::InvalidParams));
        assert_eq!(runtime.can_delete.get(), 42);

        let overflow = object([("can_delete", JsonValue::Unsigned(u64::from(u32::MAX) + 1))]);
        let error = do_can_delete(&request(&overflow, &runtime)).expect_err("must reject");
        assert_eq!(error.error_code(), Some(RpcErrorCode::InvalidParams));
        assert_eq!(runtime.can_delete.get(), 42);
    }
}
