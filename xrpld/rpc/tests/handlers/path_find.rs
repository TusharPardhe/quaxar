//! Tests for the path find RPC handler.

use std::{cell::Cell, collections::BTreeMap};

use protocol::JsonValue;
use rpc::{
    PathFindSession, PathFinderRequest, PathFinderSource, PathRequestManager, RpcErrorCode,
    RpcRuntime, do_path_find, do_ripple_path_find,
};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Default)]
struct FakeRuntime {
    path_search_max: Cell<u32>,
    synced: Cell<bool>,
}

impl RpcRuntime for FakeRuntime {
    fn path_search_max(&self) -> u32 {
        self.path_search_max.get()
    }

    fn network_synced(&self) -> bool {
        self.synced.get()
    }
}

#[derive(Default)]
struct FakeSession {
    id: u64,
    api_version: Cell<u32>,
    request_id: Cell<Option<u64>>,
}

impl PathFindSession for FakeSession {
    fn session_id(&self) -> u64 {
        self.id
    }

    fn api_version(&self) -> u32 {
        self.api_version.get()
    }

    fn set_api_version(&self, api_version: u32) {
        self.api_version.set(api_version);
    }

    fn current_path_request_id(&self) -> Option<u64> {
        self.request_id.get()
    }

    fn set_current_path_request_id(&self, request_id: Option<u64>) {
        self.request_id.set(request_id);
    }
}

#[derive(Default)]
struct FakePathSource {
    legacy_calls: Cell<u32>,
    normal_calls: Cell<u32>,
    levels: std::cell::RefCell<Vec<u32>>,
}

impl PathFinderSource for FakePathSource {
    fn find_paths(
        &self,
        request: &PathFinderRequest,
        _params: &JsonValue,
        search_level: u32,
        is_legacy: bool,
    ) -> Result<JsonValue, rpc::RpcStatus> {
        self.levels.borrow_mut().push(search_level);
        if is_legacy {
            self.legacy_calls.set(self.legacy_calls.get() + 1);
        } else {
            self.normal_calls.set(self.normal_calls.get() + 1);
        }
        Ok(JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
            (
                "source_account".to_owned(),
                JsonValue::String(request.source_account.clone()),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(request.destination_account.clone()),
            ),
        ]))]))
    }
}

fn account(fill: u8) -> String {
    protocol::to_base58(protocol::AccountID::from_array([fill; 20]))
}

fn request(subcommand: &'static str) -> JsonValue {
    object([
        ("subcommand", JsonValue::String(subcommand.to_owned())),
        ("source_account", JsonValue::String(account(1))),
        ("destination_account", JsonValue::String(account(2))),
        (
            "destination_amount",
            JsonValue::String("1000000".to_owned()),
        ),
    ])
}

#[test]
fn path_find_requires_enabled_runtime_and_websocket_session() {
    let runtime = FakeRuntime {
        path_search_max: Cell::new(0),
        synced: Cell::new(true),
    };
    let result = do_path_find(
        &request("create"),
        2,
        &runtime,
        Option::<&FakeSession>::None,
        &PathRequestManager::new(),
        &FakePathSource::default(),
        100,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NotSupported.token().to_owned()
        ))
    );

    let runtime = FakeRuntime {
        path_search_max: Cell::new(8),
        synced: Cell::new(true),
    };
    let result = do_path_find(
        &request("create"),
        2,
        &runtime,
        Option::<&FakeSession>::None,
        &PathRequestManager::new(),
        &FakePathSource::default(),
        100,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoEvents.token().to_owned()
        ))
    );

    let runtime = FakeRuntime {
        path_search_max: Cell::new(8),
        synced: Cell::new(false),
    };
    let result = do_path_find(
        &request("create"),
        2,
        &runtime,
        Option::<&FakeSession>::None,
        &PathRequestManager::new(),
        &FakePathSource::default(),
        100,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoNetwork.token().to_owned()
        ))
    );
}

