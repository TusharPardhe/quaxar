//! Tests for the validator list sites RPC handler.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::JsonValue;
use rpc::{
    JsonContext, JsonContextHeaders, RpcRole, ValidatorListSitesSource, do_validator_list_sites,
};

#[derive(Debug, Default)]
struct FakeValidatorListSitesSource {
    calls: RefCell<u32>,
}

impl ValidatorListSitesSource for FakeValidatorListSitesSource {
    fn get_validator_list_sites(&self) -> JsonValue {
        *self.calls.borrow_mut() += 1;
        JsonValue::Object(BTreeMap::from([
            ("sites".to_owned(), JsonValue::Unsigned(2)),
            ("active".to_owned(), JsonValue::Bool(true)),
        ]))
    }
}

fn context<'a, Env>(params: &'a JsonValue, env: &'a Env, role: RpcRole) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    }
}

#[test]
fn validator_list_sites_returns_source_json() {
    let params = JsonValue::Object(Default::default());
    let source = FakeValidatorListSitesSource::default();
    let result = do_validator_list_sites(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    assert_eq!(result.get("sites"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(result.get("active"), Some(&JsonValue::Bool(true)));
    assert_eq!(*source.calls.borrow(), 1);
}

/// rippled `doValidatorListSites` returns `getValidatorSites().getJson()`.
/// Before the fix quaxar's application source always answered with an empty
/// `validator_sites` list, even with `[validator_list_sites]` configured.
#[test]
fn validator_list_sites_reports_the_configured_sites() {
    let app = app::ApplicationRoot::with_options(app::ApplicationRootOptions {
        standalone: true,
        ..app::ApplicationRootOptions::default()
    })
    .expect("standalone root should build");
    let uri = "https://vl.example.com".to_owned();
    assert!(app.validator_sites().load(std::slice::from_ref(&uri)));

    let JsonValue::Object(result) =
        rpc::ApplicationServerInfo::new(&app).get_validator_list_sites()
    else {
        panic!("result must be an object");
    };
    let Some(JsonValue::Array(sites)) = result.get("validator_sites") else {
        panic!("validator_sites must be an array: {result:?}");
    };
    assert_eq!(sites.len(), 1);
    let JsonValue::Object(site) = &sites[0] else {
        panic!("site must be an object");
    };
    assert_eq!(site.get("uri"), Some(&JsonValue::String(uri)));
    let Some(JsonValue::String(next)) = site.get("next_refresh_time") else {
        panic!("next_refresh_time must be a string: {site:?}");
    };
    // rippled to_string(system_clock::time_point): "%Y-%b-%d %T %Z".
    assert!(next.ends_with(" UTC"), "{next}");
    let time = next.rsplit(' ').nth(1).expect("time part");
    assert_eq!(time.len(), "HH:MM:SS.nnnnnnnnn".len(), "{next}");
}
