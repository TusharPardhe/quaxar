//! Narrow `feature` RPC handler port.

use std::collections::BTreeMap;

use basics::base_uint::{Uint256, to_string};
use protocol::{JsonValue, feature_id, feature_name};

use crate::commands::rpc_helpers::rpc_error;
use crate::state::role::Role;
use crate::status::RpcErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureRequest<'a> {
    pub params: &'a JsonValue,
    pub role: Role,
}

pub trait FeatureSource {
    fn feature_table_json(&self, is_admin: bool) -> JsonValue;
    fn feature_json(&self, feature: Uint256, is_admin: bool) -> Option<JsonValue>;
    fn veto_feature(&self, feature: Uint256);
    fn unveto_feature(&self, feature: Uint256);
    fn majority_timestamps(&self) -> BTreeMap<Uint256, i64>;
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

fn ensure_object(value: &mut JsonValue) -> &mut BTreeMap<String, JsonValue> {
    if !matches!(value, JsonValue::Object(_)) {
        *value = JsonValue::Object(BTreeMap::new());
    }

    let JsonValue::Object(object) = value else {
        unreachable!("json value should be an object");
    };
    object
}

fn resolve_feature(value: &str) -> Option<Uint256> {
    let registered = feature_id(value);
    if feature_name(&registered).is_some() {
        return Some(registered);
    }

    Uint256::from_hex(value)
        .ok()
        .filter(|feature| feature_name(feature).is_some())
}

fn insert_majorities(
    features: &mut BTreeMap<String, JsonValue>,
    majorities: &BTreeMap<Uint256, i64>,
) {
    for (feature, majority) in majorities {
        let feature_key = to_string(feature);
        let object = ensure_object(
            features
                .entry(feature_key)
                .or_insert_with(|| JsonValue::Object(BTreeMap::new())),
        );
        object.insert("majority".to_owned(), JsonValue::Signed(*majority));
    }
}

pub fn do_feature<S: FeatureSource>(request: &FeatureRequest<'_>, source: &S) -> JsonValue {
    let is_admin = request.role == Role::Admin;
    let majorities = source.majority_timestamps();

    let JsonValue::Object(object) = request.params else {
        let JsonValue::Object(mut features) = source.feature_table_json(is_admin) else {
            panic!("xrpl::doFeature : invalid features result type");
        };
        insert_majorities(&mut features, &majorities);
        return JsonValue::Object(BTreeMap::from([(
            "features".to_owned(),
            JsonValue::Object(features),
        )]));
    };

    let Some(feature_value) = object.get("feature") else {
        let JsonValue::Object(mut features) = source.feature_table_json(is_admin) else {
            panic!("xrpl::doFeature : invalid features result type");
        };
        insert_majorities(&mut features, &majorities);
        return JsonValue::Object(BTreeMap::from([(
            "features".to_owned(),
            JsonValue::Object(features),
        )]));
    };

    let JsonValue::String(feature_name_value) = feature_value else {
        return rpc_error(RpcErrorCode::InvalidParams);
    };

    let Some(feature) = resolve_feature(feature_name_value) else {
        return rpc_error(RpcErrorCode::BadFeature);
    };

    if let Some(vetoed) = object.get("vetoed") {
        if !is_admin {
            return rpc_error(RpcErrorCode::NoPermission);
        }

        if json_value_as_bool(vetoed) {
            source.veto_feature(feature);
        } else {
            source.unveto_feature(feature);
        }
    }

    let Some(JsonValue::Object(mut reply)) = source.feature_json(feature, is_admin) else {
        return rpc_error(RpcErrorCode::BadFeature);
    };

    if let Some(majority) = majorities.get(&feature) {
        reply.insert("majority".to_owned(), JsonValue::Signed(*majority));
    }

    JsonValue::Object(reply)
}