#[test]
fn path_find_create_status_and_close_follow_request_lifecycle() {
    let runtime = FakeRuntime {
        path_search_max: Cell::new(8),
        synced: Cell::new(true),
    };
    let session = FakeSession {
        id: 7,
        ..FakeSession::default()
    };
    let manager = PathRequestManager::new();
    let source = FakePathSource::default();

    let create = do_path_find(
        &request("create"),
        2,
        &runtime,
        Some(&session),
        &manager,
        &source,
        100,
    );
    let JsonValue::Object(create) = create else {
        panic!("create must be an object");
    };
    assert_eq!(session.api_version.get(), 2);
    assert!(create.contains_key("alternatives"));
    assert_eq!(
        create.get("source_account"),
        Some(&JsonValue::String(account(1)))
    );
    assert_eq!(
        create.get("destination_account"),
        Some(&JsonValue::String(account(2)))
    );
    assert_eq!(
        create.get("destination_amount"),
        Some(&JsonValue::String("1000000".to_owned()))
    );
    assert!(session.request_id.get().is_some());
    assert_eq!(manager.request_count(), 1);

    let status = do_path_find(
        &request("status"),
        2,
        &runtime,
        Some(&session),
        &manager,
        &source,
        100,
    );
    let JsonValue::Object(status) = status else {
        panic!("status must be an object");
    };
    assert!(status.contains_key("alternatives"));

    let close = do_path_find(
        &request("close"),
        2,
        &runtime,
        Some(&session),
        &manager,
        &source,
        100,
    );
    let JsonValue::Object(close) = close else {
        panic!("close must be an object");
    };
    assert_eq!(close.get("closed"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        close.get("source_account"),
        Some(&JsonValue::String(account(1)))
    );
    assert_eq!(
        close.get("destination_account"),
        Some(&JsonValue::String(account(2)))
    );
    assert!(close.contains_key("alternatives"));
    assert_eq!(session.request_id.get(), None);
    assert_eq!(manager.request_count(), 0);
    assert_eq!(source.levels.borrow().first().copied(), Some(2));
}

#[test]
fn ripple_path_find_prefers_direct_legacy_requests_when_explicit_or_unsynced() {
    let runtime = FakeRuntime {
        path_search_max: Cell::new(8),
        synced: Cell::new(false),
    };
    let session = FakeSession::default();
    let manager = PathRequestManager::new();
    let source = FakePathSource::default();

    let result = do_ripple_path_find(
        &request("create"),
        &runtime,
        Some(&session),
        &manager,
        &source,
        90,
        true,
    );
    let JsonValue::Object(result) = result else {
        panic!("legacy result must be an object");
    };
    assert!(result.contains_key("alternatives"));
    assert!(result.contains_key("destination_currencies"));
    assert_eq!(source.legacy_calls.get(), 1);
    assert_eq!(session.request_id.get(), None);
}

#[test]
fn path_find_rejects_malformed_destination_amount() {
    let runtime = FakeRuntime {
        path_search_max: Cell::new(8),
        synced: Cell::new(true),
    };
    let session = FakeSession {
        id: 11,
        ..FakeSession::default()
    };
    let manager = PathRequestManager::new();
    let source = FakePathSource::default();
    let invalid_request = object([
        ("subcommand", JsonValue::String("create".to_owned())),
        ("source_account", JsonValue::String(account(1))),
        ("destination_account", JsonValue::String(account(2))),
        ("destination_amount", JsonValue::Array(Vec::new())),
    ]);

    let result = do_path_find(
        &invalid_request,
        2,
        &runtime,
        Some(&session),
        &manager,
        &source,
        200,
    );
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    assert_eq!(
        result.get("error"),
        Some(&JsonValue::String("dstAmtMalformed".to_owned()))
    );
}

#[test]
fn path_find_status_and_close_without_open_request_report_no_path_request() {
    let runtime = FakeRuntime {
        path_search_max: Cell::new(8),
        synced: Cell::new(true),
    };
    let session = FakeSession {
        id: 13,
        ..FakeSession::default()
    };
    let manager = PathRequestManager::new();
    let source = FakePathSource::default();

    let status = do_path_find(
        &request("status"),
        2,
        &runtime,
        Some(&session),
        &manager,
        &source,
        100,
    );
    let JsonValue::Object(status) = status else {
        panic!("status response must be an object");
    };
    assert_eq!(
        status.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoPathRequest.token().to_owned()
        ))
    );

    let close = do_path_find(
        &request("close"),
        2,
        &runtime,
        Some(&session),
        &manager,
        &source,
        100,
    );
    let JsonValue::Object(close) = close else {
        panic!("close response must be an object");
    };
    assert_eq!(
        close.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoPathRequest.token().to_owned()
        ))
    );
}

#[test]
fn path_find_close_requires_network_sync_before_session_events() {
    let runtime = FakeRuntime {
        path_search_max: Cell::new(8),
        synced: Cell::new(false),
    };
    let session = FakeSession {
        id: 17,
        ..FakeSession::default()
    };

    let close = do_path_find(
        &request("close"),
        2,
        &runtime,
        Some(&session),
        &PathRequestManager::new(),
        &FakePathSource::default(),
        100,
    );
    let JsonValue::Object(close) = close else {
        panic!("close response must be an object");
    };
    assert_eq!(
        close.get("error"),
        Some(&JsonValue::String(
            RpcErrorCode::NoNetwork.token().to_owned()
        ))
    );
}
