//! Integration tests for the feature RPC handler.

use protocol::JsonValue;
use rpc::feature::{do_feature, FeatureRequest};
use rpc_integration_tests::env::*;

#[test]
fn feature_list_all_via_app() {
    let alice = TestAccount::new("feat_alice");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = do_feature(
        &FeatureRequest {
            params: &JsonValue::Object(Default::default()),
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };

    // Should have features map
    assert!(result.contains_key("features"));
    let JsonValue::Object(features) = result.get("features").unwrap() else {
        panic!("object")
    };

    // Each feature should have name, enabled, supported
    for (_key, value) in features.iter().take(3) {
        let JsonValue::Object(feature) = value else {
            continue;
        };
        assert!(feature.contains_key("name"));
        assert!(feature.contains_key("enabled"));
        assert!(feature.contains_key("supported"));
    }
}

#[test]
fn feature_veto_requires_admin() {
    let alice = TestAccount::new("feat_alice2");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = do_feature(
        &FeatureRequest {
            params: &json([
                ("feature", JsonValue::String("Batch".to_owned())),
                ("vetoed", JsonValue::Bool(true)),
            ]),
            role: rpc::RpcRole::User,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("noPermission".to_owned()))
    );
}

#[test]
fn feature_unknown_returns_bad_feature() {
    let alice = TestAccount::new("feat_alice3");
    let env = RpcTestEnv::new(&[(&alice, 10_000_000_000)]);

    let source = env.rpc_source();
    let result = do_feature(
        &FeatureRequest {
            params: &json([(
                "feature",
                JsonValue::String("NonExistentFeature".to_owned()),
            )]),
            role: rpc::RpcRole::Admin,
        },
        &source,
    );
    let JsonValue::Object(result) = result else {
        panic!("object")
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("badFeature".to_owned()))
    );
}
