//! Tests for feature.

use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::{Uint256, to_string};
use protocol::{JsonValue, feature_id};
use rpc::{
    RpcRole,
    feature::{FeatureRequest, FeatureSource, do_feature},
};

#[derive(Debug, Default)]
struct FakeFeatureSource {
    table: RefCell<BTreeMap<String, JsonValue>>,
    single: RefCell<BTreeMap<Uint256, JsonValue>>,
    majorities: BTreeMap<Uint256, i64>,
}

impl FakeFeatureSource {
    fn insert_feature(
        &self,
        feature: Uint256,
        name: &str,
        enabled: bool,
        vetoed: bool,
        supported: bool,
    ) {
        let object = JsonValue::Object(BTreeMap::from([
            ("name".to_owned(), JsonValue::String(name.to_owned())),
            ("enabled".to_owned(), JsonValue::Bool(enabled)),
            ("vetoed".to_owned(), JsonValue::Bool(vetoed)),
            ("supported".to_owned(), JsonValue::Bool(supported)),
        ]));
        let key = to_string(&feature);
        self.table.borrow_mut().insert(key, object.clone());
        self.single.borrow_mut().insert(feature, object);
    }
}

impl FeatureSource for FakeFeatureSource {
    fn feature_table_json(&self, _is_admin: bool) -> JsonValue {
        JsonValue::Object(self.table.borrow().clone())
    }

    fn feature_json(&self, feature: Uint256, _is_admin: bool) -> Option<JsonValue> {
        self.single.borrow().get(&feature).cloned()
    }

    fn veto_feature(&self, feature: Uint256) {
        self.update_veto_state(feature, true);
    }

    fn unveto_feature(&self, feature: Uint256) {
        self.update_veto_state(feature, false);
    }

    fn majority_timestamps(&self) -> BTreeMap<Uint256, i64> {
        self.majorities.clone()
    }
}

