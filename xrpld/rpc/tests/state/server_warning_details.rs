//! Tests for server warning details.

use app::{ApplicationRoot, UnsupportedMajorityWarningDetails};
use protocol::JsonValue;
use rpc::{
    ApplicationServerInfo, JsonContext, JsonContextHeaders, RpcRole, WARN_RPC_UNSUPPORTED_MAJORITY,
    do_server_info,
};
use std::collections::BTreeMap;

fn context<'a, Env>(params: &'a JsonValue, env: &'a Env, role: RpcRole) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: matches!(role, RpcRole::Admin),
    }
}

#[test]
fn server_info_only_shows_unsupported_majority_details_to_admin() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_unsupported_majority_warning_details(Some(UnsupportedMajorityWarningDetails {
        expected_date: 1_700_000_000,
        expected_date_utc: "2023-Nov-14 22:13:20 UTC".to_owned(),
    }));
    let source = ApplicationServerInfo::new(&app);
    let params = JsonValue::Object(BTreeMap::new());

    let admin = do_server_info(&context(&params, &source, RpcRole::Admin));
    let guest = do_server_info(&context(&params, &source, RpcRole::User));

    let JsonValue::Object(admin) = admin else {
        panic!("admin result must be an object");
    };
    let JsonValue::Object(guest) = guest else {
        panic!("guest result must be an object");
    };
    let JsonValue::Object(admin_info) = admin.get("info").expect("info must exist") else {
        panic!("admin info must be an object");
    };
    let JsonValue::Array(admin_warnings) = admin_info
        .get("warnings")
        .expect("admin warnings must exist")
    else {
        panic!("admin warnings must be an array");
    };
    let JsonValue::Object(admin_warning) = &admin_warnings[0] else {
        panic!("admin warning must be an object");
    };
    let JsonValue::Object(details) = admin_warning.get("details").expect("details must exist")
    else {
        panic!("details must be an object");
    };
    let JsonValue::Object(guest_info) = guest.get("info").expect("info must exist") else {
        panic!("guest info must be an object");
    };

    assert_eq!(
        details.get("expected_date_UTC"),
        Some(&JsonValue::String("2023-Nov-14 22:13:20 UTC".to_owned()))
    );
    assert!(!guest_info.contains_key("warnings"));
}

#[test]
fn server_info_keeps_admin_unsupported_majority_warning_without_details() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_unsupported_majority_warned(true);
    let source = ApplicationServerInfo::new(&app);
    let params = JsonValue::Object(BTreeMap::new());

    let admin = do_server_info(&context(&params, &source, RpcRole::Admin));
    let guest = do_server_info(&context(&params, &source, RpcRole::User));

    let JsonValue::Object(admin) = admin else {
        panic!("admin result must be an object");
    };
    let JsonValue::Object(guest) = guest else {
        panic!("guest result must be an object");
    };
    let JsonValue::Object(admin_info) = admin.get("info").expect("info must exist") else {
        panic!("admin info must be an object");
    };
    let JsonValue::Array(admin_warnings) = admin_info
        .get("warnings")
        .expect("admin warnings must exist")
    else {
        panic!("admin warnings must be an array");
    };
    let JsonValue::Object(admin_warning) = &admin_warnings[0] else {
        panic!("admin warning must be an object");
    };
    let JsonValue::Object(guest_info) = guest.get("info").expect("info must exist") else {
        panic!("guest info must be an object");
    };

    assert_eq!(admin_warnings.len(), 1);
    assert_eq!(
        admin_warning.get("id"),
        Some(&JsonValue::Signed(WARN_RPC_UNSUPPORTED_MAJORITY))
    );
    assert!(!admin_warning.contains_key("details"));
    assert!(!guest_info.contains_key("warnings"));
}

#[test]
fn server_info_shows_amendment_blocked_warning() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_amendment_blocked(true);
    let source = ApplicationServerInfo::new(&app);
    let params = JsonValue::Object(BTreeMap::new());

    let result = do_server_info(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info") else {
        panic!("info must be an object");
    };

    // When amendment_blocked, should have warnings array with amendment_blocked warning
    if let Some(JsonValue::Array(warnings)) = info.get("warnings") {
        let has_amendment_blocked = warnings.iter().any(|w| {
            matches!(w, JsonValue::Object(obj) if obj.get("id") == Some(&JsonValue::Signed(rpc::WARN_RPC_AMENDMENT_BLOCKED)))
        });
        assert!(
            has_amendment_blocked,
            "should have amendment_blocked warning"
        );
    }
}

#[test]
fn server_info_no_warnings_when_not_blocked() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    // Don't set amendment_blocked
    let source = ApplicationServerInfo::new(&app);
    let params = JsonValue::Object(BTreeMap::new());

    let result = do_server_info(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info") else {
        panic!("info must be an object");
    };

    // No warnings when nothing is wrong
    assert!(
        !info.contains_key("warnings"),
        "should not have warnings when not blocked"
    );
}
