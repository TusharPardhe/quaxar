//! Narrow `server_info` RPC handler port.

use std::collections::BTreeMap;

use protocol::JsonValue;

use crate::{JsonContext, RpcRole};

pub trait ServerInfoSource {
    fn get_server_info(&self, human: bool, admin: bool, counters: bool) -> JsonValue;
}

fn json_value_as_bool(value: &JsonValue) -> bool {
    match value {
        JsonValue::Bool(value) => *value,
        JsonValue::Signed(value) => *value != 0,
        JsonValue::Unsigned(value) => *value != 0,
        JsonValue::String(value) => !value.is_empty(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => false,
    }
}

pub(crate) fn want_counters(params: &JsonValue) -> bool {
    let JsonValue::Object(object) = params else {
        return false;
    };

    object.get("counters").is_some_and(json_value_as_bool)
}

pub(crate) fn build_response<S: ServerInfoSource>(
    context: &JsonContext<'_, S>,
    human: bool,
    key: &str,
) -> JsonValue {
    JsonValue::Object(BTreeMap::from([(
        key.to_owned(),
        context.env.get_server_info(
            human,
            context.role == RpcRole::Admin,
            want_counters(context.params),
        ),
    )]))
}

pub fn do_server_info<S: ServerInfoSource>(context: &JsonContext<'_, S>) -> JsonValue {
    build_response(context, true, "info")
}

#[cfg(test)]
mod tests {
    use super::want_counters;
    use protocol::JsonValue;
    use std::collections::BTreeMap;

    #[test]
    fn want_counters_accepts_truthy_values_and_rejects_falsey_inputs() {
        let params = JsonValue::Object(BTreeMap::from([
            ("counters".to_owned(), JsonValue::Bool(true)),
            ("ignored".to_owned(), JsonValue::Unsigned(0)),
        ]));
        assert!(want_counters(&params));

        let params = JsonValue::Object(BTreeMap::from([(
            "counters".to_owned(),
            JsonValue::String("1".to_owned()),
        )]));
        assert!(want_counters(&params));

        let params = JsonValue::Object(BTreeMap::from([(
            "counters".to_owned(),
            JsonValue::Bool(false),
        )]));
        assert!(!want_counters(&params));
        assert!(!want_counters(&JsonValue::Array(Vec::new())));
    }
}