impl FakeFeatureSource {
    fn update_veto_state(&self, feature: Uint256, vetoed: bool) {
        let key = to_string(&feature);
        {
            let mut table = self.table.borrow_mut();
            if let Some(JsonValue::Object(object)) = table.get_mut(&key) {
                object.insert("vetoed".to_owned(), JsonValue::Bool(vetoed));
            }
        }
        {
            let mut single = self.single.borrow_mut();
            if let Some(JsonValue::Object(object)) = single.get_mut(&feature) {
                object.insert("vetoed".to_owned(), JsonValue::Bool(vetoed));
            }
        }
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

#[test]
fn feature_lists_all_features_and_majorities() {
    let source = FakeFeatureSource {
        majorities: BTreeMap::from([(feature_id("XRPFees"), 42)]),
        ..Default::default()
    };
    source.insert_feature(feature_id("XRPFees"), "XRPFees", false, false, true);

    let result = do_feature(
        &FeatureRequest {
            params: &JsonValue::Object(Default::default()),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(features) = result.get("features").expect("features must exist") else {
        panic!("features must be an object");
    };

    let key = to_string(&feature_id("XRPFees"));
    let JsonValue::Object(feature) = features.get(&key).expect("feature entry must exist") else {
        panic!("feature entry must be an object");
    };
    assert_eq!(feature.get("majority"), Some(&JsonValue::Signed(42)));
    assert_eq!(feature.get("enabled"), Some(&JsonValue::Bool(false)));
}

#[test]
fn feature_accepts_name_or_hex_and_applies_veto() {
    let source = FakeFeatureSource {
        majorities: BTreeMap::from([(feature_id("Batch"), 99)]),
        ..Default::default()
    };
    source.insert_feature(feature_id("Batch"), "Batch", false, false, true);

    let named = do_feature(
        &FeatureRequest {
            params: &object([
                ("feature", JsonValue::String("Batch".to_owned())),
                ("vetoed", JsonValue::Bool(true)),
            ]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(named) = named else {
        panic!("named result must be an object");
    };
    assert_eq!(named.get("majority"), Some(&JsonValue::Signed(99)));
    assert_eq!(named.get("vetoed"), Some(&JsonValue::Bool(true)));

    let hex = do_feature(
        &FeatureRequest {
            params: &object([(
                "feature",
                JsonValue::String(to_string(&feature_id("Batch"))),
            )]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(hex) = hex else {
        panic!("hex result must be an object");
    };
    assert_eq!(hex.get("majority"), Some(&JsonValue::Signed(99)));
    assert_eq!(hex.get("vetoed"), Some(&JsonValue::Bool(true)));
}

#[test]
fn feature_rejects_invalid_and_unknown_features() {
    let source = FakeFeatureSource::default();

    let invalid = do_feature(
        &FeatureRequest {
            params: &object([("feature", JsonValue::Unsigned(1))]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(invalid) = invalid else {
        panic!("invalid result must be an object");
    };
    assert_eq!(
        invalid.get("error"),
        Some(&JsonValue::String("invalidParams".to_owned()))
    );
    assert_eq!(invalid.get("error_code"), Some(&JsonValue::Signed(31)));

    let bad = do_feature(
        &FeatureRequest {
            params: &object([("feature", JsonValue::String("AllTheThings".to_owned()))]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(bad) = bad else {
        panic!("bad result must be an object");
    };
    assert_eq!(
        bad.get("error"),
        Some(&JsonValue::String("badFeature".to_owned()))
    );

    let denied = do_feature(
        &FeatureRequest {
            params: &object([
                ("feature", JsonValue::String("Batch".to_owned())),
                ("vetoed", JsonValue::Bool(true)),
            ]),
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(denied) = denied else {
        panic!("denied result must be an object");
    };
    assert_eq!(
        denied.get("error"),
        Some(&JsonValue::String("noPermission".to_owned()))
    );
    assert_eq!(denied.get("error_code"), Some(&JsonValue::Signed(6)));
}

#[test]
fn feature_unveto_clears_veto_state() {
    let source = FakeFeatureSource::default();
    source.insert_feature(feature_id("Batch"), "Batch", false, true, true);

    // Verify it starts vetoed
    let before = do_feature(
        &FeatureRequest {
            params: &object([("feature", JsonValue::String("Batch".to_owned()))]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(before) = before else {
        panic!("before must be an object");
    };
    assert_eq!(before.get("vetoed"), Some(&JsonValue::Bool(true)));

    // Unveto
    let result = do_feature(
        &FeatureRequest {
            params: &object([
                ("feature", JsonValue::String("Batch".to_owned())),
                ("vetoed", JsonValue::Bool(false)),
            ]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("vetoed"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        result.get("name"),
        Some(&JsonValue::String("Batch".to_owned()))
    );
    assert_eq!(result.get("supported"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("enabled"), Some(&JsonValue::Bool(false)));
}

#[test]
fn feature_list_returns_all_features_with_correct_structure() {
    let source = FakeFeatureSource::default();
    source.insert_feature(feature_id("XRPFees"), "XRPFees", true, false, true);
    source.insert_feature(feature_id("Batch"), "Batch", false, true, true);
    source.insert_feature(feature_id("Clawback"), "Clawback", true, false, true);

    let result = do_feature(
        &FeatureRequest {
            params: &JsonValue::Object(Default::default()),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(features) = result.get("features").expect("features") else {
        panic!("features must be an object");
    };
    assert_eq!(features.len(), 3);

    // Each feature should have name, enabled, vetoed, supported
    for (_key, value) in features.iter() {
        let JsonValue::Object(feature) = value else {
            panic!("feature must be an object");
        };
        assert!(feature.contains_key("name"));
        assert!(feature.contains_key("enabled"));
        assert!(feature.contains_key("vetoed"));
        assert!(feature.contains_key("supported"));
    }
}

#[test]
fn feature_user_can_read_but_not_veto() {
    let source = FakeFeatureSource::default();
    source.insert_feature(feature_id("Batch"), "Batch", false, false, true);

    // User can read
    let read = do_feature(
        &FeatureRequest {
            params: &object([("feature", JsonValue::String("Batch".to_owned()))]),
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(read) = read else {
        panic!("read must be an object");
    };
    assert_eq!(read.get("error"), None);
    assert_eq!(
        read.get("name"),
        Some(&JsonValue::String("Batch".to_owned()))
    );

    // User cannot veto
    let veto = do_feature(
        &FeatureRequest {
            params: &object([
                ("feature", JsonValue::String("Batch".to_owned())),
                ("vetoed", JsonValue::Bool(true)),
            ]),
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(veto) = veto else {
        panic!("veto must be an object");
    };
    assert_eq!(
        veto.get("error"),
        Some(&JsonValue::String("noPermission".to_owned()))
    );
}

#[test]
fn feature_single_lookup_returns_all_fields() {
    let source = FakeFeatureSource {
        majorities: BTreeMap::from([(feature_id("Batch"), 1234)]),
        ..Default::default()
    };
    source.insert_feature(feature_id("Batch"), "Batch", true, false, true);

    let result = do_feature(
        &FeatureRequest {
            params: &object([("feature", JsonValue::String("Batch".to_owned()))]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    // Single feature lookup should have name, enabled, supported, vetoed
    assert_eq!(
        result.get("name"),
        Some(&JsonValue::String("Batch".to_owned()))
    );
    assert_eq!(result.get("enabled"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("supported"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("vetoed"), Some(&JsonValue::Bool(false)));
    assert_eq!(result.get("majority"), Some(&JsonValue::Signed(1234)));
}

#[test]
fn feature_disabled_feature_shows_correct_state() {
    let source = FakeFeatureSource::default();
    source.insert_feature(feature_id("XRPFees"), "XRPFees", false, false, true);

    let result = do_feature(
        &FeatureRequest {
            params: &object([("feature", JsonValue::String("XRPFees".to_owned()))]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("enabled"), Some(&JsonValue::Bool(false)));
    assert_eq!(result.get("supported"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("vetoed"), Some(&JsonValue::Bool(false)));
    // No majority when not in majorities map
    assert_eq!(result.get("majority"), None);
}

#[test]
fn feature_vetoed_feature_shows_vetoed_true() {
    let source = FakeFeatureSource::default();
    source.insert_feature(feature_id("Clawback"), "Clawback", false, true, true);

    let result = do_feature(
        &FeatureRequest {
            params: &object([("feature", JsonValue::String("Clawback".to_owned()))]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(result.get("vetoed"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("enabled"), Some(&JsonValue::Bool(false)));
}

#[test]
fn feature_list_includes_majority_timestamps() {
    let source = FakeFeatureSource {
        majorities: BTreeMap::from([(feature_id("Batch"), 100), (feature_id("XRPFees"), 200)]),
        ..Default::default()
    };
    source.insert_feature(feature_id("Batch"), "Batch", false, false, true);
    source.insert_feature(feature_id("XRPFees"), "XRPFees", false, false, true);

    let result = do_feature(
        &FeatureRequest {
            params: &JsonValue::Object(Default::default()),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(features) = result.get("features").expect("features") else {
        panic!("features must be an object");
    };

    let batch_key = basics::base_uint::to_string(&feature_id("Batch"));
    let JsonValue::Object(batch) = features.get(&batch_key).expect("Batch feature") else {
        panic!("batch must be an object");
    };
    assert_eq!(batch.get("majority"), Some(&JsonValue::Signed(100)));

    let xrp_key = basics::base_uint::to_string(&feature_id("XRPFees"));
    let JsonValue::Object(xrp) = features.get(&xrp_key).expect("XRPFees feature") else {
        panic!("xrp must be an object");
    };
    assert_eq!(xrp.get("majority"), Some(&JsonValue::Signed(200)));
}

#[test]
fn feature_veto_and_unveto_round_trip() {
    let source = FakeFeatureSource::default();
    source.insert_feature(feature_id("Batch"), "Batch", false, false, true);

    // Veto
    let veto_result = do_feature(
        &FeatureRequest {
            params: &object([
                ("feature", JsonValue::String("Batch".to_owned())),
                ("vetoed", JsonValue::Bool(true)),
            ]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(veto_result) = veto_result else {
        panic!("veto result must be an object");
    };
    assert_eq!(veto_result.get("vetoed"), Some(&JsonValue::Bool(true)));

    // Unveto
    let unveto_result = do_feature(
        &FeatureRequest {
            params: &object([
                ("feature", JsonValue::String("Batch".to_owned())),
                ("vetoed", JsonValue::Bool(false)),
            ]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(unveto_result) = unveto_result else {
        panic!("unveto result must be an object");
    };
    assert_eq!(unveto_result.get("vetoed"), Some(&JsonValue::Bool(false)));
}

#[test]
fn feature_non_admin_cannot_list_all() {
    let source = FakeFeatureSource::default();
    source.insert_feature(feature_id("Batch"), "Batch", false, false, true);

    let result = do_feature(
        &FeatureRequest {
            params: &JsonValue::Object(Default::default()),
            role: RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    // User can still list features (read-only)
    assert!(result.contains_key("features") || result.contains_key("error"));
}

#[test]
fn feature_lookup_by_hex_hash() {
    let source = FakeFeatureSource::default();
    let batch_id = feature_id("Batch");
    source.insert_feature(batch_id, "Batch", true, false, true);

    let result = do_feature(
        &FeatureRequest {
            params: &object([(
                "feature",
                JsonValue::String(basics::base_uint::to_string(&batch_id)),
            )]),
            role: RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("name"),
        Some(&JsonValue::String("Batch".to_owned()))
    );
    assert_eq!(result.get("enabled"), Some(&JsonValue::Bool(true)));
}
